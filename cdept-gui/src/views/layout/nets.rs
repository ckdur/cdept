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
use libcdept::link::nets::{
    self, Connectivity, FlylineStyle, LineStyle, LiveNets, NetReport, NetRun, NetStatus,
};
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

/// The connectivity traced on the layout after edits (helper `live.py`), compared again.
pub(super) struct Live {
    pub nets: LiveNets,
    pub connectivity: Connectivity,
    pub report: NetReport,
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
    live_asked: Option<u64>,
    /// Flylines the helper drew, as it last said.
    pub drawn: usize,
    /// Schematic net names for the schematic tabs (taken by `coordinate_links`).
    pub probe_out: Option<Vec<String>>,
    /// The schematic nets last exchanged with the schematic tabs.
    pub probed: Vec<String>,
    /// The last extraction's terminals traced again after edits.
    live: Option<Live>,
    /// The helper epoch live tracing was set up in.
    live_set: Option<u64>,
    /// Why live tracing is not there or stopped (a sentence), else empty.
    pub live_note: String,
    /// How flylines look: the user's settings with the project's on top.
    pub style: FlylineStyle,
    /// The user's settings as edited in the style window (written back to the
    /// settings by `LayoutView::ui`).
    pub user_style: FlylineStyle,
    /// `user_style` was changed here: to be written to the settings.
    pub user_dirty: bool,
    /// The style the helper got last.
    sent_style: Option<FlylineStyle>,
    /// The style window is open.
    pub style_open: bool,
    /// Update Connectivity was asked for: waiting for the schematic to be saved first
    /// (`coordinate_links` saves it, then starts).
    pub requested: bool,
    /// The save of the schematic asked for: its file and the request.
    pub save_wait: Option<(PathBuf, u64)>,
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
            live_asked: None,
            drawn: 0,
            probe_out: None,
            probed: Vec::new(),
            live: None,
            live_set: None,
            live_note: String::new(),
            style: FlylineStyle::default(),
            user_style: FlylineStyle::default(),
            user_dirty: false,
            sent_style: None,
            style_open: false,
            requested: false,
            save_wait: None,
        }
    }
}

impl NetsState {
    pub fn running(&self) -> bool {
        self.job.is_some() || self.requested
    }

    fn run(&self) -> Option<&NetRun> {
        self.last.as_ref().and_then(|r| r.as_ref().ok())
    }

    /// A schematic net named by any of its names (`A`, or `x1/IN` inside x1: a net
    /// path from the top schematic, docs/LINKING.md §9): its name in the comparison.
    fn net_named(&self, key: &str) -> Option<String> {
        if self.report().is_some_and(|r| r.net(key).is_some()) {
            return Some(key.to_string());
        }
        let run = self.run()?;
        run.flat
            .nets
            .iter()
            .find(|n| n.names.iter().any(|k| k == key))
            .map(|n| n.name.clone())
    }

    /// Every name of these schematic nets, through the hierarchy (`A`, `x1/IN`): what
    /// a schematic tab picks those of the level it shows from.
    fn aliases(&self, names: &[String]) -> Vec<String> {
        let Some(run) = self.run() else {
            return names.to_vec();
        };
        let mut out = Vec::new();
        for name in names {
            match run.flat.nets.iter().find(|n| &n.name == name) {
                Some(net) => {
                    for k in std::iter::once(&net.name).chain(&net.names) {
                        if !out.contains(k) {
                            out.push(k.clone());
                        }
                    }
                }
                None => out.push(name.clone()),
            }
        }
        out
    }

    /// The comparison to show: with the live connectivity when there is one.
    fn report(&self) -> Option<&NetReport> {
        match &self.live {
            Some(live) => Some(&live.report),
            None => self.run().map(|r| &r.report),
        }
    }

    /// The layout connectivity the report is of.
    fn connectivity(&self) -> Option<&Connectivity> {
        match &self.live {
            Some(live) => Some(&live.connectivity),
            None => self.run().map(|r| &r.connectivity),
        }
    }

    /// Compare again: the parallel groups changed, or new live connectivity came.
    fn recompare(&mut self) {
        let Some(Ok(run)) = &mut self.last else {
            return;
        };
        let joined = nets::joined_groups(&run.parallel);
        run.report = nets::compare(&run.flat, &run.connectivity, &run.map, &joined);
        if let Some(live) = &mut self.live {
            live.report = nets::compare(&run.flat, &live.connectivity, &run.map, &joined);
        }
    }

