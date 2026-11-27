//! The calculator panel of the Simulation tab: an expression on one run point or on all
//! of them, with completion, a function browser with help and a signal browser; scalar
//! results in a table, waveform results sent to the built-in UberWave (plain, Bode or
//! eye), markers where scalars were measured; "Add as output" turns the expression into
//! an `[[output]]` (and a spec) of the testbench; and the viewer's cursors followed live:
//! their positions, the plotted signals under them, and the outputs that use `cursor_a`
//! / `cursor_b`, evaluated again when a cursor moves.
//!
//! Everything runs in background threads; the panel polls their slots.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Color32, Grid, RichText, ScrollArea, TextEdit, Ui};
use libcdept::Project;
use libcdept::calc::funcs;
use libcdept::wave::{self, PointRaw, calc as wcalc};

/// How waveform results go to the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LayoutChoice {
    #[default]
    Plain,
    Bode,
    Eye,
}

/// One row of the results table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResultRow {
    pub point: String,
    /// A number (as text), `waveform` lines, or the error.
    pub text: String,
    pub value: Option<f64>,
    pub error: bool,
}

/// What a finished evaluation left.
#[derive(Debug, Clone, Default)]
struct Finished {
    rows: Vec<ResultRow>,
    notes: Vec<String>,
    /// "Send to tab": what goes to a waveform tab.
    to_tab: Option<TabSend>,
}

/// Calculator results for a waveform tab of CDePT's own viewer ("Send to tab"): the
/// waveforms of each point as arrays (no file in between) and the markers.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TabSend {
    /// The testbench view: library, cell, view.
    pub run: (String, String, String),
    /// The points evaluated (their raw files, for a new tab).
    pub points: Vec<PointRaw>,
    /// Point, trace (named without the point: one signal of every point).
    pub traces: Vec<(String, wave::ipc::Trace)>,
    pub markers: Vec<crate::views::wave::Marker>,
    /// Nothing to send: why (said in the panel instead).
    pub nothing: bool,
    /// The layout chosen in the panel: plain, Bode (gain and phase panes, margins), eye.
    pub layout: wcalc::Layout,
}

/// How an evaluation ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendTo {
    /// Only the numbers.
    No,
    /// Waveforms and markers to the built-in UberWave (or its process).
    UberWave,
    /// Waveforms and markers to a waveform tab.
    Tab,
}

/// The values under the viewer's cursors.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CursorView {
    pub a: Option<f64>,
    pub b: Option<f64>,
    /// Plotted signal, value at A, value at B.
    pub signals: Vec<(String, Option<f64>, Option<f64>)>,
    /// Output name, its value with cursor_a/cursor_b set, the spec verdict (if any).
    pub outputs: Vec<(String, Result<f64, String>)>,
}

/// Cursor moves reported by the viewer (from its thread) or a waveform tab: a, b,
/// generation.
static CURSORS: Mutex<(Option<f64>, Option<f64>)> = Mutex::new((None, None));
static CURSOR_GEN: AtomicU64 = AtomicU64::new(0);
static LISTENING: Mutex<bool> = Mutex::new(false);
/// The values under the cursors when a waveform tab reported them last (the built-in
/// UberWave is asked for its own); `None` when UberWave reported last.
static TAB_SIGNALS: Mutex<Option<Vec<CursorValues>>> = Mutex::new(None);

/// A signal and its values at cursor A and B.
type CursorValues = (String, Option<f64>, Option<f64>);
/// A waveform tab has reported cursors (the cursor section works without UberWave).
static TAB_CURSORS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A waveform tab's cursors moved (or the values under them changed): the spec table
/// follows them as it follows the built-in UberWave's.
pub(crate) fn publish_tab_cursors(
    a: Option<f64>,
    b: Option<f64>,
    signals: Vec<(String, Option<f64>, Option<f64>)>,
) {
    *CURSORS.lock().unwrap_or_else(|e| e.into_inner()) = (a, b);
    *TAB_SIGNALS.lock().unwrap_or_else(|e| e.into_inner()) = Some(signals);
    TAB_CURSORS.store(true, Ordering::SeqCst);
    CURSOR_GEN.fetch_add(1, Ordering::SeqCst);
}

/// Ask the built-in viewer to report cursor moves (once per process).
fn listen(ctx: &egui::Context) {
    let mut on = LISTENING.lock().unwrap_or_else(|e| e.into_inner());
    if *on {
        return;
    }
    *on = true;
    let ctx = ctx.clone();
    wave::embedded::set_cursor_listener(Some(Arc::new(move |a, b| {
        *CURSORS.lock().unwrap_or_else(|e| e.into_inner()) = (a, b);
        *TAB_SIGNALS.lock().unwrap_or_else(|e| e.into_inner()) = None;
        CURSOR_GEN.fetch_add(1, Ordering::SeqCst);
        ctx.request_repaint();
    })));
}

