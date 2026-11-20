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

    /// A new selection was read from xschem: tell the layout side if it changed.
    pub(super) fn selection_changed(&mut self) {
        let names: Vec<String> = self
            .selection
            .as_ref()
            .map(|s| s.instances.iter().map(|i| i.name.clone()).collect())
            .unwrap_or_default();
        if names != self.link.probed {
            self.link.probed = names.clone();
            self.link.probe_out = Some(names);
        }
    }

    /// Names selected here since the last call, for the layout tabs.
    pub fn take_probe_out(&mut self) -> Option<Vec<String>> {
        self.link.probe_out.take()
    }

    pub fn take_link_request(&mut self) -> Option<LinkRequest> {
        self.link.request.take()
    }

    /// Select the instances `names` in xschem, as asked by a layout tab.
    pub fn probe_in(&mut self, names: &[String]) {
        if names == self.link.probed {
            return;
        }
        self.link.probed = names.to_vec();
        let mut script = String::from("xschem unselect_all 1\n");
        for name in names {
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
