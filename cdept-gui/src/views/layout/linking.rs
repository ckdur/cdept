//! The layout tab's side of linking with the schematic (docs/LINKING.md §6): Generate /
//! Update from Schematic, the Link panel, and cross-probing with schematic tabs.
//!
//! The schematic is read from disk (as saved) in Rust (`libcdept::link`); the helper
//! makes the top cell match (`link_apply`, one undoable step) and tells which instances
//! are bound (`link_instances`). The link (schematic, what each instance became) goes to
//! the sidecar with the next save.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Color32, Ui};
use libcdept::layout::{Meta, Source};
use libcdept::link::{self, Applied, ApplyOptions, DeviceMap, Schematic, Target};
use serde_json::{Value, json};

use super::LayoutView;
use crate::state::{Action, AppCtx};

/// One bound instance as the helper reports it.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct Bound {
    #[serde(default)]
    pub cell: String,
    #[serde(default)]
    pub library: String,
    #[serde(default)]
    pub pcell: bool,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Instances {
    #[serde(default)]
    instances: BTreeMap<String, Bound>,
    #[serde(default)]
    unbound: usize,
}

/// The state of the link in a layout tab.
#[derive(Default)]
pub(super) struct LinkState {
    /// The Link panel is shown.
    pub open: bool,
    /// Selections follow between this layout and its schematic's tabs.
    pub cross_probe: bool,
    /// The schematic the layout is linked with (sidecar), else the cell's own.
    pub source: Option<(Source, PathBuf)>,
    schematic: Option<Schematic>,
    map: Option<DeviceMap>,
    error: Option<String>,
    instances: Instances,
    instances_asked: Option<u64>,
    applying: Option<u64>,
    /// What the last Generate did.
    pub last: Option<Applied>,
    /// The link to write to the sidecar after the next save.
    pending_record: Option<(Schematic, Vec<String>)>,
    /// The selection last seen (count, box), to notice changes.
    seen_selection: Option<(usize, Option<[i64; 4]>)>,
    selected_asked: Option<u64>,
    /// Names selected here, for the schematic tabs (taken by `coordinate_links`).
    pub probe_out: Option<Vec<String>>,
    /// The names last exchanged with the schematic, to not send them back.
    pub probed: Vec<String>,
}

impl LinkState {
    pub fn new() -> LinkState {
        LinkState {
            cross_probe: true,
            ..LinkState::default()
        }
    }

    /// A new helper: questions to the old one get no answer.
    pub fn forget_questions(&mut self) {
        self.applying = None;
        self.instances_asked = None;
        self.selected_asked = None;
        self.seen_selection = None;
    }
}

/// Status of one schematic instance in the layout.
fn status(wanted: &link::Wanted, bound: Option<&Bound>) -> (&'static str, Color32) {
    let Some(b) = bound else {
        return ("missing", Color32::from_rgb(220, 120, 0));
    };
    if b.count > 1 {
        return ("twice", Color32::from_rgb(220, 60, 60));
    }
    match &wanted.target {
        Target::Pcell {
            library,
            cell,
            params,
        } => {
            if !b.pcell || &b.cell != cell || &b.library != library {
                return ("other cell", Color32::from_rgb(220, 60, 60));
            }
            let differs = params.iter().any(|(k, v)| {
                let have = match b.params.get(k) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                have != *v && !numbers_equal(&have, v)
            });
            if differs {
                ("parameters", Color32::from_rgb(220, 120, 0))
            } else {
                ("ok", Color32::from_rgb(60, 170, 60))
            }
        }
        Target::Cell {
            library,
            cell,
            view,
            ..
        } => {
            if b.label.as_deref() == Some(&format!("{library}/{cell}/{view}")) {
                ("ok", Color32::from_rgb(60, 170, 60))
            } else {
                ("other cell", Color32::from_rgb(220, 60, 60))
            }
        }
    }
}

fn numbers_equal(a: &str, b: &str) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() <= 1e-12 * x.abs().max(y.abs()).max(1.0),
        _ => false,
    }
}