    /// A new helper: questions to the old one get no answer, and it has no database.
    pub fn forget_questions(&mut self) {
        self.probe_asked = None;
        self.flylines_asked = None;
        self.loaded = None;
        self.live_set = None;
        self.sent_style = None;
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
    /// Join a parallel group (its index) or not.
    Parallel(usize, bool),
}

/// An egui colour button for a `#rrggbb` setting.
fn color_field(ui: &mut Ui, text: &mut String) {
    let mut rgb = nets::parse_color(text).unwrap_or([255, 0, 255]);
    if ui.color_edit_button_srgb(&mut rgb).changed() {
        *text = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    }
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
    /// the schematic's, as saved. When the schematic is open with unsaved changes it is
    /// saved first (`coordinate_links` sees to it, then calls
    /// [`start_connectivity`](Self::start_connectivity)).
    pub fn update_connectivity(&mut self) {
        if self.nets.running() {
            return;
        }
        self.nets.open = true;
        self.nets.requested = true;
        self.nets.save_wait = None;
    }

    /// Update Connectivity asked for, waiting for the schematic's save.
    pub fn connectivity_requested(&self) -> bool {
        self.nets.requested
    }

    /// The save of the schematic asked for before the extraction, if any.
    pub fn connectivity_save_request(&self) -> Option<u64> {
        self.nets.save_wait.as_ref().map(|(_, id)| *id)
    }

    /// The schematic `file` is being saved (request `id`) before the extraction.
    pub fn connectivity_saving(&mut self, file: PathBuf, id: u64) {
        self.notify(
            NoticeLevel::Info,
            "Saving the schematic first: the nets are compared with it as saved".into(),
        );
        self.nets.save_wait = Some((file, id));
    }

    /// Start the extraction asked for (the schematic is saved).
    pub fn start_connectivity(&mut self) {
        self.nets.requested = false;
        self.nets.save_wait = None;
        if self.nets.job.is_some() {
            return;
        }
        match self.prepare_nets() {
            Ok(job) => self.nets.job = Some(job),
            Err(error) => self.nets.last = Some(Err(error)),
        }
    }

    /// The schematic could not be saved: no extraction.
    pub fn connectivity_not_started(&mut self, why: &str) {
        self.nets.requested = false;
        self.nets.save_wait = None;
        let text = format!("The schematic was not saved ({why}): connectivity not updated");
        self.notify(NoticeLevel::Warning, text.clone());
        self.nets.last = Some(Err(text));
    }

    fn prepare_nets(&mut self) -> Result<Job, String> {
        let Some((source, schematic)) = self.link.source.clone() else {
            return Err("this layout has no schematic to compare with".into());
        };
        if verify::find_deck(&self.project, verify::Check::Lvs).is_none() {
            let mut text = nets::NO_DECK.to_string();
            if let Some(first) = text.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            return Err(format!("{text}."));
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

    /// For a tutorial (guide.rs): `layout.net_ok:<net>` for each net that is whole,
    /// and `layout.nets_clean`, while the comparison is of the layout as it is.
    pub(super) fn net_facts(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.nets_stale() {
            return out;
        }
        let Some(report) = self.nets.report() else {
            return out;
        };
        for net in &report.nets {
            let name = &net.name;
            out.push(match net.status {
                nets::NetStatus::Ok => format!("layout.net_ok:{name}"),
                _ => format!("layout.net_bad:{name}"),
            });
        }
        if report.clean() {
            out.push("layout.nets_clean".into());
        }
        out
    }

    /// The layout changed since the connectivity was extracted, and live tracing has
    /// not caught up (or cannot: it is off, or instances were added).
    pub fn nets_stale(&self) -> bool {
        let changed = self
            .nets
            .made_at
            .is_some_and(|at| at != (self.epoch, self.revision));
        changed && !self.nets_live_current()
    }

    /// Live tracing covers the layout as it is now (and nothing was added since the
    /// extraction).
    pub fn nets_live_current(&self) -> bool {
        self.nets.live.as_ref().is_some_and(|l| {
            l.nets.revision == self.revision
                && l.nets.added.is_empty()
                && self.nets.live_set == Some(self.epoch)
        })
    }

    /// The live connectivity from the helper: compare again, draw the new flylines.
    pub(super) fn live_nets(&mut self, live: LiveNets) {
        if !live.stopped.is_empty() {
            self.nets.live_note = live.stopped.clone();
            self.notify(NoticeLevel::Warning, format!("Nets: {}", live.stopped));
        }
        let Some(run) = self.nets.run() else {
            return;
        };
        let connectivity = run.connectivity.with_live(&live);
        let joined = nets::joined_groups(&run.parallel);
        let report = nets::compare(&run.flat, &connectivity, &run.map, &joined);
        self.nets.live = Some(Live {
            nets: live,
            connectivity,
            report,
        });
        if self.ready() && self.nets.loaded == Some(self.epoch) {
            self.send_flylines();
            if let Some(name) = self.nets.shown.clone() {
                self.highlight_net(&name, false);
            }
        }
    }

    /// Take the user's flyline settings (and the project's on top); a change is drawn.
    pub(super) fn take_style(&mut self, user: &FlylineStyle) {
        self.nets.user_style = user.clone();
        let style = user.with(&self.project.klayout.flylines);
        if style != self.nets.style {
            let live_changed =
                style.live != self.nets.style.live || style.delay_ms != self.nets.style.delay_ms;
            self.nets.style = style;
            if live_changed && self.ready() && self.nets.live_set == Some(self.epoch) {
                self.quiet(
                    "nets_live",
                    json!({"on": self.nets.style.live, "delay_ms": self.nets.style.delay_ms}),
                );
            }
        }
        if self.nets.sent_style.as_ref() != Some(&self.nets.style)
            && self.ready()
            && self.nets.loaded == Some(self.epoch)
            && self.nets.flylines_asked.is_none()
        {
            self.send_flylines();
            if let Some(name) = self.nets.shown.clone() {
                self.highlight_net(&name, false);
            }
        }
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
                    let top = if self.doc.cell.is_empty() {
                        self.cells
                            .as_ref()
                            .map(|c| c.top.clone())
                            .filter(|t| !t.is_empty())
                            .unwrap_or_else(|| verify::fallback_top(&self.file))
                    } else {
                        verify::expected_top(&self.file, &self.doc.cell)
                    };
                    let stream = verify::stream_of(&out, &value, &top);
                    let project = self.project.clone();
                    let recorded = nets::recorded_parallel(&self.file);
                    // The pins as the editor has them now (docs/LINKING.md §9).
                    let pins: std::collections::BTreeMap<String, libcdept::layout::Pin> =
                        serde_json::from_value(value["pins"].clone()).unwrap_or_default();
                    let (sender, result) = mpsc::channel();
                    let wake = ctx.clone();
                    std::thread::Builder::new()
                        .name("cdept-nets".into())
                        .spawn(move || {
                            let outcome = nets::run(
                                &project, source, &schematic, &stream, &dir, &recorded, &pins,
                            );
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
                    self.nets.live = None;
                    self.nets.live_set = None;
                    self.nets.live_note.clear();
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
            let json_file = run.json.clone();
            self.nets.loaded = Some(self.epoch);
            self.quiet("nets_load", json!({"lvsdb": lvsdb, "top": top}));
            // Live tracing from the terminals of this extraction (when the layout is
            // still the one extracted; a new helper has lost the edits anyway).
            if self
                .nets
                .made_at
                .is_some_and(|(_, revision)| revision == self.revision)
                || self
                    .nets
                    .made_at
                    .is_some_and(|(epoch, _)| epoch != self.epoch)
            {
                self.nets.live_set = Some(self.epoch);
                self.nets.live = None;
                self.nets.live_asked = Some(self.raw_call(
                    "nets_live",
                    json!({
                        "connectivity": json_file,
                        "on": self.nets.style.live,
                        "delay_ms": self.nets.style.delay_ms,
                    }),
                ));
            }
            self.send_flylines();
            if let Some(name) = self.nets.shown.clone() {
                self.highlight_net(&name, false);
            }
        }
        if let Some(id) = self.nets.live_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.nets.live_asked = None;
            let value = answer
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .unwrap_or(Value::Null);
            self.nets.live_note = if value["available"].as_bool() == Some(true) {
                String::new()
            } else {
                value["reason"]
                    .as_str()
                    .filter(|r| !r.is_empty())
                    .unwrap_or("live flylines are not available")
                    .to_string()
            };
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
        let lines: Vec<[f64; 4]> = match (self.nets.flylines, self.nets.report()) {
            (true, Some(report)) => report
                .flylines()
                .iter()
                .map(|l| [l.from[0], l.from[1], l.to[0], l.to[1]])
                .collect(),
            _ => Vec::new(),
        };
        let style = self.nets.style.clone();
        self.nets.flylines_asked = Some(self.raw_call(
            "nets_flylines",
            json!({
                "lines": lines,
                "color": style.color,
                "width": style.width,
                "style": style.line.name(),
                "opacity": f64::from(style.opacity) / 100.0,
            }),
        ));
        self.nets.sent_style = Some(style);
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
        let report = self
            .nets
            .report()
            .ok_or("there is no connectivity yet (Update Connectivity)")?;
        if report.net(name).is_none() {
            return Err(format!(
                "the schematic has no net {name} with terminals in the layout"
            ));
        }
        self.highlight_net(name, true);
        self.nets.probed = vec![name.to_string()];
        if self.link.cross_probe {
            self.nets.probe_out = Some(self.nets.aliases(&[name.to_string()]));
        }
        Ok(())
    }

    fn highlight_net(&mut self, name: &str, zoom: bool) {
        let net = self.nets.report().and_then(|report| report.net(name));
        let ids = net.map(|n| n.layout_nets()).unwrap_or_default();
        let shorted =
            net.is_some_and(|n| matches!(n.status, NetStatus::Short | NetStatus::OpenShort));
        let color = if shorted {
            self.nets.style.short.clone()
        } else {
            self.nets.style.highlight.clone()
        };
        let live = self.nets.live.is_some();
        self.nets.shown = Some(name.to_string());
        self.quiet(
            "nets_highlight",
            json!({"nets": ids, "zoom": zoom, "color": color, "live": live}),
        );
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
        if self.nets.report().is_none() {
            return;
        }
        // Names of any level of the hierarchy (`x1/IN`) are the nets they are part of.
        let known: Vec<String> = names
            .iter()
            .filter_map(|n| self.nets.net_named(n))
            .collect();
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
        let live = self.nets.live.is_some();
        self.nets.probe_asked =
            Some(self.raw_call("nets_probe", json!({"pixel": [x, y], "live": live})));
    }

    /// The answer to `nets_probe`: highlight the layout net, tell the schematic tabs.
    fn probed(&mut self, value: &Value) {
        let Some(id) = value["net"].as_u64().map(|n| n as usize) else {
            self.nets.probed.clear();
            if value["unbound"].as_bool() == Some(true) {
                // The helper shows it: a net no terminal of the extraction is on.
                self.nets.shown = Some("(a net of no terminal)".into());
                self.notify(NoticeLevel::Info, "A net with no device terminal".into());
            } else {
                self.clear_highlight();
                self.notify(NoticeLevel::Info, "No net there".into());
            }
            return;
        };
        let (label, names) = match (self.nets.connectivity(), self.nets.report()) {
            (Some(connectivity), Some(report)) => (
                connectivity.net_label(id),
                report
                    .nets_on(id)
                    .iter()
                    .map(|n| n.name.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => (format!("#{id}"), Vec::new()),
        };
        let live = self.nets.live.is_some();
        let color = self.nets.style.highlight.clone();
        self.quiet(
            "nets_highlight",
            json!({"nets": [id], "zoom": false, "live": live, "color": color}),
        );
        let text = if names.is_empty() {
            format!("Layout net {label}: no schematic net")
        } else {
            format!("Layout net {label}: schematic {}", names.join(", "))
        };
        self.notify(NoticeLevel::Info, text);
        self.nets.shown = Some(names.first().cloned().unwrap_or(label));
        self.nets.probed = names.clone();
        if self.link.cross_probe {
            self.nets.probe_out = Some(self.nets.aliases(&names));
        }
    }

    /// For the test driver and the status line: one line about the nets.
    pub fn nets_status(&self) -> String {
        if self.nets.save_wait.is_some() {
            return "saving the schematic".into();
        }
        if self.nets.running() {
            return "running".into();
        }
        match &self.nets.last {
            None => "none".into(),
            Some(Err(e)) => format!("failed: {e}"),
            Some(Ok(run)) => {
                let report = self.nets.report().unwrap_or(&run.report);
                let (ok, open, short) = report.counts();
                let mut text = format!("{ok} ok, {open} open, {short} shorted");
                if report.clean() {
                    text.push_str(", clean");
                }
                if self.nets_stale() {
                    text.push_str(" (out of date)");
                } else if self.nets.live.is_some() {
                    text.push_str(" (live)");
                }
                text
            }
        }
    }

    /// Schematic nets selected here since the last call, for the schematic tabs.
    pub fn take_net_probe_out(&mut self) -> Option<Vec<String>> {
        self.nets.probe_out.take()
    }

    /// What the net check says about pin `name` (`ok`, "the pin is on no wire", …), when
    /// there is a check.
    pub(super) fn net_pin_status(&self, name: &str) -> Option<String> {
        let report = self.nets.report()?;
        report
            .pins
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.status.clone())
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
        let update = ui
            .add_enabled(can, egui::Button::new("Update Connectivity"))
            .on_hover_text(
                "Extract the layout as it is now (PDK's LVS deck) and compare its nets with \
                 the schematic's: opens, shorts, flylines",
            );
        crate::guide::anchor(ui, "menu.layout.update_connectivity", &update);
        if update.clicked() {
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
        if ui.button("Flyline Style…").clicked() {
            self.nets.style_open = true;
            ui.close();
        }
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
                Act::Parallel(index, joined) => self.join_parallel(index, joined),
            }
        }
        self.style_window(ui.ctx());
    }

    /// Join the parallel group `index` (one flyline, one layout device for all) or not;
    /// recorded in the sidecar.
    pub fn join_parallel(&mut self, index: usize, joined: bool) {
        let groups = match &mut self.nets.last {
            Some(Ok(run)) => match run.parallel.get_mut(index) {
                Some(group) => {
                    group.joined = joined;
                    run.parallel.clone()
                }
                None => return,
            },
            _ => return,
        };
        self.nets.recompare();
        if let Err(error) = nets::record_parallel(&self.file, &groups) {
            self.notify(
                NoticeLevel::Warning,
                format!("Parallel devices not recorded: {error:#}"),
            );
        }
        if self.ready() && self.nets.loaded == Some(self.epoch) {
            self.send_flylines();
        }
    }

    /// The parallel groups of the last comparison: members and whether joined.
    pub fn parallel_groups(&self) -> Vec<(Vec<String>, bool)> {
        self.nets
            .run()
            .map(|run| {
                run.parallel
                    .iter()
                    .map(|g| (g.members.clone(), g.joined))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The window to change how flylines look (the user's settings).
    fn style_window(&mut self, ctx: &egui::Context) {
        if !self.nets.style_open {
            return;
        }
        let mut open = true;
        let mut style = self.nets.user_style.clone();
        egui::Window::new("Flyline Style")
            .id(egui::Id::new(("klayout-flyline-style", self.id)))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                egui::Grid::new(("klayout-flyline-style-grid", self.id))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Colour");
                        color_field(ui, &mut style.color);
                        ui.end_row();
                        ui.label("Width");
                        ui.add(egui::Slider::new(&mut style.width, 1..=8).suffix(" px"));
                        ui.end_row();
                        ui.label("Line");
                        egui::ComboBox::from_id_salt(("klayout-flyline-line", self.id))
                            .selected_text(style.line.name())
                            .show_ui(ui, |ui| {
                                for line in LineStyle::ALL {
                                    ui.selectable_value(&mut style.line, line, line.name());
                                }
                            });
                        ui.end_row();
                        ui.label("Opacity");
                        ui.add(egui::Slider::new(&mut style.opacity, 10..=100).suffix(" %"));
                        ui.end_row();
                        ui.label("Net shown");
                        color_field(ui, &mut style.highlight);
                        ui.end_row();
                        ui.label("Shorted net");
                        color_field(ui, &mut style.short);
                        ui.end_row();
                        ui.label("Pins")
                            .on_hover_text("Pins CDePT draws: those without a pin layer");
                        color_field(ui, &mut style.pin);
                        ui.end_row();
                        ui.label("Follow edits");
                        ui.checkbox(&mut style.live, "").on_hover_text(
                            "Trace the wiring again after each change (devices as \
                                 extracted); off: flylines change with Update Connectivity",
                        );
                        ui.end_row();
                        ui.label("After a change");
                        ui.add(
                            egui::Slider::new(&mut style.delay_ms, 50..=2000)
                                .suffix(" ms")
                                .logarithmic(true),
                        );
                        ui.end_row();
                    });
                if !self.project.klayout.flylines.is_empty() {
                    ui.weak("The project's [klayout.flylines] in cdept.toml takes precedence.");
                }
                if ui.button("Defaults").clicked() {
                    style = FlylineStyle::default();
                }
            });
        self.nets.style_open = open;
        if style != self.nets.user_style {
            self.nets.user_style = style;
            self.nets.user_dirty = true;
        }
    }

    /// Change the user's flyline style (as the style window does), for scripts.
    pub fn set_flyline_style(&mut self, key: &str, value: &str) -> Result<(), String> {
        let style = &mut self.nets.user_style;
        let bad = || format!("{key}: {value} is not understood");
        match key {
            "color" | "highlight" | "short" | "pin" => {
                if nets::parse_color(value).is_none() {
                    return Err(bad());
                }
                let v = value.to_string();
                match key {
                    "color" => style.color = v,
                    "highlight" => style.highlight = v,
                    "pin" => style.pin = v,
                    _ => style.short = v,
                }
            }
            "width" => style.width = value.parse().map_err(|_| bad())?,
            "opacity" => style.opacity = value.parse().map_err(|_| bad())?,
            "delay_ms" => style.delay_ms = value.parse().map_err(|_| bad())?,
            "line" => style.line = LineStyle::parse(value).ok_or_else(bad)?,
            "live" => style.live = matches!(value, "on" | "yes" | "true" | "1"),
            _ => return Err(format!("no flyline setting {key}")),
        }
        self.nets.user_dirty = true;
        Ok(())
    }

    /// The style the helper drew the flylines with last.
    pub fn flyline_style_sent(&self) -> Option<&FlylineStyle> {
        self.nets.sent_style.as_ref()
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
            if ui
                .button("Style…")
                .on_hover_text("How flylines look, and whether they follow the edits")
                .clicked()
            {
                self.nets.style_open = true;
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
        if let Some(live) = &self.nets.live
            && !live.nets.added.is_empty()
        {
            ui.colored_label(
                WARN,
                format!(
                    "Placed since the extraction: {}. Their devices count after Update.",
                    live.nets.added.join(", ")
                ),
            );
        } else if self.nets_stale() {
            ui.colored_label(
                WARN,
                "The layout changed since the extraction: press Update to see it.",
            );
        } else if let Some(live) = &self.nets.live
            && self.nets_live_current()
        {
            ui.weak(format!(
                "Flylines follow the edits (traced in {:.0} ms; {} terminals kept as \
                 extracted).",
                live.nets.ms, live.nets.untraced
            ));
        }
        if !self.nets.live_note.is_empty() && self.nets.style.live {
            ui.weak(&self.nets.live_note);
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
        let report = self.nets.report().unwrap_or(&run.report);
        let connectivity = self.nets.connectivity().unwrap_or(&run.connectivity);
        let _ = connectivity;
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
                if !run.parallel.is_empty() {
                    CollapsingHeader::new(format!("Parallel devices ({})", run.parallel.len()))
                        .id_salt(("klayout-nets-parallel", id))
                        .default_open(true)
                        .show(ui, |ui| {
                            for (index, group) in run.parallel.iter().enumerate() {
                                let mut joined = group.joined;
                                let mut text = group.members.join(" ");
                                if group.manual {
                                    text.push_str(" (yours)");
                                }
                                if ui
                                    .checkbox(&mut joined, text)
                                    .on_hover_text(
                                        "Joined: one flyline for the group, and one layout \
                                         device may stand for all of them",
                                    )
                                    .changed()
                                {
                                    acts.push(Act::Parallel(index, joined));
                                }
                            }
                        });
                }
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