/// An output to add to the testbench ("Add as output").
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewOutput {
    pub name: String,
    pub expr: String,
    pub plot: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

type Slot<T> = Arc<Mutex<Option<T>>>;

/// The panel's state (one per Simulation tab).
#[derive(Default)]
pub(crate) struct CalcPanel {
    pub expr: String,
    /// `None`: every point.
    pub point: Option<String>,
    pub plot: String,
    pub markers: bool,
    pub layout: LayoutChoice,
    pub eye_period: String,
    pub eye_offset: String,
    pub show_functions: bool,
    pub func_filter: String,
    pub show_signals: bool,
    pub sig_filter: String,
    signals: Option<(PathBuf, Vec<String>)>,
    pub rows: Vec<ResultRow>,
    pub notes: Vec<String>,
    pending: Option<Slot<Finished>>,
    pub out_name: String,
    pub out_min: String,
    pub out_max: String,
    pub follow: bool,
    pub cursor: CursorView,
    cursor_seen: u64,
    cursor_pending: Option<Slot<CursorView>>,
    /// Something to say under the buttons (an added output, a bad eye period).
    pub status: Option<(String, bool)>,
    started: bool,
    /// "Send to tab" results not yet handed to a waveform tab.
    to_tab: Option<TabSend>,
}

impl CalcPanel {
    pub(crate) fn new() -> CalcPanel {
        CalcPanel {
            markers: true,
            follow: true,
            ..Default::default()
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// "Send to tab" results that are ready, once (the app hands them to a tab).
    pub(crate) fn take_tab_send(&mut self) -> Option<TabSend> {
        self.take_finished();
        self.to_tab.take()
    }

    fn take_finished(&mut self) {
        let done = self
            .pending
            .as_ref()
            .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take());
        if let Some(done) = done {
            self.rows = done.rows;
            self.notes = done.notes;
            if done.to_tab.is_some() {
                self.to_tab = done.to_tab;
            }
            self.pending = None;
        }
        let view = self
            .cursor_pending
            .as_ref()
            .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take());
        if let Some(view) = view {
            self.cursor = view;
            self.cursor_pending = None;
        }
    }

    /// Wait (a script) until the evaluation in progress is done, up to `secs`.
    pub(crate) fn wait(&mut self, secs: u64) -> Result<(), String> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while self.busy() {
            self.take_finished();
            if !self.busy() {
                break;
            }
            if std::time::Instant::now() >= end {
                return Err(format!("the calculator did not finish within {secs} s"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }

    fn layout(&self) -> Result<wcalc::Layout, String> {
        Ok(match self.layout {
            LayoutChoice::Plain => wcalc::Layout::Plain,
            LayoutChoice::Bode => wcalc::Layout::Bode,
            LayoutChoice::Eye => {
                let num = |t: &str, what: &str| -> Result<f64, String> {
                    libcdept::calc::parse_spice_number(t.trim())
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| format!("the eye {what} '{t}' is not a number"))
                };
                let period = num(&self.eye_period, "period")?;
                if period <= 0.0 {
                    return Err("the eye period must be positive".into());
                }
                let offset = if self.eye_offset.trim().is_empty() {
                    0.0
                } else {
                    num(&self.eye_offset, "offset")?
                };
                wcalc::Layout::Eye { period, offset }
            }
        })
    }

    /// Evaluate (and send waveforms and markers to UberWave or to a waveform tab) in a
    /// thread.
    pub(crate) fn start(
        &mut self,
        project: &Project,
        run: (&str, &str, &str),
        send: SendTo,
        ctx: Option<&egui::Context>,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("an evaluation is still running".into());
        }
        let expr = self.expr.trim().to_string();
        if expr.is_empty() {
            return Err("type an expression first".into());
        }
        let layout = self.layout()?;
        let points = select(project, run, self.point.as_deref())?;
        let slot: Slot<Finished> = Arc::default();
        self.pending = Some(slot.clone());
        self.rows.clear();
        self.notes = vec![
            match send {
                SendTo::No => "evaluating...",
                SendTo::UberWave => "sending...",
                SendTo::Tab => "evaluating for the waveform tab...",
            }
            .into(),
        ];
        let (project, l, c, v) = (
            project.clone(),
            run.0.to_string(),
            run.1.to_string(),
            run.2.to_string(),
        );
        let plot = (!self.plot.trim().is_empty()).then(|| self.plot.trim().to_string());
        let markers = self.markers;
        let extra = cursor_names();
        let ctx = ctx.cloned();
        let layout_name = self.layout;
        spawn(move || {
            let finished = if send == SendTo::Tab {
                let outcomes =
                    wcalc::evaluate_points_with(&points, &expr, None, plot.as_deref(), &extra);
                let mut f = rows_of(&outcomes);
                let mut tab = tab_send((&l, &c, &v), &points, &outcomes, &expr, markers);
                tab.layout = layout;
                let sent: usize = tab.traces.len();
                if sent > 0 {
                    f.notes.push(format!(
                        "{sent} waveform(s) to a waveform tab{}",
                        if tab.markers.is_empty() {
                            String::new()
                        } else {
                            format!(", {} marker(s)", tab.markers.len())
                        }
                    ));
                }
                match layout_name {
                    LayoutChoice::Bode => f.notes.push(
                        "Bode layout: gain and phase panes on a log frequency axis, margins as markers".into(),
                    ),
                    LayoutChoice::Eye => f.notes.push("eye layout: the waveform folded in a pane of its own".into()),
                    LayoutChoice::Plain => {}
                }
                if !tab.nothing {
                    f.to_tab = Some(tab);
                } else {
                    f.notes
                        .push("nothing to send to a tab (no waveform result, no marker)".into());
                }
                f
            } else if send == SendTo::UberWave {
                let req = wcalc::Request {
                    expr: expr.clone(),
                    plot,
                    markers,
                    layout,
                    extra,
                    ..Default::default()
                };
                let sent = wcalc::send(&project, &l, &c, &v, &points, &req);
                let mut f = rows_of(&sent.outcomes);
                for (name, n) in &sent.pushed {
                    f.notes.push(format!("sent '{name}' ({n} points)"));
                }
                if let Some(p) = &sent.problem {
                    f.notes.push(format!("not sent: {p}"));
                }
                f.notes.extend(sent.notes.iter().cloned());
                f
            } else {
                rows_of(&wcalc::evaluate_points_with(
                    &points,
                    &expr,
                    None,
                    plot.as_deref(),
                    &extra,
                ))
            };
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(finished);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        Ok(())
    }

    /// The output "Add as output" would add, or why not.
    pub(crate) fn new_output(&self) -> Result<NewOutput, String> {
        let name = self.out_name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err("give the output a name (one word)".into());
        }
        let expr = self.expr.trim();
        libcdept::calc::parse(expr).map_err(|e| e.render(expr))?;
        let num = |t: &str, what: &str| -> Result<Option<f64>, String> {
            let t = t.trim();
            if t.is_empty() {
                return Ok(None);
            }
            libcdept::calc::parse_spice_number(t)
                .map(Some)
                .ok_or_else(|| format!("{what} '{t}' is not a number"))
        };
        Ok(NewOutput {
            name: name.to_string(),
            expr: expr.to_string(),
            plot: self.plot.trim().to_string(),
            min: num(&self.out_min, "min")?,
            max: num(&self.out_max, "max")?,
        })
    }

    /// Fetch the values under the cursors and evaluate the outputs that use them (in a
    /// thread) when a cursor moved.
    fn refresh_cursors(
        &mut self,
        project: &Project,
        run: (&str, &str, &str),
        outputs: &[(String, String, String)],
        ctx: &egui::Context,
    ) {
        let generation = CURSOR_GEN.load(Ordering::SeqCst);
        if generation == self.cursor_seen || self.cursor_pending.is_some() {
            return;
        }
        self.cursor_seen = generation;
        let (a, b) = *CURSORS.lock().unwrap_or_else(|e| e.into_inner());
        let uses: Vec<(String, String, String)> = outputs
            .iter()
            .filter(|(_, e, _)| {
                let e = e.to_lowercase();
                e.contains("cursor_a") || e.contains("cursor_b")
            })
            .cloned()
            .collect();
        let point = self.point.clone().or_else(|| {
            wave::points(project, run.0, run.1, run.2)
                .first()
                .map(|p| p.point.clone())
        });
        let raw = point.map(|p| {
            libcdept::sim::runs_dir(project, run.0, run.1, run.2)
                .join(p)
                .join("out.raw")
        });
        let slot: Slot<CursorView> = Arc::default();
        self.cursor_pending = Some(slot.clone());
        let ctx = ctx.clone();
        spawn(move || {
            let view = cursor_view(a, b, raw.as_deref(), &uses);
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(view);
            ctx.request_repaint();
        });
    }

    /// The panel. Returns an output to add to the testbench when asked.
    pub(crate) fn ui(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        run: (&str, &str, &str),
        outputs: &[(String, String, String)],
        specs: &[libcdept::sim::Spec],
    ) -> Option<NewOutput> {
        if !self.started {
            *self = CalcPanel::new();
            self.started = true;
        }
        self.take_finished();
        let points = wave::points(project, run.0, run.1, run.2);
        let mut add = None;
        egui::CollapsingHeader::new("Calculator")
            .id_salt("sim-calc")
            .default_open(true)
            .show(ui, |ui| {
                if points.is_empty() {
                    ui.weak("No run point of this testbench has a raw file yet: run it first.");
                    return;
                }
                add = self.body(ui, project, run, &points, outputs, specs);
            });
        if self.busy() || self.cursor_pending.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        add
    }

    #[allow(clippy::too_many_arguments)]
    fn body(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        run: (&str, &str, &str),
        points: &[PointRaw],
        outputs: &[(String, String, String)],
        specs: &[libcdept::sim::Spec],
    ) -> Option<NewOutput> {
        let mut add = None;
        if self
            .point
            .as_ref()
            .is_some_and(|p| !points.iter().any(|x| &x.point == p))
        {
            self.point = None;
        }
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("calc-point")
                .selected_text(self.point.clone().unwrap_or_else(|| format!("all points ({})", points.len())))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.point, None, format!("all points ({})", points.len()));
                    for p in points {
                        ui.selectable_value(&mut self.point, Some(p.point.clone()), &p.point);
                    }
                });
            ui.label("plot");
            ui.add(TextEdit::singleline(&mut self.plot).desired_width(40.0).hint_text("any"))
                .on_hover_text("Look only in this analysis of the raw file: tran, ac, dc, noise, op");
            ui.toggle_value(&mut self.show_functions, "Functions");
            ui.toggle_value(&mut self.show_signals, "Signals");
            ui.checkbox(&mut self.markers, "markers")
                .on_hover_text("Put markers in UberWave where scalar results were measured (crossings, edges, extremes)");
            egui::ComboBox::from_id_salt("calc-layout")
                .selected_text(match self.layout {
                    LayoutChoice::Plain => "plain",
                    LayoutChoice::Bode => "Bode",
                    LayoutChoice::Eye => "eye",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.layout, LayoutChoice::Plain, "plain")
                        .on_hover_text("Traces into the active pane");
                    ui.selectable_value(&mut self.layout, LayoutChoice::Bode, "Bode")
                        .on_hover_text("A complex (AC) result: magnitude and phase in two panes, log frequency");
                    ui.selectable_value(&mut self.layout, LayoutChoice::Eye, "eye")
                        .on_hover_text("Every period drawn on top of the others");
                });
            if self.layout == LayoutChoice::Eye {
                ui.label("period");
                ui.add(TextEdit::singleline(&mut self.eye_period).desired_width(50.0).hint_text("1n"));
                ui.label("offset");
                ui.add(TextEdit::singleline(&mut self.eye_offset).desired_width(50.0).hint_text("0"));
            }
        });
        let mut evaluate = false;
        let mut send = false;
        let mut send_tab = false;
        ui.horizontal(|ui| {
            let edit = ui.add(
                TextEdit::singleline(&mut self.expr)
                    .id(egui::Id::new("calc-expr"))
                    .code_editor()
                    .hint_text("expression, e.g. enob(sample(v(\"dout\"), 100n), window=\"rect\")")
                    .desired_width((ui.available_width() - 290.0).max(120.0)),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                send = true;
            }
            let ready = !self.busy() && !self.expr.trim().is_empty();
            if ui
                .add_enabled(ready, egui::Button::new("Evaluate"))
                .on_hover_text("Evaluate on the point(s): numbers in the table, nothing sent")
                .clicked()
            {
                evaluate = true;
            }
            if ui
                .add_enabled(ready, egui::Button::new("Send to UberWave"))
                .on_hover_text("Evaluate and send waveforms (and markers) to the built-in UberWave")
                .clicked()
            {
                send = true;
            }
            if ui
                .add_enabled(ready, egui::Button::new("Send to tab"))
                .on_hover_text("Evaluate and put the waveforms (and markers) into a waveform tab of CDePT's own viewer: the newest tab of this testbench, else a new one")
                .clicked()
            {
                send_tab = true;
            }
        });
        help_line(ui, &self.expr);
        suggestions(ui, &mut self.expr);
        if self.show_functions {
            self.function_browser(ui);
        }
        if self.show_signals {
            self.signal_browser(ui, points);
        }
        if evaluate || send || send_tab {
            let ctx = ui.ctx().clone();
            let how = if send_tab {
                SendTo::Tab
            } else if send {
                SendTo::UberWave
            } else {
                SendTo::No
            };
            if let Err(e) = self.start(project, run, how, Some(&ctx)) {
                self.status = Some((e, true));
            } else {
                self.status = None;
            }
        }
        if let Some((text, bad)) = &self.status {
            let color = if *bad {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, text);
        }
        self.results_table(ui, outputs, specs);
        ui.horizontal_wrapped(|ui| {
            ui.label("Add as output");
            ui.add(TextEdit::singleline(&mut self.out_name).desired_width(100.0).hint_text("name"));
            ui.add(TextEdit::singleline(&mut self.out_min).desired_width(60.0).hint_text("min"));
            ui.add(TextEdit::singleline(&mut self.out_max).desired_width(60.0).hint_text("max"));
            if ui
                .button("Add to the testbench")
                .on_hover_text("An [[output]] with this expression (and a spec when min or max is given) in the setup form; Save to keep it")
                .clicked()
            {
                match self.new_output() {
                    Ok(o) => add = Some(o),
                    Err(e) => self.status = Some((e, true)),
                }
            }
        });
        self.cursor_ui(ui, project, run, outputs, specs);
        add
    }

    fn results_table(
        &self,
        ui: &mut Ui,
        outputs: &[(String, String, String)],
        specs: &[libcdept::sim::Spec],
    ) {
        if self.rows.is_empty() && self.notes.is_empty() {
            return;
        }
        // The spec of an output with this expression, if the testbench has one.
        let spec = outputs
            .iter()
            .find(|(_, e, _)| e.trim() == self.expr.trim())
            .and_then(|(n, _, _)| specs.iter().find(|s| s.name.eq_ignore_ascii_case(n)));
        Grid::new("calc-results")
            .striped(true)
            .num_columns(3)
            .show(ui, |ui| {
                for r in &self.rows {
                    ui.label(&r.point);
                    if r.error {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            r.text.lines().next().unwrap_or(""),
                        )
                        .on_hover_text(&r.text);
                    } else {
                        ui.label(RichText::new(&r.text).monospace());
                    }
                    match (spec, r.value) {
                        (Some(s), Some(v)) => {
                            let ok = s.min.is_none_or(|m| v >= m) && s.max.is_none_or(|m| v <= m);
                            ui.colored_label(
                                if ok {
                                    Color32::from_rgb(60, 170, 80)
                                } else {
                                    ui.visuals().error_fg_color
                                },
                                if ok { "PASS" } else { "FAIL" },
                            );
                        }
                        _ => {
                            ui.label("");
                        }
                    }
                    ui.end_row();
                }
            });
        for n in &self.notes {
            ui.label(RichText::new(n).weak());
        }
    }

    fn cursor_ui(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        run: (&str, &str, &str),
        outputs: &[(String, String, String)],
        specs: &[libcdept::sim::Spec],
    ) {
        egui::CollapsingHeader::new("Cursors (UberWave or a waveform tab)")
            .id_salt("calc-cursors")
            .default_open(false)
            .show(ui, |ui| {
                ui.checkbox(&mut self.follow, "follow the viewer's cursors")
                    .on_hover_text("The values under cursors A and B of the built-in UberWave or of a waveform tab (whichever moved last), and the outputs that use cursor_a / cursor_b, evaluated again when a cursor moves");
                if !self.follow {
                    return;
                }
                if !libcdept::wave::backend::embedded_available()
                    && !TAB_CURSORS.load(Ordering::SeqCst)
                {
                    ui.weak("needs the built-in UberWave, or cursors in a waveform tab");
                    return;
                }
                if libcdept::wave::backend::embedded_available() {
                    listen(ui.ctx());
                }
                let ctx = ui.ctx().clone();
                self.refresh_cursors(project, run, outputs, &ctx);
                let c = &self.cursor;
                let fmt = |v: Option<f64>| v.map(|v| format!("{v:.6e}")).unwrap_or_else(|| "—".into());
                ui.label(format!(
                    "A = {}   B = {}   B−A = {}   1/(B−A) = {}",
                    fmt(c.a),
                    fmt(c.b),
                    fmt(c.a.zip(c.b).map(|(a, b)| b - a)),
                    fmt(c.a.zip(c.b).map(|(a, b)| 1.0 / (b - a)).filter(|v| v.is_finite())),
                ));
                if !c.signals.is_empty() {
                    Grid::new("calc-cursor-signals").striped(true).num_columns(4).show(ui, |ui| {
                        ui.strong("signal");
                        ui.strong("at A");
                        ui.strong("at B");
                        ui.strong("B−A");
                        ui.end_row();
                        for (s, a, b) in &c.signals {
                            ui.label(s);
                            ui.label(fmt(*a));
                            ui.label(fmt(*b));
                            ui.label(fmt(a.zip(*b).map(|(a, b)| b - a)));
                            ui.end_row();
                        }
                    });
                }
                if c.outputs.is_empty() {
                    ui.weak("Outputs whose expression uses cursor_a or cursor_b (e.g. value_at(v(\"out\"), cursor_a)) are evaluated here as the cursors move.");
                } else {
                    Grid::new("calc-cursor-outputs").striped(true).num_columns(3).show(ui, |ui| {
                        for (name, value) in &c.outputs {
                            ui.label(name);
                            match value {
                                Ok(v) => {
                                    ui.label(format!("{v:.6e}"));
                                    match specs.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
                                        Some(s) => {
                                            let ok = s.min.is_none_or(|m| *v >= m) && s.max.is_none_or(|m| *v <= m);
                                            ui.colored_label(
                                                if ok { Color32::from_rgb(60, 170, 80) } else { ui.visuals().error_fg_color },
                                                if ok { "PASS" } else { "FAIL" },
                                            );
                                        }
                                        None => {
                                            ui.label("");
                                        }
                                    }
                                }
                                Err(e) => {
                                    ui.colored_label(ui.visuals().error_fg_color, e);
                                    ui.label("");
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
            });
    }

    fn function_browser(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("find");
            ui.add(
                TextEdit::singleline(&mut self.func_filter)
                    .desired_width(160.0)
                    .hint_text("name or word of the help"),
            );
        });
        let filter = self.func_filter.to_lowercase();
        ScrollArea::vertical()
            .id_salt("calc-funcs")
            .max_height(180.0)
            .show(ui, |ui| {
                for f in funcs::all() {
                    let sig = signature(f);
                    if !filter.is_empty()
                        && !sig.to_lowercase().contains(&filter)
                        && !f.doc.to_lowercase().contains(&filter)
                    {
                        continue;
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button(RichText::new(&sig).monospace())
                            .on_hover_text(f.doc)
                            .clicked()
                        {
                            insert_call(&mut self.expr, f.names[0]);
                        }
                        ui.label(RichText::new(f.doc).weak().small());
                    });
                }
            });
    }

    fn signal_browser(&mut self, ui: &mut Ui, points: &[PointRaw]) {
        let raw = self
            .point
            .as_ref()
            .and_then(|p| points.iter().find(|x| &x.point == p))
            .or(points.first())
            .map(|p| p.raw.clone());
        let Some(raw) = raw else { return };
        if self.signals.as_ref().is_none_or(|(f, _)| f != &raw) {
            let names = libcdept::raw::RawFile::open(&raw)
                .map(|r| {
                    let mut all = vec![];
                    for (i, p) in r.plots().iter().enumerate() {
                        for v in p.vars.iter().skip(1) {
                            let entry = format!(
                                "{}\t{}",
                                v.name,
                                if r.plots().len() > 1 { p.kind() } else { "" }
                            );
                            if !all.contains(&entry) {
                                all.push(entry);
                            }
                        }
                        let _ = i;
                    }
                    all
                })
                .unwrap_or_default();
            self.signals = Some((raw.clone(), names));
        }
        ui.horizontal(|ui| {
            ui.label("find");
            ui.add(TextEdit::singleline(&mut self.sig_filter).desired_width(160.0));
        });
        let filter = self.sig_filter.to_lowercase();
        let names = self
            .signals
            .as_ref()
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        ScrollArea::vertical()
            .id_salt("calc-sigs")
            .max_height(140.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for entry in names
                        .iter()
                        .filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter))
                        .take(400)
                    {
                        let (name, kind) = entry.split_once('\t').unwrap_or((entry, ""));
                        let label = if kind.is_empty() {
                            name.to_string()
                        } else {
                            format!("{name} [{kind}]")
                        };
                        if ui.small_button(label).clicked() {
                            self.expr.push_str(&call_for(name, kind));
                        }
                    }
                });
            });
    }
}