impl LayoutView {
    /// The schematic this layout belongs to: the sidecar's link, else the cell's own
    /// schematic view.
    pub(super) fn find_link_source(&mut self) {
        let meta_source = Meta::read_for(&self.file)
            .ok()
            .flatten()
            .and_then(|m| m.source);
        let (library, cell) = (&self.doc.library, &self.doc.cell);
        let view = meta_source.as_ref().map(|s| s.view.clone());
        let lib = meta_source
            .as_ref()
            .map_or(library.as_str(), |s| s.library.as_str());
        let c = meta_source
            .as_ref()
            .map_or(cell.as_str(), |s| s.cell.as_str());
        self.link.source = link::schematic_view(&self.project, lib, c, view.as_deref()).ok();
    }

    /// The schematic file this layout is linked with, if any.
    pub fn link_schematic(&self) -> Option<&std::path::Path> {
        self.link.source.as_ref().map(|(_, f)| f.as_path())
    }

    /// Read the schematic again (as saved) and ask the helper what is bound.
    pub(super) fn refresh_link(&mut self, project: Option<&libcdept::Project>) {
        let project = project.unwrap_or(&self.project).clone();
        self.link.error = None;
        let Some((source, file)) = self.link.source.clone() else {
            self.link.error = Some("this cell has no schematic to link with".into());
            return;
        };
        match DeviceMap::for_project(&project)
            .and_then(|map| link::read_schematic(&project, &map, source, &file).map(|s| (map, s)))
        {
            Ok((map, schematic)) => {
                self.link.map = Some(map);
                self.link.schematic = Some(schematic);
            }
            Err(e) => self.link.error = Some(format!("{e:#}")),
        }
        if self.link.instances_asked.is_none() {
            self.link.instances_asked = Some(self.raw_call("link_instances", json!({})));
        }
    }

    /// Generate / Update from Schematic: make the layout match the schematic as saved.
    pub fn generate_from_schematic(&mut self, project: Option<&libcdept::Project>, remove: bool) {
        if self.link.applying.is_some() {
            return;
        }
        self.refresh_link(project);
        let (Some(map), Some(schematic)) = (&self.link.map, &self.link.schematic) else {
            let text = self
                .link
                .error
                .clone()
                .unwrap_or_else(|| "nothing to generate from".into());
            self.notify(libcdept::klayout::protocol::NoticeLevel::Error, text);
            return;
        };
        let project = project.unwrap_or(&self.project);
        let request = link::apply_request(project, map, schematic, &ApplyOptions { remove });
        // Cells placed from other views join what the helper knows.
        for w in &schematic.wanted {
            if let Target::Cell { file, .. } = &w.target {
                let table = libcdept::layout::refs::resolve(project, file);
                self.refs.files.extend(table.files);
            }
        }
        self.session.set_refs(self.refs.clone());
        self.link.applying = Some(self.raw_call("link_apply", request));
        self.link.open = true;
    }

