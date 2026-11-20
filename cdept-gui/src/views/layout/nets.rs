//! Nets of the layout against the schematic's (docs/LINKING.md §8): Update
//! Connectivity, the Nets panel, flylines, net probing and cross-probing of nets.
//!
//! Update Connectivity: the helper of the tab writes the layout as it is now (unsaved
//! changes too), then a thread extracts it with the PDK's LVS deck, reads the
//! schematic's nets with xschem and compares (`libcdept::link::nets`). The helper then
//! loads the LVS database to draw nets and to say which net is under a point; the
//! flylines join the pieces of open nets. What was extracted is as old as the layout
//! was then: the panel says when the layout changed since.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use egui::{CollapsingHeader, Color32, RichText, ScrollArea, Ui};
use libcdept::klayout::protocol::{NoticeLevel, method};
use libcdept::klayout::verify;
use libcdept::layout::Source;
use libcdept::link::nets::{self, NetRun, NetStatus};
use serde_json::{Value, json};

use super::LayoutView;

const OK: Color32 = Color32::from_rgb(60, 170, 60);
const WARN: Color32 = Color32::from_rgb(220, 120, 0);
const BAD: Color32 = Color32::from_rgb(220, 60, 60);

enum Job {
    /// The helper writes the layout; the id of the call and the revision written.
    Export {
        id: u64,
        dir: PathBuf,
        out: PathBuf,
        source: Source,
        schematic: PathBuf,
        at: (u64, u64),
    },
    /// Extraction and comparison on a thread.
    Run {
        result: Receiver<Result<NetRun, String>>,
        started: Instant,
        at: (u64, u64),
    },
}

/// The nets of one layout tab.
pub(super) struct NetsState {
    /// The Nets panel is shown.
    pub open: bool,
    /// Flylines are drawn.
    pub flylines: bool,
    /// A left click in the picture probes the net under it (instead of editing).
    pub probing: bool,
    /// Extract again after every save.
    pub after_save: bool,
    job: Option<Job>,
    last: Option<Result<NetRun, String>>,
    /// (helper epoch, revision) of the layout the last result was made from.
    made_at: Option<(u64, u64)>,
    /// The helper epoch the database was loaded into.
    loaded: Option<u64>,
    /// The schematic net shown (highlighted), or a layout net probed.
    pub shown: Option<String>,
    probe_asked: Option<u64>,
    flylines_asked: Option<u64>,
    /// Flylines the helper drew, as it last said.
    pub drawn: usize,
    /// Schematic net names for the schematic tabs (taken by `coordinate_links`).
    pub probe_out: Option<Vec<String>>,
    /// The schematic nets last exchanged with the schematic tabs.
    pub probed: Vec<String>,
}

impl Default for NetsState {
    fn default() -> Self {
        NetsState {
            open: false,
            flylines: true,
            probing: false,
            after_save: false,
            job: None,
            last: None,
            made_at: None,
            loaded: None,
            shown: None,
            probe_asked: None,
            flylines_asked: None,
            drawn: 0,
            probe_out: None,
            probed: Vec::new(),
        }
    }
}

impl NetsState {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    fn run(&self) -> Option<&NetRun> {
        self.last.as_ref().and_then(|r| r.as_ref().ok())
    }

    /// A new helper: questions to the old one get no answer, and it has no database.
    pub fn forget_questions(&mut self) {
        self.probe_asked = None;
        self.flylines_asked = None;
        self.loaded = None;
        if matches!(self.job, Some(Job::Export { .. })) {
            self.job = None;
        }
    }
}

/// What a click in the panel asks for.
enum Act {
    Show(String),
    Clear,
    Update,
    Flylines(bool),
    Close,
}

fn status_color(status: NetStatus) -> Color32 {
    match status {
        NetStatus::Ok => OK,
        NetStatus::Unplaced => WARN,
        _ => BAD,
    }
}

impl LayoutView {
    /// Update Connectivity: extract the layout as it is now and compare its nets with
    /// the schematic's.
    pub fn update_connectivity(&mut self) {
        if self.nets.running() {
            return;
        }
        self.nets.open = true;
        match self.prepare_nets() {
            Ok(job) => self.nets.job = Some(job),
            Err(error) => self.nets.last = Some(Err(error)),
        }
    }