/// The values under cursors `a`/`b` of the built-in viewer and the outputs `uses`
/// (name, expression, plot) evaluated on `raw` with `cursor_a`/`cursor_b` set.
fn cursor_view(
    a: Option<f64>,
    b: Option<f64>,
    raw: Option<&std::path::Path>,
    uses: &[(String, String, String)],
) -> CursorView {
    let mut view = CursorView {
        a,
        b,
        ..Default::default()
    };
    let from_tab = TAB_SIGNALS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if let Some(values) = from_tab {
        view.signals = values;
    } else if let Some(v) = wave::embedded::running()
        && let Ok(c) = v.cursors()
    {
        view.signals = c.values.into_iter().map(|v| (v.signal, v.a, v.b)).collect();
    }
    let extra = cursor_names_of(a, b);
    for (name, expr, plot) in uses {
        let value = match raw {
            Some(r) => {
                let plot = (!plot.is_empty()).then_some(plot.as_str());
                match wcalc::evaluate_raw_with(r, expr, plot, &extra) {
                    Ok((v, _)) => v
                        .as_f64()
                        .ok_or_else(|| format!("not a number: {}", v.display())),
                    Err(e) => Err(format!("{e:#}").lines().next().unwrap_or("").to_string()),
                }
            }
            None => Err("no run point with a raw file".into()),
        };
        view.outputs.push((name.clone(), value));
    }
    view
}