    /// Take in the answers about the link and notice selection changes; called every
    /// frame.
    pub(super) fn poll_link(&mut self) {
        use libcdept::klayout::protocol::NoticeLevel;
        if let Some(id) = self.link.applying
            && let Some(answer) = self.take_reply(id)
        {
            self.link.applying = None;
            match answer
                .and_then(|text| serde_json::from_str::<Applied>(&text).map_err(|e| e.to_string()))
            {
                Ok(applied) => {
                    let text = format!(
                        "From the schematic: {} added, {} taken over, {} updated, {} replaced, \
                         {} removed",
                        applied.added.len(),
                        applied.bound.len(),
                        applied.updated.len(),
                        applied.replaced.len(),
                        applied.removed.len()
                    );
                    self.notify(NoticeLevel::Info, text);
                    for f in &applied.failed {
                        self.notify(NoticeLevel::Error, format!("{}: {}", f.name, f.error));
                    }
                    if let Some(schematic) = &self.link.schematic {
                        self.link.pending_record = Some((schematic.clone(), applied.present()));
                    }
                    self.link.last = Some(applied);
                    self.link.instances_asked = Some(self.raw_call("link_instances", json!({})));
                }
                Err(e) => self.notify(NoticeLevel::Error, format!("Generate: {e}")),
            }
        }
        if let Some(id) = self.link.instances_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.link.instances_asked = None;
            if let Ok(found) = answer
                .and_then(|t| serde_json::from_str::<Instances>(&t).map_err(|e| e.to_string()))
            {
                self.link.instances = found;
            }
        }
        // Cross-probing, layout → schematic: a changed selection is asked about.
        let seen = (
            self.selection.count,
            self.selection
                .bbox
                .map(|b| b.map(|v| (v * 1000.0).round() as i64)),
        );
        if self.link.seen_selection != Some(seen) {
            self.link.seen_selection = Some(seen);
            if self.link.cross_probe && self.link.selected_asked.is_none() && self.ready() {
                self.link.selected_asked = Some(self.raw_call("link_selected", json!({})));
            }
        }
        if let Some(id) = self.link.selected_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.link.selected_asked = None;
            let names: Vec<String> = answer
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| serde_json::from_value(v["names"].clone()).ok())
                .unwrap_or_default();
            if names != self.link.probed {
                self.link.probed = names.clone();
                self.link.probe_out = Some(names);
            }
        }
    }

    /// After a save: the link of the last Generate goes to the sidecar.
    pub(super) fn record_link(&mut self) -> Result<(), String> {
        let Some((schematic, present)) = self.link.pending_record.take() else {
            return Ok(());
        };
        link::record(&self.file, &schematic, &present)
            .map_err(|e| format!("saved, but the link was not recorded: {e:#}"))
    }

    /// Select (and show) the instances bound to `names`, as asked by a schematic tab.
    pub fn probe_in(&mut self, names: &[String]) {
        if !self.link.cross_probe || !self.ready() || names == self.link.probed {
            return;
        }
        self.link.probed = names.to_vec();
        self.quiet("link_select", json!({"names": names, "zoom": false}));
    }

    /// The Layout menu entries.
    pub(super) fn link_menu(&mut self, ui: &mut Ui, ctx: &mut AppCtx, editable: bool) {
        let has_source = self.link.source.is_some();
        if ui
            .add_enabled(
                editable && has_source && self.link.applying.is_none(),
                egui::Button::new("Generate / Update from Schematic"),
            )
            .on_hover_text(
                "Place the schematic's devices (PDK PCells) and cells, update changed \
                 parameters; uses the schematic as saved",
            )
            .clicked()
        {
            self.generate_from_schematic(ctx.project.as_ref(), false);
            ui.close();
        }
        if ui
            .add_enabled(has_source, egui::Button::new("Open Schematic"))
            .clicked()
        {
            if let Some(file) = self.link_schematic() {
                ctx.request(Action::OpenFile(file.to_path_buf()));
            }
            ui.close();
        }
        if ui.checkbox(&mut self.link.open, "Link Panel").changed() && self.link.open {
            self.refresh_link(ctx.project.as_ref());
        }
        ui.checkbox(&mut self.link.cross_probe, "Cross-probe with the Schematic");
    }

    /// The Link panel (bottom of the tab).
    pub(super) fn link_panel(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        if !self.link.open {
            return;
        }
        enum Act {
            Generate(bool),
            Refresh,
            Select(String),
            OpenSchematic,
            Close,
        }
        let mut acts = Vec::new();
        let editable = self.ready() && self.editable;
        egui::Panel::bottom(egui::Id::new(("klayout-link", self.id)))
            .resizable(true)
            .default_size(180.0)
            .size_range(80.0..=600.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Link");
                    match &self.link.source {
                        Some((source, _)) => {
                            if ui
                                .link(source.label())
                                .on_hover_text("Open the schematic")
                                .clicked()
                            {
                                acts.push(Act::OpenSchematic);
                            }
                        }
                        None => {
                            ui.weak("no schematic");
                        }
                    }
                    ui.separator();
                    if self.link.applying.is_some() {
                        ui.spinner();
                    }
                    if ui
                        .add_enabled(
                            editable && self.link.source.is_some(),
                            egui::Button::new("Generate / Update"),
                        )
                        .clicked()
                    {
                        acts.push(Act::Generate(false));
                    }
                    if ui
                        .add_enabled(editable, egui::Button::new("Remove Extra"))
                        .on_hover_text(
                            "Generate, and delete instances whose schematic instance is gone",
                        )
                        .clicked()
                    {
                        acts.push(Act::Generate(true));
                    }
                    if ui.button("Refresh").clicked() {
                        acts.push(Act::Refresh);
                    }
                    ui.checkbox(&mut self.link.cross_probe, "Cross-probe");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            acts.push(Act::Close);
                        }
                    });
                });
                if let Some(error) = &self.link.error {
                    ui.colored_label(Color32::from_rgb(220, 60, 60), error);
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        let Some(schematic) = &self.link.schematic else {
                            ui.weak("Press Refresh to compare with the schematic.");
                            return;
                        };
                        egui::Grid::new(("klayout-link-grid", self.id))
                            .striped(true)
                            .num_columns(3)
                            .show(ui, |ui| {
                                for w in &schematic.wanted {
                                    let bound = self.link.instances.instances.get(&w.name);
                                    let (text, color) = status(w, bound);
                                    if ui
                                        .selectable_label(false, &w.name)
                                        .on_hover_text("Select it in the layout")
                                        .clicked()
                                    {
                                        acts.push(Act::Select(w.name.clone()));
                                    }
                                    ui.label(w.target.label());
                                    ui.colored_label(color, text);
                                    ui.end_row();
                                }
                                let wanted: Vec<&str> =
                                    schematic.wanted.iter().map(|w| w.name.as_str()).collect();
                                for (name, b) in &self.link.instances.instances {
                                    if wanted.contains(&name.as_str()) {
                                        continue;
                                    }
                                    if ui.selectable_label(false, name).clicked() {
                                        acts.push(Act::Select(name.clone()));
                                    }
                                    ui.label(b.label.clone().unwrap_or_else(|| b.cell.clone()));
                                    ui.colored_label(
                                        Color32::from_rgb(220, 60, 60),
                                        "not in schematic",
                                    );
                                    ui.end_row();
                                }
                                for p in &schematic.problems {
                                    ui.label(&p.name);
                                    ui.label(&p.symbol);
                                    ui.colored_label(Color32::from_rgb(220, 60, 60), &p.reason);
                                    ui.end_row();
                                }
                            });
                        if self.link.instances.unbound > 0 {
                            ui.weak(format!(
                                "{} instance(s) of the layout are not linked to the schematic",
                                self.link.instances.unbound
                            ));
                        }
                    });
            });
        for act in acts {
            match act {
                Act::Generate(remove) => self.generate_from_schematic(ctx.project.as_ref(), remove),
                Act::Refresh => self.refresh_link(ctx.project.as_ref()),
                Act::Select(name) => self.link_select(&[name]),
                Act::OpenSchematic => {
                    if let Some(file) = self.link_schematic() {
                        ctx.request(Action::OpenFile(file.to_path_buf()));
                    }
                }
                Act::Close => self.link.open = false,
            }
        }
    }

    /// Select instances by schematic name and show them, as a click in the Link panel
    /// does; the schematic tabs follow.
    pub fn link_select(&mut self, names: &[String]) {
        self.link.probed = names.to_vec();
        self.quiet("link_select", json!({"names": names, "zoom": true}));
        self.link.probe_out = Some(names.to_vec());
    }

    /// For the test driver: the status of a schematic instance in the Link panel.
    pub fn link_status(&self, name: &str) -> Option<&'static str> {
        let schematic = self.link.schematic.as_ref()?;
        let w = schematic.wanted.iter().find(|w| w.name == name)?;
        Some(status(w, self.link.instances.instances.get(name)).0)
    }

    /// For the test driver: a Generate or an `link_instances` question is under way.
    pub fn link_busy(&self) -> bool {
        self.link.applying.is_some() || self.link.instances_asked.is_some()
    }

    /// The helper runs and can be asked to generate.
    pub fn link_ready(&self) -> bool {
        self.ready()
    }

    /// Names selected here since the last call, for the schematic tabs.
    pub fn take_probe_out(&mut self) -> Option<Vec<String>> {
        self.link.probe_out.take()
    }

    /// Names bound to the instances selected in the layout, as last asked.
    pub fn link_probed(&self) -> &[String] {
        &self.link.probed
    }
}
