//! The schematic tab's side of linking with the layout (docs/LINKING.md §6): Open
//! Layout, Generate Layout from Schematic, and cross-probing (the selection of instances
//! follows between this tab and the layout tabs linked with it).

use egui::Ui;
use libcdept::layout::Source;
use libcdept::xschem::tcl;

use super::SchematicView;
use crate::state::{Action, AppCtx};

/// What the schematic tab asks of the layout side (taken by
/// `Views::coordinate_links`).
#[derive(Debug, Clone, PartialEq)]
pub enum LinkRequest {
    /// Generate or update this layout file from the schematic, opening it first.
    Generate(std::path::PathBuf),
}

/// The link state of a schematic tab.
#[derive(Debug, Clone, Default)]
pub struct SchematicLink {
    /// Instance names last exchanged with the layout side.
    pub probed: Vec<String>,
    /// Names selected here, for the layout tabs.
    pub probe_out: Option<Vec<String>>,
    pub request: Option<LinkRequest>,
    /// Nets last exchanged with the layout side (selected here, or highlighted for it).
    pub nets_probed: Vec<String>,
    /// Nets selected here, for the layout tabs.
    pub nets_out: Option<Vec<String>>,
    /// What selecting a layout tab's names here made the selection (it is not sent
    /// back: it may be shallower than what was asked for).
    pub echo: Option<Vec<String>>,
    /// xschem's window title last seen (another schematic shown: read the selection,
    /// and so where in the hierarchy, again).
    pub title_seen: Option<String>,
}

impl SchematicView {
    fn link_source(&self) -> Option<Source> {
        if self.doc.file.is_some() {
            return None;
        }
        Some(Source {
            library: self.doc.library.clone(),
            cell: self.doc.cell.clone(),
            view: self.doc.view.clone(),
        })
    }

    /// Where xschem is in the hierarchy, as an instance path prefix: `` at the top,
    /// `x1/` inside instance x1 (docs/LINKING.md §9).
    pub fn hierarchy_prefix(&self) -> String {
        self.selection
            .as_ref()
            .map(|s| s.prefix())
            .unwrap_or_default()
    }

    /// The schematic xschem shows (another one than the tab's after a descent).
    pub fn shown_schematic(&self) -> Option<&std::path::Path> {
        self.selection.as_ref().and_then(|s| s.schematic.as_deref())
    }

    /// A new selection was read from xschem: tell the layout side if it changed. Names
    /// go out as instance paths from the tab's own schematic (`x1/M1` when xschem is
    /// inside x1).
    pub(super) fn selection_changed(&mut self) {
        let prefix = self.hierarchy_prefix();
        let names: Vec<String> = self
            .selection
            .as_ref()
            .map(|s| {
                s.instances
                    .iter()
                    .map(|i| format!("{prefix}{}", i.name))
                    .collect()
            })
            .unwrap_or_default();
        if self.link.echo.as_ref() == Some(&names) {
            self.link.echo = None;
        } else if names != self.link.probed {
            self.link.echo = None;
            self.link.probed = names.clone();
            self.link.probe_out = Some(names);
        }
        let nets: Vec<String> = self
            .selection
            .as_ref()
            .map(|s| s.nets.iter().map(|n| format!("{prefix}{n}")).collect())
            .unwrap_or_default();
        if !nets.is_empty() && nets != self.link.nets_probed {
            self.link.nets_probed = nets.clone();
            self.link.nets_out = Some(nets);
        }
    }

    /// Nets selected here since the last call, for the layout tabs.
    pub fn take_net_probe_out(&mut self) -> Option<Vec<String>> {
        self.link.nets_out.take()
    }

    /// Highlight the nets `names` with xschem's net highlight, as asked by a layout tab
    /// (an empty list clears). Names are net paths from the tab's own schematic (every
    /// name a net has: `A`, `x1/IN`); those of the schematic xschem shows now are
    /// highlighted.
    pub fn probe_nets_in(&mut self, names: &[String]) {
        if names == self.link.nets_probed {
            return;
        }
        self.link.nets_probed = names.to_vec();
        let prefix = self.hierarchy_prefix();
        let mut script = String::from("xschem unhilight_all\n");
        for name in names
            .iter()
            .filter_map(|n| n.strip_prefix(prefix.as_str()))
            .filter(|n| !n.contains('/'))
        {
            script.push_str(&format!("xschem hilight_netname {}\n", tcl::quote(name)));
        }
        self.session.tcl(&script);
    }

    /// The nets last exchanged with the layout side.
    pub fn nets_probed(&self) -> &[String] {
        &self.link.nets_probed
    }

    /// Names selected here since the last call, for the layout tabs.
    pub fn take_probe_out(&mut self) -> Option<Vec<String>> {
        self.link.probe_out.take()
    }

    pub fn take_link_request(&mut self) -> Option<LinkRequest> {
        self.link.request.take()
    }

    /// Select the instances `names` in xschem, as asked by a layout tab. Names are
    /// instance paths from the tab's own schematic; the instance of the schematic shown
    /// now on each path is selected (`x1/M1` selects x1 at the top, M1 inside x1).
    pub fn probe_in(&mut self, names: &[String]) {
        if names == self.link.probed {
            return;
        }
        self.link.probed = names.to_vec();
        let prefix = self.hierarchy_prefix();
        let mut here: Vec<&str> = names
            .iter()
            .filter_map(|n| n.strip_prefix(prefix.as_str()))
            .filter_map(|n| n.split('/').next())
            .filter(|n| !n.is_empty())
            .collect();
        here.sort_unstable();
        here.dedup();
        self.link.echo = Some(here.iter().map(|n| format!("{prefix}{n}")).collect());
        // The other layouts linked with this schematic follow too (the one that asked
        // has these names already and ignores them).
        self.link.probe_out = Some(names.to_vec());
        let mut script = String::from("xschem unselect_all 1\n");
        for name in here {
            script.push_str(&format!("xschem select instance {}\n", tcl::quote(name)));
        }
        self.session.tcl(&script);
        self.selection_stale = true;
    }

    /// The entries of the Cell menu.
    pub(super) fn link_menu(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let Some(source) = self.link_source() else {
            return;
        };
        let layouts = ctx
            .project
            .as_ref()
            .map(|p| libcdept::link::layouts_of(p, &source))
            .unwrap_or_default();
        ui.separator();
        if ui
            .add_enabled(!layouts.is_empty(), egui::Button::new("Open Layout"))
            .on_hover_text("Open the layout linked with this schematic")
            .clicked()
        {
            ctx.request(Action::OpenFile(layouts[0].clone()));
            ui.close();
        }
        if ui
            .button("Generate / Update Layout")
            .on_hover_text(
                "Place the schematic's devices (PDK PCells) and cells in the layout, update \
                 changed parameters (the schematic as saved; the layout view is made if \
                 there is none)",
            )
            .clicked()
        {
            ui.close();
            let file = match layouts.first() {
                Some(file) => Ok(file.clone()),
                None => match ctx.project.as_mut() {
                    Some(project) => libcdept::link::layout_view(
                        project,
                        &source.library,
                        &source.cell,
                        None,
                        true,
                    )
                    .map_err(|e| format!("{e:#}")),
                    None => Err("no project".to_string()),
                },
            };
            match file {
                Ok(file) => {
                    ctx.request(Action::OpenFile(file.clone()));
                    self.link.request = Some(LinkRequest::Generate(file));
                }
                Err(e) => tracing::error!("Generate Layout: {e}"),
            }
        }
    }
}