/// `cursor_a` / `cursor_b` for the expression, when the viewer has them.
fn cursor_names() -> Vec<(String, f64)> {
    let (a, b) = *CURSORS.lock().unwrap_or_else(|e| e.into_inner());
    cursor_names_of(a, b)
}

fn cursor_names_of(a: Option<f64>, b: Option<f64>) -> Vec<(String, f64)> {
    let mut out = vec![];
    if let Some(a) = a {
        out.push(("cursor_a".to_string(), a));
    }
    if let Some(b) = b {
        out.push(("cursor_b".to_string(), b));
    }
    out
}

/// What "Send to tab" hands over: the waveforms of the points named as the signal
/// (the name, else the expression; a complex result as `(mag dB)` and `(phase)`), tagged
/// with their point, and the markers of the scalars.
fn tab_send(
    run: (&str, &str, &str),
    points: &[PointRaw],
    outcomes: &[wcalc::PointOutcome],
    expr: &str,
    markers: bool,
) -> TabSend {
    let mut traces = Vec::new();
    for o in outcomes {
        if let Ok(wcalc::Evaluated::Traces(ts)) = &o.result {
            for t in ts {
                let mut t = t.clone();
                // one signal of every point: the point is the source's tag, not the name
                let suffix = format!(" @ {}", o.point);
                if let Some(i) = t.name.find(&suffix) {
                    t.name = format!("{}{}", &t.name[..i], &t.name[i + suffix.len()..]);
                }
                traces.push((o.point.clone(), t));
            }
        }
    }
    let markers: Vec<crate::views::wave::Marker> = wcalc::markers_of(outcomes, expr, None, markers)
        .into_iter()
        .map(|m| crate::views::wave::Marker {
            label: m.label,
            x: m.x,
            y: m.y,
            x_unit: m.x_unit,
            y_unit: None,
        })
        .collect();
    TabSend {
        run: (run.0.to_string(), run.1.to_string(), run.2.to_string()),
        points: points.to_vec(),
        nothing: traces.is_empty() && markers.is_empty(),
        traces,
        markers,
        layout: wcalc::Layout::Plain,
    }
}