    fn prepare_nets(&mut self) -> Result<Job, String> {
        let Some((source, schematic)) = self.link.source.clone() else {
            return Err("this layout has no schematic to compare with".into());
        };
        if verify::find_deck(&self.project, verify::Check::Lvs).is_none() {
            return Err(
                "No LVS deck to extract the layout's nets with: set [klayout] lvs in \
                 cdept.toml (none was found in the PDK's libs.tech/klayout)."
                    .into(),
            );
        }
        let (library, cell) = if self.doc.cell.is_empty() {
            ("files".to_string(), verify::fallback_top(&self.file))
        } else {
            (self.doc.library.clone(), self.doc.cell.clone())
        };
        let dir = verify::work_dir_named(&self.project, &library, &cell, "nets")
            .map_err(|e| format!("{e:#}"))?;
        let out = dir.join(format!("{cell}.gds"));
        let id = self.raw_call(method::EXPORT, json!({ "out": out }));
        Ok(Job::Export {
            id,
            dir,
            out,
            source,
            schematic,
            at: (self.epoch, self.revision),
        })
    }

    /// The layout changed since the connectivity was extracted.
    pub fn nets_stale(&self) -> bool {
        self.nets
            .made_at
            .is_some_and(|at| at != (self.epoch, self.revision))
    }