fn spawn(f: impl FnOnce() + Send + 'static) {
    // a big stack: the evaluator recurses over the expression
    let _ = std::thread::Builder::new()
        .name("cdept-calc".into())
        .stack_size(16 << 20)
        .spawn(f);
}

fn select(
    project: &Project,
    run: (&str, &str, &str),
    point: Option<&str>,
) -> Result<Vec<PointRaw>, String> {
    let names: Vec<String> = point.map(String::from).into_iter().collect();
    wave::select_points(project, run.0, run.1, run.2, &names).map_err(|e| format!("{e:#}"))
}

fn rows_of(outcomes: &[wcalc::PointOutcome]) -> Finished {
    let mut f = Finished::default();
    for o in outcomes {
        let (text, value, error) = match &o.result {
            Ok(wcalc::Evaluated::Scalar(s)) => (s.clone(), s.trim().parse::<f64>().ok(), false),
            Ok(wcalc::Evaluated::Traces(t)) => (
                t.iter()
                    .map(|t| format!("waveform '{}' ({} points)", t.name, t.x.len()))
                    .collect::<Vec<_>>()
                    .join(", "),
                None,
                false,
            ),
            Err(e) => (e.clone(), None, true),
        };
        f.rows.push(ResultRow {
            point: o.point.clone(),
            text,
            value,
            error,
        });
    }
    f
}

/// `name(params)` of a function, optional ones in brackets.
fn signature(f: &funcs::Func) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match p.strip_prefix('?') {
            Some(o) => format!("[{o}]"),
            None => p.to_string(),
        })
        .collect();
    format!("{}({})", f.names.join("|"), params.join(", "))
}

/// The word being typed at the end of the expression.
fn last_word(expr: &str) -> (usize, String) {
    let start = expr
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(0, |i| i + c_len(expr, i));
    (start, expr[start..].to_lowercase())
}

fn c_len(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(1, char::len_utf8)
}

/// The innermost function call the end of the expression is inside: its name.
fn enclosing_call(expr: &str) -> Option<String> {
    let mut depth = 0i32;
    let bytes: Vec<(usize, char)> = expr.char_indices().collect();
    for (k, &(i, c)) in bytes.iter().enumerate().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                let before = &expr[..i];
                let start = before
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .map_or(0, |j| j + c_len(before, j));
                let _ = k;
                let name = &before[start..];
                return (!name.is_empty()).then(|| name.to_lowercase());
            }
            _ => {}
        }
    }
    None
}

/// The signature and help of the function being typed or called.
fn help_line(ui: &mut Ui, expr: &str) {
    let (_, word) = last_word(expr);
    let name = funcs::lookup(&word)
        .map(|_| word.clone())
        .or_else(|| enclosing_call(expr));
    if let Some(f) = name.as_deref().and_then(funcs::lookup) {
        ui.label(RichText::new(signature(f)).monospace().strong());
        ui.label(RichText::new(f.doc).weak());
    }
}

/// The function names that start like the word at the end of the expression.
fn suggestions(ui: &mut Ui, expr: &mut String) {
    let (start, word) = last_word(expr);
    if word.len() < 2 {
        return;
    }
    let mut names: Vec<&str> = funcs::all()
        .iter()
        .flat_map(|f| f.names.iter().copied())
        .filter(|n| n.starts_with(&word) && *n != word)
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for n in names.into_iter().take(12) {
            let help = funcs::lookup(n).map(|f| f.doc).unwrap_or("");
            if ui.small_button(n).on_hover_text(help).clicked() {
                expr.truncate(start);
                expr.push_str(n);
                expr.push('(');
            }
        }
    });
}

/// Append `name(` (or replace the word being typed with it).
fn insert_call(expr: &mut String, name: &str) {
    let (start, word) = last_word(expr);
    if !word.is_empty() && name.starts_with(&word) {
        expr.truncate(start);
    }
    expr.push_str(name);
    expr.push('(');
}

/// The call that reads a signal: `v("out")`, `i("vdd")` for a current, `sig("...")`.
fn call_for(name: &str, kind: &str) -> String {
    let analysis = if kind.is_empty() {
        String::new()
    } else {
        format!(", analysis=\"{kind}\"")
    };
    let lower = name.to_lowercase();
    if let Some(inner) = lower.strip_prefix("v(").and_then(|s| s.strip_suffix(')')) {
        let _ = inner;
        return format!("v(\"{}\"{analysis})", &name[2..name.len() - 1]);
    }
    if let Some(inner) = lower.strip_prefix("i(").and_then(|s| s.strip_suffix(')')) {
        let _ = inner;
        return format!("i(\"{}\"{analysis})", &name[2..name.len() - 1]);
    }
    if lower.ends_with("#branch") {
        return format!("i(\"{}\"{analysis})", &name[..name.len() - 7]);
    }
    format!("sig(\"{name}\"{analysis})")
}