    /// Move a running extraction on, (re)load the database into the helper; every frame.
    pub(super) fn poll_nets(&mut self, ctx: &egui::Context) {
        match self.nets.job.take() {
            Some(Job::Export {
                id,
                dir,
                out,
                source,
                schematic,
                at,
            }) => match self.take_reply(id) {
                None => {
                    self.nets.job = Some(Job::Export {
                        id,
                        dir,
                        out,
                        source,
                        schematic,
                        at,
                    })
                }
                Some(Err(error)) => {
                    self.nets.last = Some(Err(format!(
                        "The layout could not be written for the extraction: {error}"
                    )));
                }
                Some(Ok(answer)) => {
                    let value: Value = serde_json::from_str(&answer).unwrap_or(Value::Null);
                    let top = self
                        .cells
                        .as_ref()
                        .map(|c| c.top.clone())
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| verify::fallback_top(&self.file));
                    let stream = verify::stream_of(&out, &value, &top);
                    let project = self.project.clone();
                    let (sender, result) = mpsc::channel();
                    let wake = ctx.clone();
                    std::thread::Builder::new()
                        .name("cdept-nets".into())
                        .spawn(move || {
                            let outcome = nets::run(&project, source, &schematic, &stream, &dir);
                            let _ = sender.send(outcome.map_err(|e| format!("{e:#}")));
                            wake.request_repaint();
                        })
                        .expect("cannot start a thread");
                    self.nets.job = Some(Job::Run {
                        result,
                        started: Instant::now(),
                        at,
                    });
                }
            },
            Some(Job::Run {
                result,
                started,
                at,
            }) => match result.try_recv() {
                Ok(outcome) => {
                    match &outcome {
                        Ok(run) => {
                            let (ok, open, short) = run.report.counts();
                            let level = if run.report.clean() {
                                NoticeLevel::Info
                            } else {
                                NoticeLevel::Warning
                            };
                            self.notify(
                                level,
                                format!("Nets: {ok} ok, {open} open, {short} shorted"),
                            );
                        }
                        Err(error) => {
                            self.notify(NoticeLevel::Error, format!("Connectivity: {error}"))
                        }
                    }
                    self.nets.last = Some(outcome);
                    self.nets.made_at = Some(at);
                    self.nets.loaded = None;
                }
                Err(TryRecvError::Empty) => {
                    self.nets.job = Some(Job::Run {
                        result,
                        started,
                        at,
                    })
                }
                Err(TryRecvError::Disconnected) => {
                    self.nets.last = Some(Err("the extraction stopped".into()))
                }
            },
            None => {}
        }
        // The helper (perhaps a new one) gets the database of the last result.
        if self.ready()
            && self.nets.loaded != Some(self.epoch)
            && let Some(run) = self.nets.run()
        {
            let lvsdb = run.lvsdb.clone();
            let top = run.connectivity.top.clone();
            self.nets.loaded = Some(self.epoch);
            self.quiet("nets_load", json!({"lvsdb": lvsdb, "top": top}));
            self.send_flylines();
            if let Some(name) = self.nets.shown.clone() {
                self.highlight_net(&name, false);
            }
        }
        if let Some(id) = self.nets.flylines_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.nets.flylines_asked = None;
            self.nets.drawn = answer
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v["markers"].as_u64())
                .unwrap_or(0) as usize;
        }
        if let Some(id) = self.nets.probe_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.nets.probe_asked = None;
            match answer.and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
            {
                Ok(value) => self.probed(&value),
                Err(error) => self.notify(NoticeLevel::Error, format!("Probe: {error}")),
            }
        }
    }

    fn send_flylines(&mut self) {
        let lines: Vec<[f64; 4]> = match (self.nets.flylines, self.nets.run()) {
            (true, Some(run)) => run
                .report
                .flylines()
                .iter()
                .map(|l| [l.from[0], l.from[1], l.to[0], l.to[1]])
                .collect(),
            _ => Vec::new(),
        };
        self.nets.flylines_asked = Some(self.raw_call("nets_flylines", json!({ "lines": lines })));
    }

    /// Show or hide the flylines.
    pub fn set_flylines(&mut self, on: bool) {
        self.nets.flylines = on;
        if self.ready() && self.nets.loaded == Some(self.epoch) {
            self.send_flylines();
        }
    }

    /// Highlight the layout nets of the schematic net `name` (as a click in the panel
    /// does) and tell the schematic tabs. An error when there is no such net.
    pub fn show_net(&mut self, name: &str) -> Result<(), String> {
        let run = self
            .nets
            .run()
            .ok_or("there is no connectivity yet (Update Connectivity)")?;
        if run.report.net(name).is_none() {
            return Err(format!(
                "the schematic has no net {name} with terminals in the layout"
            ));
        }
        self.highlight_net(name, true);
        self.nets.probed = vec![name.to_string()];
        if self.link.cross_probe {
            self.nets.probe_out = Some(vec![name.to_string()]);
        }
        Ok(())
    }

    fn highlight_net(&mut self, name: &str, zoom: bool) {
        let ids = self
            .nets
            .run()
            .and_then(|run| run.report.net(name))
            .map(|n| n.layout_nets())
            .unwrap_or_default();
        self.nets.shown = Some(name.to_string());
        self.quiet("nets_highlight", json!({"nets": ids, "zoom": zoom}));
    }

    fn clear_highlight(&mut self) {
        self.nets.shown = None;
        self.quiet("nets_highlight", json!({"nets": []}));
    }

    /// The schematic tabs selected these nets: highlight them here.
    pub fn probe_nets_in(&mut self, names: &[String]) {
        if !self.link.cross_probe || !self.ready() || names == self.nets.probed {
            return;
        }
        self.nets.probed = names.to_vec();
        let known: Vec<String> = match self.nets.run() {
            Some(run) => names
                .iter()
                .filter(|n| run.report.net(n).is_some())
                .cloned()
                .collect(),
            None => return,
        };
        match known.first() {
            Some(name) => self.highlight_net(&name.clone(), false),
            None => self.clear_highlight(),
        }
    }

    /// Probe the net under frame pixel (x, y), as a click in probe mode does.
    pub fn probe_net_at(&mut self, x: f64, y: f64) {
        if self.nets.loaded != Some(self.epoch) {
            self.notify(
                NoticeLevel::Warning,
                "Probe: no connectivity yet (Layout › Update Connectivity)".into(),
            );
            return;
        }
        self.nets.probe_asked = Some(self.raw_call("nets_probe", json!({"pixel": [x, y]})));
    }

    /// The answer to `nets_probe`: highlight the layout net, tell the schematic tabs.
    fn probed(&mut self, value: &Value) {
        let Some(id) = value["net"].as_u64().map(|n| n as usize) else {
            self.clear_highlight();
            self.nets.probed.clear();
            self.notify(NoticeLevel::Info, "No net there".into());
            return;
        };
        let (label, names) = match self.nets.run() {
            Some(run) => (
                run.connectivity.net_label(id),
                run.report
                    .nets_on(id)
                    .iter()
                    .map(|n| n.name.clone())
                    .collect::<Vec<_>>(),
            ),
            None => (format!("#{id}"), Vec::new()),
        };
        self.quiet("nets_highlight", json!({"nets": [id], "zoom": false}));
        let text = if names.is_empty() {
            format!("Layout net {label}: no schematic net")
        } else {
            format!("Layout net {label}: schematic {}", names.join(", "))
        };
        self.notify(NoticeLevel::Info, text);
        self.nets.shown = Some(names.first().cloned().unwrap_or(label));
        self.nets.probed = names.clone();
        if self.link.cross_probe {
            self.nets.probe_out = Some(names);
        }
    }

    /// For the test driver and the status line: one line about the nets.
    pub fn nets_status(&self) -> String {
        if self.nets.running() {
            return "running".into();
        }
        match &self.nets.last {
            None => "none".into(),
            Some(Err(e)) => format!("failed: {e}"),
            Some(Ok(run)) => {
                let (ok, open, short) = run.report.counts();
                let mut text = format!("{ok} ok, {open} open, {short} shorted");
                if run.report.clean() {
                    text.push_str(", clean");
                }
                if self.nets_stale() {
                    text.push_str(" (out of date)");
                }
                text
            }
        }
    }

    /// Schematic nets selected here since the last call, for the schematic tabs.
    pub fn take_net_probe_out(&mut self) -> Option<Vec<String>> {
        self.nets.probe_out.take()
    }

    /// The schematic nets last probed or shown here.
    pub fn nets_probed(&self) -> &[String] {
        &self.nets.probed
    }

    /// Flylines the helper drew, as it last said.
    pub fn flylines_drawn(&self) -> usize {
        self.nets.drawn
    }

    /// After a save: extract again when asked to.
    pub(super) fn nets_after_save(&mut self) {
        if self.nets.after_save && self.nets.last.is_some() && !self.nets.running() {
            self.update_connectivity();
        }
    }

    /// The Layout menu entries.
    pub(super) fn nets_menu(&mut self, ui: &mut Ui, ready: bool) {
        let can = ready && self.link.source.is_some() && !self.nets.running();
        if ui
            .add_enabled(can, egui::Button::new("Update Connectivity"))
            .on_hover_text(
                "Extract the layout as it is now (PDK's LVS deck) and compare its nets with \
                 the schematic's: opens, shorts, flylines",
            )
            .clicked()
        {
            self.update_connectivity();
            ui.close();
        }
        ui.checkbox(&mut self.nets.open, "Nets Panel");
        let mut flylines = self.nets.flylines;
        if ui.checkbox(&mut flylines, "Flylines").changed() {
            self.set_flylines(flylines);
        }
        ui.checkbox(&mut self.nets.probing, "Probe Nets")
            .on_hover_text("A click in the picture shows the net under it (N)");
        ui.checkbox(
            &mut self.nets.after_save,
            "Update Connectivity after Saving",
        );
    }

    /// The Nets panel at the bottom of the tab, when it is open.
    pub(super) fn nets_panel(&mut self, ui: &mut Ui) {
        if !self.nets.open {
            return;
        }
        let mut acts = Vec::new();
        egui::Panel::bottom(egui::Id::new(("klayout-nets", self.id)))
            .resizable(true)
            .default_size(200.0)
            .size_range(80.0..=800.0)
            .show(ui, |ui| self.nets_contents(ui, &mut acts));
        for act in acts {
            match act {
                Act::Show(name) => {
                    let _ = self.show_net(&name);
                }
                Act::Clear => self.clear_highlight(),
                Act::Update => self.update_connectivity(),
                Act::Flylines(on) => self.set_flylines(on),
                Act::Close => self.nets.open = false,
            }
        }
    }

    fn nets_contents(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.strong("Nets");
            if let Some(job) = &self.nets.job {
                ui.spinner();
                let since = match job {
                    Job::Run { started, .. } => format!(" ({} s)", started.elapsed().as_secs()),
                    Job::Export { .. } => String::new(),
                };
                ui.label(format!("Extracting…{since}"));
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(500));
            } else if ui
                .add_enabled(self.link.source.is_some(), egui::Button::new("Update"))
                .on_hover_text("Extract the layout as it is now and compare")
                .clicked()
            {
                acts.push(Act::Update);
            }
            let mut flylines = self.nets.flylines;
            if ui.checkbox(&mut flylines, "Flylines").changed() {
                acts.push(Act::Flylines(flylines));
            }
            ui.checkbox(&mut self.nets.probing, "Probe")
                .on_hover_text("A click in the picture shows the net under it (N)");
            ui.checkbox(&mut self.link.cross_probe, "Cross-probe");
            if let Some(shown) = &self.nets.shown {
                ui.separator();
                ui.label(format!("Shown: {shown}"));
                if ui.button("Clear").clicked() {
                    acts.push(Act::Clear);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    acts.push(Act::Close);
                }
            });
        });
        if self.nets_stale() {
            ui.colored_label(
                WARN,
                "The layout changed since the extraction: press Update to see it.",
            );
        }
        ui.separator();
        let Some(last) = &self.nets.last else {
            if !self.nets.running() {
                ui.weak(
                    "Update extracts the layout with the PDK's LVS deck and compares its nets \
                     with the schematic's (as saved).",
                );
            }
            return;
        };
        let run = match last {
            Err(error) => {
                ui.colored_label(BAD, "The extraction failed");
                ui.label(error);
                return;
            }
            Ok(run) => run,
        };
        let report = &run.report;
        let (ok, open, short) = report.counts();
        let summary = format!("{ok} ok, {open} open, {short} shorted");
        if report.clean() {
            ui.colored_label(
                OK,
                format!("{summary}: the layout's nets match the schematic."),
            );
        } else {
            ui.label(format!("{summary}. Click a net to show it."));
        }
        let id = self.id;
        ScrollArea::vertical()
            .id_salt(("klayout-nets-scroll", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(("klayout-nets-grid", id))
                    .striped(true)
                    .num_columns(3)
                    .show(ui, |ui| {
                        let mut list: Vec<&nets::NetResult> = report.nets.iter().collect();
                        // The problems first.
                        list.sort_by_key(|n| (n.status == NetStatus::Ok, n.name.clone()));
                        for net in list {
                            let selected = self.nets.shown.as_deref() == Some(net.name.as_str());
                            if ui
                                .selectable_label(selected, &net.name)
                                .on_hover_text("Show it in the layout")
                                .clicked()
                            {
                                acts.push(Act::Show(net.name.clone()));
                            }
                            ui.colored_label(status_color(net.status), net.status.label());
                            let mut what = Vec::new();
                            if net.pieces.len() > 1 {
                                what.push(format!("{} pieces", net.pieces.len()));
                            }
                            if !net.shorted_with.is_empty() {
                                what.push(format!("with {}", net.shorted_with.join(", ")));
                            }
                            if !net.missing.is_empty() {
                                what.push(format!("not placed: {}", net.missing.join(" ")));
                            }
                            ui.label(what.join("; "));
                            ui.end_row();
                        }
                    });
                let lists: [(&str, Vec<String>); 5] = [
                    (
                        "Shorts",
                        report
                            .shorts
                            .iter()
                            .map(|s| format!("{}: {}", s.label, s.schematic_nets.join(", ")))
                            .collect(),
                    ),
                    ("Unconnected terminals", report.unconnected.clone()),
                    ("Not in the layout", report.unplaced.clone()),
                    (
                        "Pins",
                        report
                            .pins
                            .iter()
                            .filter(|p| p.status != "ok")
                            .map(|p| format!("{}: {}", p.name, p.status))
                            .collect(),
                    ),
                    (
                        "Notes",
                        report
                            .extra_devices
                            .iter()
                            .map(|d| format!("{d}: belongs to no schematic instance"))
                            .chain(report.problems.iter().cloned())
                            .collect(),
                    ),
                ];
                for (title, items) in lists {
                    if items.is_empty() {
                        continue;
                    }
                    CollapsingHeader::new(format!("{title} ({})", items.len()))
                        .id_salt(("klayout-nets-list", id, title))
                        .default_open(title != "Notes")
                        .show(ui, |ui| {
                            for item in items {
                                ui.label(item);
                            }
                        });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.weak("Database");
                    ui.label(
                        RichText::new(run.lvsdb.display().to_string())
                            .monospace()
                            .small(),
                    );
                });
            });
    }
}