/// `sim calc ...` of a test script (docs/GUI-TESTING.md): what the panel's widgets do.
pub(crate) fn script(
    view: &mut super::SimulationView,
    project: &Project,
    args: &[String],
) -> Option<Result<(), String>> {
    if args.first().map(String::as_str) != Some("calc") {
        return None;
    }
    Some(script_calc(view, project, &args[1..]))
}

fn script_calc(
    view: &mut super::SimulationView,
    project: &Project,
    args: &[String],
) -> Result<(), String> {
    const USAGE: &str = "usage: sim calc expr <text> | point <name|all> | plot <kind|any> | layout plain|bode|eye <period> [offset] | \
        markers on|off | evaluate | send | send_tab | follow | expect <point|*> <text> | expect_value <point> <min> <max> | expect_note <text> | \
        add_output <name> [min] [max] | cursors <a> <b> | expect_cursor_output <name> <min> <max> | expect_cursor_signal <name> | \
        functions <filter> | signals <filter>";
    let run = view.target_run().ok_or("no testbench is open")?;
    let run_ref = (run.0.as_str(), run.1.as_str(), run.2.as_str());
    if let Some(ctx) = view.egui.clone() {
        listen(&ctx);
    }
    let p = &mut view.calc;
    if !p.started {
        *p = CalcPanel::new();
        p.started = true;
    }
    let num = |t: &str| -> Result<f64, String> {
        libcdept::calc::parse_spice_number(t).ok_or_else(|| format!("'{t}' is not a number"))
    };
    let rest = |i: usize| args.get(i..).map(|r| r.join(" ")).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("expr") => {
            p.expr = rest(1);
            Ok(())
        }
        Some("point") => {
            p.point = match args.get(1).map(String::as_str) {
                Some("all") | None => None,
                Some(name) => Some(name.to_string()),
            };
            Ok(())
        }
        Some("plot") => {
            p.plot = match args.get(1).map(String::as_str) {
                Some("any") | None => String::new(),
                Some(k) => k.to_string(),
            };
            Ok(())
        }
        Some("markers") => {
            p.markers = matches!(args.get(1).map(String::as_str), Some("on" | "yes" | "true"));
            Ok(())
        }
        Some("layout") => {
            match args.get(1).map(String::as_str) {
                Some("plain") => p.layout = LayoutChoice::Plain,
                Some("bode") => p.layout = LayoutChoice::Bode,
                Some("eye") => {
                    p.layout = LayoutChoice::Eye;
                    p.eye_period = args.get(2).cloned().unwrap_or_default();
                    p.eye_offset = args.get(3).cloned().unwrap_or_default();
                }
                _ => return Err(USAGE.into()),
            }
            Ok(())
        }
        Some(verb @ ("evaluate" | "send" | "send_tab")) => {
            let how = match verb {
                "send" => SendTo::UberWave,
                "send_tab" => SendTo::Tab,
                _ => SendTo::No,
            };
            p.start(project, run_ref, how, None)?;
            p.wait(300)?;
            for r in &p.rows {
                tracing::info!(target: "cdept::script", "calc {}: {}", r.point, r.text.lines().next().unwrap_or(""));
            }
            for n in &p.notes {
                tracing::info!(target: "cdept::script", "calc: {n}");
            }
            Ok(())
        }
        Some("expect") => {
            let (point, text) = (args.get(1).ok_or(USAGE)?, rest(2));
            let hit = p
                .rows
                .iter()
                .any(|r| (point == "*" || &r.point == point) && r.text.contains(&text));
            if hit {
                Ok(())
            } else {
                Err(format!(
                    "no result of '{point}' contains '{text}': {:?}",
                    p.rows
                ))
            }
        }
        Some("expect_value") => {
            let point = args.get(1).ok_or(USAGE)?;
            let (lo, hi) = (
                num(args.get(2).ok_or(USAGE)?)?,
                num(args.get(3).ok_or(USAGE)?)?,
            );
            let row = p
                .rows
                .iter()
                .find(|r| &r.point == point)
                .ok_or_else(|| format!("no result for '{point}': {:?}", p.rows))?;
            match row.value {
                Some(v) if v >= lo && v <= hi => {
                    tracing::info!(target: "cdept::script", "calc {point} = {v}");
                    Ok(())
                }
                other => Err(format!(
                    "{point}: {other:?} is not within {lo}..{hi} ({})",
                    row.text
                )),
            }
        }
        Some("expect_note") => {
            let text = rest(1);
            if p.notes.iter().any(|n| n.contains(&text)) {
                Ok(())
            } else {
                Err(format!("no note contains '{text}': {:?}", p.notes))
            }
        }
        Some("add_output") => {
            p.out_name = args.get(1).cloned().unwrap_or_default();
            p.out_min = args.get(2).cloned().unwrap_or_default();
            p.out_max = args.get(3).cloned().unwrap_or_default();
            let o = p.new_output()?;
            view.add_output(o);
            Ok(())
        }
        Some("cursors") => {
            let (a, b) = (
                num(args.get(1).ok_or(USAGE)?)?,
                num(args.get(2).ok_or(USAGE)?)?,
            );
            if let Some(ctx) = &view.egui {
                listen(ctx);
            }
            let v = wave::embedded::running().ok_or("no embedded UberWave window is open")?;
            let before = CURSOR_GEN.load(Ordering::SeqCst);
            v.set_cursors(Some(a), Some(b))
                .map_err(|e| format!("{e:#}"))?;
            // the listener reports it from the viewer's thread
            let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while CURSOR_GEN.load(Ordering::SeqCst) == before {
                if std::time::Instant::now() > end {
                    return Err("the viewer did not report the cursor move (is the listener on? open the Cursors section)".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let outputs = view
                .form
                .as_ref()
                .map(|f| f.outputs.clone())
                .unwrap_or_default();
            let uses: Vec<_> = outputs
                .into_iter()
                .filter(|(_, e, _)| e.to_lowercase().contains("cursor_"))
                .collect();
            let p = &mut view.calc;
            let point = p.point.clone().or_else(|| {
                wave::points(project, run_ref.0, run_ref.1, run_ref.2)
                    .first()
                    .map(|x| x.point.clone())
            });
            let raw = point.map(|x| {
                libcdept::sim::runs_dir(project, run_ref.0, run_ref.1, run_ref.2)
                    .join(x)
                    .join("out.raw")
            });
            let (ca, cb) = *CURSORS.lock().unwrap_or_else(|e| e.into_inner());
            p.cursor = cursor_view(ca, cb, raw.as_deref(), &uses);
            p.cursor_seen = CURSOR_GEN.load(Ordering::SeqCst);
            for (s, a, b) in &p.cursor.signals {
                tracing::info!(target: "cdept::script", "cursor {s}: A {a:?} B {b:?}");
            }
            for (n, v) in &p.cursor.outputs {
                tracing::info!(target: "cdept::script", "cursor output {n}: {v:?}");
            }
            Ok(())
        }
        Some("follow") => {
            // What the Cursors section does when a cursor moved (in a waveform tab or
            // UberWave): the values and the outputs that use cursor_a / cursor_b, now.
            let outputs = view
                .form
                .as_ref()
                .map(|f| f.outputs.clone())
                .unwrap_or_default();
            let uses: Vec<_> = outputs
                .into_iter()
                .filter(|(_, e, _)| e.to_lowercase().contains("cursor_"))
                .collect();
            let p = &mut view.calc;
            let point = p.point.clone().or_else(|| {
                wave::points(project, run_ref.0, run_ref.1, run_ref.2)
                    .first()
                    .map(|x| x.point.clone())
            });
            let raw = point.map(|x| {
                libcdept::sim::runs_dir(project, run_ref.0, run_ref.1, run_ref.2)
                    .join(x)
                    .join("out.raw")
            });
            let (ca, cb) = *CURSORS.lock().unwrap_or_else(|e| e.into_inner());
            p.cursor = cursor_view(ca, cb, raw.as_deref(), &uses);
            p.cursor_seen = CURSOR_GEN.load(Ordering::SeqCst);
            tracing::info!(target: "cdept::script", "cursors A {ca:?} B {cb:?}");
            for (s, a, b) in &p.cursor.signals {
                tracing::info!(target: "cdept::script", "cursor {s}: A {a:?} B {b:?}");
            }
            for (n, v) in &p.cursor.outputs {
                tracing::info!(target: "cdept::script", "cursor output {n}: {v:?}");
            }
            Ok(())
        }
        Some("expect_cursor_output") => {
            let name = args.get(1).ok_or(USAGE)?;
            let (lo, hi) = (
                num(args.get(2).ok_or(USAGE)?)?,
                num(args.get(3).ok_or(USAGE)?)?,
            );
            match p.cursor.outputs.iter().find(|(n, _)| n == name) {
                Some((_, Ok(v))) if *v >= lo && *v <= hi => Ok(()),
                other => Err(format!(
                    "cursor output {name}: {other:?} not within {lo}..{hi}"
                )),
            }
        }
        Some("expect_cursor_signal") => {
            let name = rest(1);
            if p.cursor
                .signals
                .iter()
                .any(|(s, a, _)| s.contains(&name) && a.is_some())
            {
                Ok(())
            } else {
                Err(format!(
                    "no value under cursor A for '{name}': {:?}",
                    p.cursor.signals
                ))
            }
        }
        Some("functions") => {
            p.show_functions = true;
            p.func_filter = rest(1);
            Ok(())
        }
        Some("signals") => {
            p.show_signals = true;
            p.sig_filter = rest(1);
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_calls_and_signals() {
        assert_eq!(last_word("db20(v").1, "v");
        assert_eq!(last_word("enob(").1, "");
        assert_eq!(last_word("µ+ab").1, "ab");
        assert_eq!(
            enclosing_call("enob(sample(v(\"d\"), 1n), "),
            Some("enob".into())
        );
        assert_eq!(enclosing_call("enob(sample("), Some("sample".into()));
        assert_eq!(enclosing_call("1 + 2"), None);
        assert_eq!(enclosing_call("(((("), None);
        assert_eq!(call_for("v(out)", ""), "v(\"out\")");
        assert_eq!(call_for("i(vdd)", "tran"), "i(\"vdd\", analysis=\"tran\")");
        assert_eq!(call_for("V1#branch", ""), "i(\"V1\")");
        assert_eq!(
            call_for("onoise_spectrum", "noise"),
            "sig(\"onoise_spectrum\", analysis=\"noise\")"
        );
        let mut e = String::from("1 + en");
        insert_call(&mut e, "enob");
        assert_eq!(e, "1 + enob(");
        let mut p = CalcPanel::new();
        p.expr = "max(v(\"out\"))".into();
        p.out_name = "vmax".into();
        p.out_min = "1".into();
        let o = p.new_output().unwrap();
        assert_eq!((o.name.as_str(), o.min, o.max), ("vmax", Some(1.0), None));
        p.out_name = "two words".into();
        assert!(p.new_output().is_err());
        p.out_name = "x".into();
        p.expr = "max(".into();
        assert!(p.new_output().is_err());
        p.layout = LayoutChoice::Eye;
        p.eye_period = "-1".into();
        assert!(p.layout().is_err());
        p.eye_period = "1n".into();
        p.eye_offset = "200p".into();
        assert_eq!(
            p.layout().unwrap(),
            wcalc::Layout::Eye {
                period: 1e-9,
                offset: 2e-10
            }
        );
    }
}
