//! A Magic layout view (`.mag`) in a tab: Magic itself draws, in the helper process
//! `cdept-magic` (docs/MAGIC.md, "Embedded Magic"), with the same session machinery
//! as an xschem tab. CDePT adds a tool bar, a command line, Magic's messages, a layer
//! list, DRC results, layout-from-schematic and cross-probing with schematic tabs.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::{Color32, Event, EventFilter, Modifiers, Pos2, Rect, Sense, Ui, Vec2};
use libcdept::Project;
use libcdept::magic::embed;
use libcdept::xschem::input::Input;
use libcdept::xschem::protocol::UiUpdate;
use libcdept::xschem::session::Reply;
use libcdept::xschem::{Session, Status, tcl};

use super::schematic::keys;
use super::schematic::mirror::Mirror;
use super::{CloseChoice, EditAction, EditCaps, Picture, View};
use crate::dock::{DocId, DocRef, Tab};
use crate::state::{Action, AppCtx, Selection};

/// Lines of Magic's messages kept for the message pane.
const LOG_LINES: usize = 2000;
/// Replies kept for whoever asked (the test driver).
const KEPT_REPLIES: usize = 64;
/// How often the edit cell's state (name, unsaved changes, selection) is asked for.
const POLL: Duration = Duration::from_millis(400);

/// One DRC error box.
#[derive(Debug, Clone, PartialEq)]
pub struct DrcItem {
    pub why: String,
    /// llx lly urx ury, microns.
    pub area: [f64; 4],
}

#[derive(Default)]
struct Drc {
    asked: Option<u64>,
    style: String,
    items: Vec<DrcItem>,
    /// A run has finished at least once.
    done: bool,
    open: bool,
}

enum Closing {
    Asking,
    Saving(u64),
}

/// A job on a thread (LVS), and what it says when done.
struct Job {
    what: &'static str,
    result: Receiver<Result<String, String>>,
}

/// Steps of "Generate/Update from Schematic" (all in this helper, so the edits are
/// Magic's own and can be undone).
enum Generate {
    /// Saving the layout first, so that the plan is made from the file.
    Saving(u64),
    /// The plan's Tcl was sent: then save and record.
    Applying(u64, Box<libcdept::link::Schematic>),
    Recording(u64, Box<libcdept::link::Schematic>),
}

pub struct MagicView {
    id: DocId,
    doc: DocRef,
    file: PathBuf,
    editable: bool,
    session: Session,
    epoch: u64,
    mirror: Mirror,
    picture: Picture,
    requested: Option<(u32, u32)>,
    buttons: u32,
    inside: bool,
    wheel: Vec2,
    pointer: Option<Pos2>,
    placement: Option<(Pos2, f32)>,
    replies: Vec<Reply>,
    /// Edit cell and whether it has unsaved changes, as last asked.
    cell: String,
    modified: bool,
    state_asked: Option<u64>,
    next_poll: Instant,
    log: VecDeque<String>,
    show_log: bool,
    command: String,
    focus_command: bool,
    history: Vec<String>,
    layers: Vec<String>,
    layers_asked: Option<u64>,
    layer_filter: String,
    show_layers: bool,
    drc: Drc,
    closing: Option<Closing>,
    saving: Option<u64>,
    /// Files this tab's last save wrote (for the "changed on disk" watch).
    written: Vec<PathBuf>,
    job: Option<Job>,
    generate: Option<Generate>,
    /// Instances selected in Magic, as last read; for cross-probing.
    selected: Vec<String>,
    probe_out: Option<Vec<String>>,
    /// Names last selected because a schematic asked (not sent back).
    echo: Option<Vec<String>>,
    /// Problems of the references of the file (missing cells), shown over the drawing.
    missing: Vec<String>,
    /// Commands typed by the user whose answers go to the messages.
    pending_commands: Vec<u64>,
}

impl MagicView {
    /// Open `file` (the view `doc` of `project`) in a Magic of its own.
    pub fn open(
        id: DocId,
        doc: DocRef,
        file: PathBuf,
        editable: bool,
        project: &Project,
        ctx: &egui::Context,
    ) -> Self {
        let repaint = ctx.clone();
        let session = Session::open(embed::session_options(
            project,
            &file,
            Arc::new(move || repaint.request_repaint()),
        ));
        let missing = libcdept::magic::refs::resolve_file(project, &file)
            .map(|r| {
                r.into_iter()
                    .filter(|r| r.target == libcdept::magic::refs::Target::Missing)
                    .map(|r| format!("{} ({}) cannot be found", r.placed.cell, r.placed.instance))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id,
            doc,
            file,
            editable,
            session,
            epoch: 0,
            mirror: Mirror::default(),
            picture: Picture::default(),
            requested: None,
            buttons: 0,
            inside: false,
            wheel: Vec2::ZERO,
            pointer: None,
            placement: None,
            replies: Vec::new(),
            cell: String::new(),
            modified: false,
            state_asked: None,
            next_poll: Instant::now(),
            log: VecDeque::new(),
            show_log: true,
            command: String::new(),
            focus_command: false,
            history: Vec::new(),
            layers: Vec::new(),
            layers_asked: None,
            layer_filter: String::new(),
            show_layers: false,
            drc: Drc::default(),
            closing: None,
            saving: None,
            written: Vec::new(),
            job: None,
            generate: None,
            selected: Vec::new(),
            probe_out: None,
            echo: None,
            missing,
            pending_commands: Vec::new(),
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn is_running(&self) -> bool {
        self.session.status() == Status::Running
    }

    pub fn failure(&self) -> Option<String> {
        match self.session.status() {
            Status::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Run Tcl in Magic; the answer comes with [`take_reply`](Self::take_reply).
    pub fn tcl(&self, script: &str) -> u64 {
        self.session.tcl(script)
    }

    pub fn take_reply(&mut self, id: u64) -> Option<Result<String, String>> {
        let at = self.replies.iter().position(|r| r.id == id)?;
        Some(self.replies.remove(at).result)
    }

    /// Save the edit cell (and the modified cells of its folder). The answer is the
    /// list of cells still modified: empty when everything was written.
    pub fn save(&mut self) -> u64 {
        let id = self.session.tcl(embed::SAVE_SCRIPT);
        self.saving = Some(id);
        id
    }

    /// What the answer to [`save`](Self::save) means.
    pub fn saved(answer: Result<String, String>) -> Result<(), String> {
        match answer {
            Ok(left) if left.trim().is_empty() => Ok(()),
            Ok(left) => Err(format!("not saved: {left}")),
            Err(e) => Err(e),
        }
    }

    pub fn may_close(&mut self) -> bool {
        if !self.modified || !self.is_running() {
            return true;
        }
        if self.closing.is_none() {
            self.closing = Some(Closing::Asking);
        }
        false
    }

    pub fn asks_to_close(&self) -> bool {
        matches!(self.closing, Some(Closing::Asking))
    }

    pub fn is_closing(&self) -> bool {
        self.closing.is_some()
    }

    pub fn close_with(&mut self, choice: CloseChoice, ctx: &mut AppCtx) {
        match choice {
            CloseChoice::Save => {
                let id = self.save();
                self.closing = Some(Closing::Saving(id));
            }
            CloseChoice::Discard => {
                self.closing = None;
                ctx.request(Action::CloseTab(Tab::Document(self.id)));
            }
            CloseChoice::Cancel => self.closing = None,
        }
    }

    pub fn frame_to_screen(&self, x: f32, y: f32) -> Option<Pos2> {
        let (origin, scale) = self.placement?;
        Some(origin + Vec2::new(x + 0.5, y + 0.5) / scale)
    }

    pub fn has_frame(&self) -> bool {
        self.is_running() && self.picture.texture().is_some()
    }

    pub fn frame_size(&self) -> Option<[u32; 2]> {
        self.picture.texture().map(|(_, size)| size)
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        self.session
            .frame(|f| {
                let mut out = Vec::new();
                let r = f.copy(libcdept::xschem::frame::Rect::new(x, y, 1, 1), &mut out);
                (!r.is_empty()).then(|| [out[0], out[1], out[2], out[3]])
            })
            .flatten()
    }

    /// Compare the frame with what is on screen, pixel for pixel (test driver).
    pub fn compare_with_screen(&self, screen: &egui::ColorImage) -> Result<(), String> {
        let (origin, scale) = self.placement.ok_or("the layout is not on screen")?;
        let (left, top) = (
            (origin.x * scale).round() as usize,
            (origin.y * scale).round() as usize,
        );
        let mut pixels = Vec::new();
        let size = self
            .session
            .frame(|frame| {
                frame.copy(frame.full(), &mut pixels);
                [frame.width() as usize, frame.height() as usize]
            })
            .ok_or("there is no frame yet")?;
        if left + size[0] > screen.size[0] || top + size[1] > screen.size[1] {
            return Err(format!(
                "the frame ({}x{} at {left},{top}) does not fit the window ({}x{})",
                size[0], size[1], screen.size[0], screen.size[1]
            ));
        }
        let mut different = 0;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let frame = &pixels[(y * size[0] + x) * 4..][..3];
                let shown = screen.pixels[(top + y) * screen.size[0] + left + x].to_array();
                if frame.iter().zip(shown).any(|(a, b)| a.abs_diff(b) > 2) {
                    different += 1;
                }
            }
        }
        if different == 0 {
            Ok(())
        } else {
            Err(format!(
                "{different} of {} pixels on screen differ from the frame",
                size[0] * size[1]
            ))
        }
    }

    /// The files the tab shows (for the "changed on disk" watch).
    pub fn disk_files(&self) -> Vec<PathBuf> {
        vec![self.file.clone()]
    }

    /// Files the last save wrote; handed out once.
    pub fn disk_written(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.written)
    }

    pub fn disk_busy(&self) -> bool {
        self.saving.is_some() || self.generate.is_some()
    }

    /// Open the file again as it is on disk.
    pub fn disk_reload(&mut self) {
        let file = tcl::quote(&self.file.to_string_lossy());
        self.session.tcl(&format!("cdept::magic::reload {file}"));
    }

    pub fn start_drc(&mut self) {
        self.drc.open = true;
        if self.drc.asked.is_none() {
            self.drc.asked = Some(self.session.tcl("cdept::magic::drc"));
        }
    }

    /// Box and zoom on DRC result `index`.
    pub fn show_drc(&mut self, index: usize) {
        if let Some(item) = self.drc.items.get(index) {
            let a = item.area;
            self.session.tcl(&format!(
                "cdept::magic::show_area {{{} {} {} {}}}",
                a[0], a[1], a[2], a[3]
            ));
        }
    }

    /// Run one Magic command typed by the user (the command line).
    pub fn run_command(&mut self, command: &str) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }
        self.log_line(&format!("% {command}"));
        self.history.retain(|c| c != command);
        self.history.push(command.to_string());
        if self.history.len() > 200 {
            self.history.remove(0);
        }
        // Errors come back as the reply: shown with the messages.
        let id = self.session.tcl(command);
        self.pending_commands.push(id);
    }

    fn log_line(&mut self, text: &str) {
        for line in text.lines() {
            self.log.push_back(line.to_string());
        }
        while self.log.len() > LOG_LINES {
            self.log.pop_front();
        }
    }

    // -- cross-probing -------------------------------------------------------------

    /// The schematic this layout is linked with: its sidecar's `source`, else the
    /// cell's own schematic.
    pub fn link_schematic(&self, project: Option<&Project>) -> Option<PathBuf> {
        let project = project?;
        libcdept::magic::link::source_of(project, &self.doc.library, &self.doc.cell, &self.file)
            .ok()
            .map(|(_, f)| f)
    }

    pub fn take_probe_out(&mut self) -> Option<Vec<String>> {
        self.probe_out.take()
    }

    /// Select these instances (names from the schematic).
    pub fn probe_in(&mut self, names: &[String]) {
        if names == self.selected.as_slice() || !self.is_running() {
            return;
        }
        let list: Vec<String> = names.iter().map(|n| tcl::quote(n)).collect();
        self.session.tcl(&format!(
            "cdept::magic::select_instances [list {}]",
            list.join(" ")
        ));
        self.echo = Some(names.to_vec());
    }

    // -- generate from schematic ------------------------------------------------------

    /// Generate or update the layout from its schematic (docs/MAGIC.md).
    pub fn request_generate(&mut self) {
        if self.generate.is_some() || !self.is_running() || !self.editable {
            return;
        }
        self.log_line("Generate from schematic: saving the layout first");
        self.generate = Some(Generate::Saving(self.save()));
    }

    pub fn generating(&self) -> bool {
        self.generate.is_some()
    }

    fn step_generate(&mut self, project: Option<&Project>, ctx: &mut AppCtx) {
        let Some(step) = self.generate.take() else {
            return;
        };
        let Some(project) = project else {
            return;
        };
        self.generate = match step {
            Generate::Saving(id) => match self.take_reply(id) {
                None => Some(Generate::Saving(id)),
                Some(answer) => match Self::saved(answer) {
                    Err(e) => {
                        ctx.error(format!("Generate from schematic: {e}"));
                        None
                    }
                    Ok(()) => self.plan_generate(project, ctx),
                },
            },
            Generate::Applying(id, schematic) => match self.take_reply(id) {
                None => Some(Generate::Applying(id, schematic)),
                Some(Err(e)) => {
                    ctx.error(format!("Generate from schematic: {e}"));
                    None
                }
                Some(Ok(_)) => {
                    let id = self.session.tcl(embed::SAVE_SCRIPT);
                    Some(Generate::Recording(id, schematic))
                }
            },
            Generate::Recording(id, schematic) => match self.take_reply(id) {
                None => Some(Generate::Recording(id, schematic)),
                Some(answer) => {
                    if let Err(e) = Self::saved(answer) {
                        ctx.error(format!("Generate from schematic: {e}"));
                    } else if let Err(e) =
                        libcdept::magic::link::record(project, &self.file, &schematic)
                    {
                        ctx.error(format!("Generate from schematic: {e:#}"));
                    } else {
                        self.log_line("Generate from schematic: done");
                        self.written.push(self.file.clone());
                        ctx.request(Action::ReloadProject);
                    }
                    None
                }
            },
        };
    }

    fn plan_generate(&mut self, project: &Project, ctx: &mut AppCtx) -> Option<Generate> {
        let made = (|| -> eyre::Result<_> {
            let (source, sch) = libcdept::magic::link::source_of(
                project,
                &self.doc.library,
                &self.doc.cell,
                &self.file,
            )?;
            let schematic = libcdept::magic::link::read_schematic(project, source, &sch)?;
            let placed = libcdept::magic::link::placed(project, &self.file)?;
            let plan = libcdept::magic::link::plan(&schematic, &placed);
            Ok((schematic, placed, plan))
        })();
        match made {
            Err(e) => {
                ctx.error(format!("Generate from schematic: {e:#}"));
                None
            }
            Ok((schematic, placed, plan)) => {
                for line in plan.lines() {
                    self.log_line(&format!("  {line}"));
                }
                if plan.actions.is_empty() {
                    self.log_line("Generate from schematic: in sync");
                    let _ = libcdept::magic::link::record(project, &self.file, &schematic);
                    return None;
                }
                let top = embed::cell_of(&self.file);
                let script = libcdept::magic::link::apply_script(&top, &plan, &placed, false);
                let id = self.session.tcl(&script);
                Some(Generate::Applying(id, Box::new(schematic)))
            }
        }
    }

    // -- jobs ---------------------------------------------------------------------------

    /// LVS against the schematic (saves first; netgen on a thread).
    pub fn start_lvs(&mut self, project: Option<&Project>) {
        let Some(project) = project.cloned() else {
            return;
        };
        if self.job.is_some() {
            return;
        }
        let (library, cell) = (self.doc.library.clone(), self.doc.cell.clone());
        let (tx, rx) = channel();
        self.log_line("LVS: extracting with Magic, comparing with netgen…");
        std::thread::spawn(move || {
            let outcome = (|| -> eyre::Result<String> {
                let magic = libcdept::magic::Magic::for_project(&project)?;
                let r = libcdept::magic::flows::lvs(&magic, &project, &library, &cell)?;
                let mut text = format!("LVS {library}/{cell}: {}", r.report.result);
                if let Some(log) = &r.report.log {
                    text += &format!("\n  log: {}", log.display());
                }
                for n in r.notes {
                    text += &format!("\n  note: {n}");
                }
                Ok(text)
            })();
            let _ = tx.send(outcome.map_err(|e| format!("{e:#}")));
        });
        self.job = Some(Job {
            what: "LVS",
            result: rx,
        });
    }

    fn poll_job(&mut self, ctx: &mut AppCtx) {
        let Some(job) = &self.job else {
            return;
        };
        match job.result.try_recv() {
            Ok(Ok(text)) => {
                self.log_line(&text);
                tracing::info!(target: "magic", "{}", text.lines().next().unwrap_or(""));
                self.job = None;
            }
            Ok(Err(e)) => {
                ctx.error(format!("{}: {e}", job.what));
                self.job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
        }
    }
}

impl MagicView {
    /// Take in what the helper reported since the last frame.
    fn sync(&mut self, ctx: &egui::Context) {
        let epoch = self.session.epoch();
        if epoch != self.epoch {
            // A new helper: what came from the old one is gone with it.
            self.epoch = epoch;
            self.mirror = Mirror::default();
            self.buttons = 0;
            self.inside = false;
            self.state_asked = None;
            self.layers_asked = None;
            self.drc.asked = None;
            self.saving = None;
            self.generate = None;
            self.pending_commands.clear();
            if epoch > 1 {
                self.log_line("Magic was started again: unsaved changes are lost");
                self.modified = false;
            }
        }
        for update in self.session.take_updates() {
            match update {
                UiUpdate::Log { text } => self.log_line(&text),
                other => self.mirror.apply(other, ctx),
            }
        }
        for update in self.session.take_panes() {
            self.mirror.apply_pane(update, ctx);
        }
        self.replies.extend(self.session.take_replies());
        // Answers to commands typed by the user.
        let mut answers = Vec::new();
        self.pending_commands.retain(|id| {
            let found = self.replies.iter().position(|r| r.id == *id);
            if let Some(at) = found {
                answers.push(self.replies.remove(at).result);
                false
            } else {
                true
            }
        });
        for a in answers {
            match a {
                Ok(text) if !text.trim().is_empty() => self.log_line(&text),
                Ok(_) => {}
                Err(e) => self.log_line(&format!("error: {e}")),
            }
        }
        if !self.is_running() {
            return;
        }
        // The edit cell, unsaved changes and the selection, a few times a second.
        if let Some(id) = self.state_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.state_asked = None;
            if let Ok(text) = answer {
                let words = tcl::parse_list(&text).unwrap_or_default();
                if let [cell, modified, selection, ..] = words.as_slice() {
                    self.cell = cell.clone();
                    // A view that cannot be saved (a GDS read into Magic) has nothing to lose.
                    self.modified = self.editable && modified != "0" && !modified.is_empty();
                    let names: Vec<String> = tcl::parse_list(selection)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|pair| tcl::parse_list(pair).ok()?.into_iter().next())
                        .collect();
                    if names != self.selected {
                        self.selected = names.clone();
                        let echoed = self.echo.take().is_some_and(|e| e == names);
                        if !echoed {
                            self.probe_out = Some(names);
                        }
                    }
                }
            }
        }
        if self.state_asked.is_none()
            && Instant::now() >= self.next_poll
            && !self.mirror.has_modal()
        {
            self.next_poll = Instant::now() + POLL;
            self.state_asked = Some(self.session.tcl(
                "list [lindex [cdept::magic::state] 0] [llength [cdept::magic::modified]] \
                 [cdept::magic::selection]",
            ));
        }
        ctx.request_repaint_after(POLL);
        if self.layers.is_empty() && self.layers_asked.is_none() {
            self.layers_asked = Some(self.session.tcl("cdept::magic::layers"));
        }
        if let Some(id) = self.layers_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.layers = answer
                .ok()
                .and_then(|t| tcl::parse_list(&t).ok())
                .unwrap_or_default();
        }
        if let Some(id) = self.drc.asked
            && let Some(answer) = self.take_reply(id)
        {
            self.drc.asked = None;
            self.drc.done = true;
            match answer {
                Ok(text) => {
                    let words = tcl::parse_list(&text).unwrap_or_default();
                    self.drc.style = words.first().cloned().unwrap_or_default();
                    self.drc.items = words
                        .get(1)
                        .and_then(|l| tcl::parse_list(l).ok())
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|item| {
                            let parts = tcl::parse_list(item).ok()?;
                            let why = parts.first()?.clone();
                            let nums: Vec<f64> = tcl::parse_list(parts.get(1)?)
                                .ok()?
                                .iter()
                                .filter_map(|n| n.parse().ok())
                                .collect();
                            let area: [f64; 4] = nums.try_into().ok()?;
                            Some(DrcItem { why, area })
                        })
                        .collect();
                    self.log_line(&format!(
                        "DRC ({}): {} error box(es)",
                        self.drc.style,
                        self.drc.items.len()
                    ));
                }
                Err(e) => self.log_line(&format!("DRC: {e}")),
            }
        }
        let excess = self.replies.len().saturating_sub(KEPT_REPLIES);
        self.replies.drain(..excess);
    }

    /// The save, when its answer came: what it wrote, the use paths made relative and
    /// the sidecar (docs/MAGIC.md).
    fn finish_save(&mut self, ctx: &mut AppCtx) {
        let Some(id) = self.saving else {
            return;
        };
        let Some(pos) = self.replies.iter().position(|r| r.id == id) else {
            return;
        };
        // Leave the reply for whoever else waits on it (closing, generate, the driver).
        let answer = self.replies[pos].result.clone();
        self.saving = None;
        match Self::saved(answer) {
            Ok(()) => {
                self.modified = false;
                if let Some(project) = ctx.project.as_ref() {
                    match libcdept::magic::refs::normalize(project, &self.file) {
                        Ok(n) => {
                            if n.paths > 0 {
                                self.log_line(&format!(
                                    "{} use path(s) written relative to this folder",
                                    n.paths
                                ));
                            }
                            self.missing = n
                                .missing
                                .iter()
                                .map(|(i, c)| format!("{c} ({i}) cannot be found"))
                                .collect();
                        }
                        Err(e) => self.log_line(&format!("after the save: {e:#}")),
                    }
                }
                self.written.push(self.file.clone());
                self.written
                    .push(libcdept::layout::meta::sidecar_path(&self.file));
                self.log_line(&format!("saved {}", self.file.display()));
            }
            Err(e) => self.log_line(&format!("save: {e}")),
        }
    }

    /// The question about unsaved changes, and the wait for the save.
    fn closing_ui(&mut self, ui: &Ui, ctx: &mut AppCtx) {
        match self.closing {
            None => {}
            Some(Closing::Asking) => {
                let mut choice = None;
                let modal = egui::Modal::new(egui::Id::new(("magic-close", self.id)));
                let response = modal.show(ui.ctx(), |ui| {
                    ui.set_max_width(420.0);
                    ui.heading("Unsaved changes");
                    ui.label(format!(
                        "{} has changes that are not saved.",
                        self.doc.title()
                    ));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (label, answer) in [
                            ("Save", CloseChoice::Save),
                            ("Discard", CloseChoice::Discard),
                            ("Cancel", CloseChoice::Cancel),
                        ] {
                            if ui.button(label).clicked() {
                                choice = Some(answer);
                            }
                        }
                    });
                });
                if response.should_close() {
                    choice.get_or_insert(CloseChoice::Cancel);
                }
                if let Some(choice) = choice {
                    self.close_with(choice, ctx);
                }
            }
            Some(Closing::Saving(id)) => match self.take_reply(id).map(Self::saved) {
                None if self.is_running() => {}
                None => self.closing = None,
                Some(Ok(())) => {
                    self.closing = None;
                    ctx.request(Action::CloseTab(Tab::Document(self.id)));
                }
                Some(Err(error)) => {
                    self.closing = None;
                    ctx.error(format!("{} was not saved: {error}", self.doc.title()));
                }
            },
        }
    }

    /// Pass the mouse and the keyboard on to Magic.
    fn input(&mut self, events: &[Event], modifiers: Modifiers, over: bool, keys: bool) {
        let Some((origin, scale)) = self.placement else {
            return;
        };
        let to_frame = |pos: Pos2| {
            let p = (pos - origin) * scale;
            (p.x.floor() as i32, p.y.floor() as i32)
        };
        for event in events {
            match event {
                Event::PointerMoved(pos) => {
                    self.pointer = Some(*pos);
                    let (x, y) = to_frame(*pos);
                    let state = keys::state_mask(modifiers, self.buttons);
                    if over || self.buttons != 0 {
                        self.inside = true;
                        self.session.input(Input::Motion { x, y, state });
                    } else {
                        self.inside = false;
                    }
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let Some(number) = keys::button_number(*button) else {
                        continue;
                    };
                    let (x, y) = to_frame(*pos);
                    let state = keys::state_mask(*modifiers, self.buttons);
                    let mask = keys::button_mask(number);
                    if *pressed && over {
                        self.session.input(Input::ButtonPress {
                            button: number,
                            x,
                            y,
                            state,
                        });
                        self.buttons |= mask;
                    } else if !*pressed && self.buttons & mask != 0 {
                        self.session.input(Input::ButtonRelease {
                            button: number,
                            x,
                            y,
                            state,
                        });
                        self.buttons &= !mask;
                        self.next_poll = Instant::now();
                    }
                }
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if over => {
                    self.wheel += keys::wheel_notches(*unit, *delta);
                    let (x, y) = self.pointer.map_or((0, 0), to_frame);
                    let state = keys::state_mask(*modifiers, self.buttons);
                    for button in keys::take_wheel_buttons(&mut self.wheel) {
                        self.session.input(Input::ButtonPress {
                            button,
                            x,
                            y,
                            state,
                        });
                        self.session.input(Input::ButtonRelease {
                            button,
                            x,
                            y,
                            state,
                        });
                    }
                }
                Event::Text(text) if keys => {
                    for c in text.chars() {
                        // `:` starts a command, as in Magic: here it is CDePT's line.
                        if c == ':' {
                            self.focus_command = true;
                            continue;
                        }
                        self.key(keys::char_symbol(c), modifiers, true);
                        self.key(keys::char_symbol(c), modifiers, false);
                    }
                }
                Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } if keys => {
                    // Ctrl+S is CDePT's save (as in layout tabs), not a Magic macro.
                    if *key == egui::Key::S
                        && (modifiers.ctrl || modifiers.command)
                        && !modifiers.shift
                    {
                        if *pressed && self.editable {
                            self.save();
                        }
                        continue;
                    }
                    if let Some(action) = keys::edit_shortcut(*key, *modifiers) {
                        if *pressed {
                            self.undo_redo(action);
                        }
                        continue;
                    }
                    if keys::types_text(*key, *modifiers) {
                        continue;
                    }
                    if let Some(symbol) = keys::key_symbol(*key, *modifiers) {
                        self.key(symbol, *modifiers, *pressed);
                    }
                }
                _ => {}
            }
        }
    }

    fn key(&mut self, keysym: u32, modifiers: Modifiers, pressed: bool) {
        let (origin, scale) = self.placement.unwrap_or((Pos2::ZERO, 1.0));
        let p = self
            .pointer
            .map_or(Vec2::ZERO, |pos| (pos - origin) * scale);
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        let state = keys::state_mask(modifiers, self.buttons);
        self.next_poll = Instant::now();
        self.session.input(if pressed {
            Input::KeyPress {
                keysym,
                x,
                y,
                state,
            }
        } else {
            Input::KeyRelease {
                keysym,
                x,
                y,
                state,
            }
        });
    }

    fn undo_redo(&mut self, action: EditAction) {
        let command = match action {
            EditAction::Undo => "undo",
            EditAction::Redo => "redo",
            _ => return,
        };
        if self.is_running() && !self.mirror.has_modal() && self.editable {
            self.session.tcl(command);
            self.next_poll = Instant::now();
        }
    }

    /// A Magic command from a button or menu: logged, its answer shown.
    fn command(&mut self, command: &str) {
        if self.is_running() {
            self.run_command(command);
            self.next_poll = Instant::now();
        }
    }

    fn status_overlay(&mut self, ui: &mut Ui, rect: Rect) {
        let status = self.session.status();
        let (text, failed) = match &status {
            Status::Running | Status::Closed => {
                if !self.missing.is_empty() {
                    let painter = ui.painter_at(rect);
                    let mut at = rect.left_top() + Vec2::splat(8.0);
                    for m in self.missing.iter().take(8) {
                        let galley = painter.layout_no_wrap(
                            format!("✖ {m}"),
                            egui::TextStyle::Body.resolve(ui.style()),
                            Color32::from_rgb(255, 80, 80),
                        );
                        let back = Rect::from_min_size(at, galley.size()).expand(3.0);
                        painter.rect_filled(back, 3.0, Color32::from_black_alpha(190));
                        let h = galley.size().y + 6.0;
                        painter.galley(at, galley, Color32::from_rgb(255, 80, 80));
                        at.y += h;
                    }
                }
                return;
            }
            Status::Starting => ("Starting Magic…".to_string(), false),
            Status::Restarting(reason) => (format!("{reason}. Starting it again…"), false),
            Status::Failed(reason) => (reason.clone(), true),
        };
        if !failed {
            let at = rect.left_top() + Vec2::splat(8.0);
            let painter = ui.painter_at(rect);
            let galley = painter.layout_no_wrap(
                text,
                egui::TextStyle::Body.resolve(ui.style()),
                Color32::from_gray(220),
            );
            painter.rect_filled(
                Rect::from_min_size(at, galley.size()).expand(4.0),
                4.0,
                Color32::from_black_alpha(170),
            );
            painter.galley(at, galley, Color32::from_gray(220));
            return;
        }
        let panel = Rect::from_center_size(rect.center(), rect.size().min(Vec2::new(560.0, 220.0)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.strong("Magic stopped");
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            ui.label(text);
                        });
                    ui.add_space(4.0);
                    if ui.button("Restart").clicked() {
                        self.session.restart();
                    }
                });
            });
        });
    }

    /// The command line and Magic's messages, at the bottom of the tab.
    fn console_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(":");
            let edit = egui::TextEdit::singleline(&mut self.command)
                .hint_text("Magic command (Enter runs it; ':' in the drawing comes here)")
                .desired_width(ui.available_width() - 90.0)
                .id(egui::Id::new(("magic-command", self.id)));
            let response = ui.add(edit);
            if self.focus_command {
                self.focus_command = false;
                response.request_focus();
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let command = std::mem::take(&mut self.command);
                self.run_command(&command);
                response.request_focus();
            }
            if response.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::ArrowUp))
                && let Some(last) = self.history.last()
            {
                self.command = last.clone();
            }
            ui.toggle_value(&mut self.show_log, "Messages");
        });
        if self.show_log {
            let height = 110.0;
            egui::ScrollArea::vertical()
                .id_salt(("magic-log", self.id))
                .max_height(height)
                .min_scrolled_height(height)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &self.log {
                        ui.monospace(line);
                    }
                });
        }
    }

    /// The layer list (paint and erase in the box) and the DRC results, on the right.
    fn side_ui(&mut self, ui: &mut Ui) {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if self.show_layers {
            ui.strong("Layers");
            ui.add(
                egui::TextEdit::singleline(&mut self.layer_filter)
                    .hint_text("filter")
                    .desired_width(150.0),
            );
            let filter = self.layer_filter.to_lowercase();
            let layers: Vec<String> = self
                .layers
                .iter()
                .filter(|l| filter.is_empty() || l.to_lowercase().contains(&filter))
                .cloned()
                .collect();
            let mut todo = None;
            egui::ScrollArea::vertical()
                .id_salt(("magic-layers", self.id))
                .max_height(260.0)
                .show(ui, |ui| {
                    for layer in &layers {
                        ui.horizontal(|ui| {
                            ui.label(layer);
                            if ui
                                .small_button("paint")
                                .on_hover_text("Paint the box with this layer")
                                .clicked()
                            {
                                todo = Some(format!("paint {layer}"));
                            }
                            if ui
                                .small_button("erase")
                                .on_hover_text("Erase this layer in the box")
                                .clicked()
                            {
                                todo = Some(format!("erase {layer}"));
                            }
                        });
                    }
                });
            if let Some(c) = todo
                && self.editable
            {
                self.command(&c);
            }
            ui.separator();
        }
        if self.drc.open {
            ui.horizontal(|ui| {
                ui.strong("DRC");
                if self.drc.asked.is_some() {
                    ui.spinner();
                } else if ui.small_button("⟳").on_hover_text("Check again").clicked() {
                    self.start_drc();
                }
                if ui.small_button("✖").clicked() {
                    self.drc.open = false;
                }
            });
            if self.drc.done {
                ui.weak(format!(
                    "{} error box(es), {}",
                    self.drc.items.len(),
                    self.drc.style
                ));
            }
            let mut show = None;
            egui::ScrollArea::vertical()
                .id_salt(("magic-drc", self.id))
                .show(ui, |ui| {
                    for (i, item) in self.drc.items.iter().enumerate().take(2000) {
                        let a = item.area;
                        let text = format!("{}  ({:.3}, {:.3})", item.why, a[0], a[1]);
                        if ui
                            .selectable_label(false, text)
                            .on_hover_text("Box it and zoom to it")
                            .clicked()
                        {
                            show = Some(i);
                        }
                    }
                });
            if let Some(i) = show {
                self.show_drc(i);
            }
        }
    }
}

/// Magic's tool bar commands: (label, hover, command).
const TOOLS: [(&str, &str, &str); 9] = [
    ("Fit", "Zoom to the whole cell (v)", "view"),
    ("Zoom In", "Zoom in (z)", "zoom 0.5"),
    ("Zoom Out", "Zoom out (Z)", "zoom 2"),
    (
        "Box",
        "Box tool: left/right buttons place the box corners",
        "tool box",
    ),
    ("Wire", "Wiring tool", "tool wiring"),
    ("Select", "Select what is under the box (s)", "select"),
    ("Delete", "Delete the selection (d)", "delete"),
    (
        "Expand",
        "Show the inside of the selected cells (x)",
        "expand",
    ),
    ("Grid", "Grid on or off (g)", "grid"),
];

impl View for MagicView {
    fn title(&self, _ctx: &AppCtx) -> String {
        let mut title = self.doc.title();
        if !self.cell.is_empty() && self.cell != embed::cell_of(&self.file) {
            title = format!("{} (in {title})", self.cell);
        }
        if self.modified {
            title.push('*');
        }
        title
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.sync(ui.ctx());
        self.finish_save(ctx);
        self.poll_job(ctx);
        let project = ctx.project.clone();
        self.step_generate(project.as_ref(), ctx);
        if self.session.status() == Status::Closed {
            ctx.request(Action::CloseTab(Tab::Document(self.id)));
        }
        // Bottom: command line and messages. Right: layers and DRC.
        egui::Panel::bottom(egui::Id::new(("magic-console", self.id)))
            .resizable(false)
            .show(ui, |ui| self.console_ui(ui));
        if self.show_layers || self.drc.open {
            egui::Panel::right(egui::Id::new(("magic-side", self.id)))
                .resizable(true)
                .default_size(300.0)
                .size_range(180.0..=700.0)
                .show(ui, |ui| self.side_ui(ui));
        }

        let area = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(area, Sense::click_and_drag());
        let scale = ui.ctx().pixels_per_point();
        let left = (area.left() * scale).ceil();
        let top = (area.top() * scale).ceil();
        let width = ((area.right() * scale).floor() - left).max(1.0) as u32;
        let height = ((area.bottom() * scale).floor() - top).max(1.0) as u32;
        let origin = Pos2::new(left, top) / scale;
        self.placement = Some((origin, scale));
        if self.requested != Some((width, height)) {
            self.requested = Some((width, height));
            self.session.resize(width, height);
        }
        let name = format!("magic-frame-{}", self.id.0);
        self.picture.update(&self.session, ui.ctx(), &name);
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, Color32::BLACK);
        if let Some((texture, pixels)) = self.picture.texture() {
            let size = Vec2::new(pixels[0] as f32, pixels[1] as f32) / scale;
            painter.image(
                texture.id(),
                Rect::from_min_size(origin, size),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let modal = self.mirror.has_modal() || self.closing.is_some();
        let running = self.is_running();
        let over = response.contains_pointer() && running && !modal;
        if over {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if ui.memory(|m| m.focused().is_none()) || response.clicked() || response.drag_started()
            {
                response.request_focus();
            }
        }
        let keys = over && response.has_focus();
        if response.has_focus() {
            let filter = EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, filter));
        }
        // Ctrl+S saves the tab that has the keyboard or the pointer, also when the pointer
        // is not over the drawing itself (the key events below need both).
        if running
            && !modal
            && self.editable
            && (response.has_focus() || response.contains_pointer())
            && ui.input_mut(|i| {
                i.consume_shortcut(&egui::KeyboardShortcut::new(
                    Modifiers::COMMAND,
                    egui::Key::S,
                ))
            })
        {
            self.save();
        }
        if running && !modal {
            let (events, modifiers) = ui.input(|i| (i.events.clone(), i.modifiers));
            self.input(&events, modifiers, over, keys);
        }
        if modal {
            painter.rect_filled(area, 0.0, Color32::from_black_alpha(90));
        }
        self.status_overlay(ui, area);
        // Tk dialogs of Magic's Tcl (PDK device dialogs, message boxes).
        let salt = egui::Id::new(("magic", self.id));
        let mut events = self.mirror.popup(ui, salt, self.pointer);
        let (project_ref, engines) = (ctx.project.as_ref(), &mut ctx.engines);
        let mut picture = |ui: &mut Ui, file: &Path| {
            if let Some(project) = project_ref {
                engines.picker_preview(ui, project, file);
            }
        };
        events.extend(
            self.mirror
                .windows(ui.ctx(), salt, area, project_ref, &mut picture),
        );
        for event in events {
            self.session.ui(event);
        }
        self.closing_ui(ui, ctx);
    }

    fn menus(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let ready = self.is_running() && !self.mirror.has_modal();
        ui.menu_button("Magic", |ui| {
            ui.add_enabled_ui(ready, |ui| {
                if ui
                    .add_enabled(self.editable, egui::Button::new("Save"))
                    .clicked()
                {
                    self.save();
                    ui.close();
                }
                ui.separator();
                for (label, command) in [
                    ("Edit Selected Cell (descend)", "edit"),
                    (
                        "Back to the Top Cell",
                        "load [lindex [cellname list topcells] 0]",
                    ),
                    ("Expand All", "expand"),
                    ("Unexpand All", "unexpand"),
                    ("Clear Selection", "select clear"),
                    ("Redraw", "redraw"),
                ] {
                    if ui.button(label).clicked() {
                        self.command(command);
                        ui.close();
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.show_layers, "Layer List");
                if ui.button("DRC (results panel)").clicked() {
                    self.start_drc();
                    ui.close();
                }
                if ui.button("Restart Magic").clicked() {
                    self.session.restart();
                    ui.close();
                }
            });
        });
        ui.menu_button("Cell", |ui| {
            let has_schematic = self.link_schematic(ctx.project.as_ref()).is_some();
            if ui
                .add_enabled(has_schematic, egui::Button::new("Open Schematic"))
                .clicked()
            {
                if let Some(file) = self.link_schematic(ctx.project.as_ref()) {
                    ctx.request(Action::OpenFile(file));
                }
                ui.close();
            }
            if ui
                .add_enabled(
                    has_schematic && ready && self.editable && !self.generating(),
                    egui::Button::new("Generate / Update from Schematic"),
                )
                .on_hover_text(
                    "Place the schematic's devices with the PDK's Magic generators, \
                     named as in the schematic; changed ones are made again",
                )
                .clicked()
            {
                self.request_generate();
                ui.close();
            }
            if ui
                .add_enabled(
                    has_schematic && self.job.is_none(),
                    egui::Button::new("LVS against Schematic (netgen)"),
                )
                .on_hover_text("Uses the saved file")
                .clicked()
            {
                self.start_lvs(ctx.project.as_ref());
                ui.close();
            }
            if ui.button("Select in Libraries").clicked() {
                ctx.selection = Selection::view(&self.doc.library, &self.doc.cell, &self.doc.view);
                ctx.request(Action::OpenTab(Tab::Libraries));
                ui.close();
            }
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let ready = self.is_running() && !self.mirror.has_modal();
        ui.add_enabled_ui(ready, |ui| {
            for (label, hover, command) in TOOLS {
                if ui.button(label).on_hover_text(hover).clicked() {
                    self.command(command);
                }
            }
            if ui
                .button("DRC")
                .on_hover_text("Check the cell as it is now")
                .clicked()
            {
                self.start_drc();
            }
            ui.toggle_value(&mut self.show_layers, "Layers");
        });
        if self.job.is_some() || self.generating() {
            ui.spinner();
        }
    }

    fn edit_caps(&self) -> EditCaps {
        let ready = self.is_running() && !self.mirror.has_modal() && self.editable;
        EditCaps {
            undo: ready,
            redo: ready,
            ..EditCaps::default()
        }
    }

    fn edit(&mut self, action: EditAction, _ctx: &mut AppCtx) {
        self.undo_redo(action);
    }

    fn file_menu(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let ready = self.is_running() && !self.mirror.has_modal();
        ui.add_enabled_ui(ready, |ui| {
            if ui
                .button("Export SVG of the window…")
                .on_hover_text("Magic's `plot svg` into <project>/magic/<library>/<cell>/")
                .clicked()
            {
                let file = self
                    .file
                    .with_extension("svg")
                    .to_string_lossy()
                    .into_owned();
                self.command(&format!("plot svg {}", tcl::quote(&file)));
                ui.close();
            }
        });
        ui.separator();
    }
}

/// `magic <verb> …` commands of a test script that are tried again until they hold.
pub(crate) fn patient(args: &[String]) -> bool {
    args.first().is_some_and(|v| v.starts_with("expect_"))
}

/// `magic <verb> …` of a test script (docs/GUI-TESTING.md):
/// `command <magic command…>`, `save`, `drc`, `drc_show <n>`, `generate`,
/// `layers on|off`, `restart`, and the checks `expect_saved`, `expect_modified 0|1`,
/// `expect_drc <n>` (or `>n`), `expect_generated`, `expect_log <text>`,
/// `expect_cell <name>`, `expect_selected <names…>`, `expect_missing <n>`.
pub(crate) fn script(view: &mut MagicView, args: &[String]) -> Result<(), String> {
    let rest = args.get(1..).unwrap_or_default().join(" ");
    let count = |text: &str| -> Result<(bool, usize), String> {
        let (more, n) = match text.trim().strip_prefix('>') {
            Some(n) => (true, n),
            None => (false, text.trim()),
        };
        Ok((
            more,
            n.parse().map_err(|_| format!("'{text}' is not a number"))?,
        ))
    };
    match args.first().map(String::as_str) {
        Some("command") => view.run_command(&rest),
        Some("save") => {
            view.save();
        }
        Some("drc") => {
            view.drc.done = false;
            view.start_drc();
        }
        Some("drc_show") => view.show_drc(count(&rest)?.1),
        Some("generate") => view.request_generate(),
        Some("layers") => view.show_layers = rest.trim() != "off",
        Some("restart") => view.session.restart(),
        Some("expect_saved") => {
            if view.saving.is_some() || view.modified {
                return Err("not saved yet".into());
            }
        }
        Some("expect_modified") => {
            let want = rest.trim() == "1";
            if view.modified != want {
                return Err(format!("modified is {}", view.modified));
            }
        }
        Some("expect_drc") => {
            if !view.drc.done || view.drc.asked.is_some() {
                return Err("the DRC has not finished".into());
            }
            let (more, n) = count(&rest)?;
            let have = view.drc.items.len();
            if (more && have <= n) || (!more && have != n) {
                return Err(format!("{have} DRC error box(es), expected {rest}"));
            }
        }
        Some("expect_generated") => {
            if view.generating() {
                return Err("still generating".into());
            }
        }
        Some("expect_log") => {
            if !view.log.iter().any(|l| l.contains(rest.as_str())) {
                return Err(format!("no message line contains '{rest}'"));
            }
        }
        Some("expect_cell") => {
            if view.cell != rest.trim() {
                return Err(format!("the edit cell is '{}'", view.cell));
            }
        }
        Some("expect_selected") => {
            let want: Vec<String> = args.get(1..).unwrap_or_default().to_vec();
            let mut have = view.selected.clone();
            have.sort();
            let mut want_sorted = want.clone();
            want_sorted.sort();
            if have != want_sorted {
                return Err(format!("selected {have:?}, expected {want:?}"));
            }
        }
        Some("expect_missing") => {
            let n = count(&rest)?.1;
            if view.missing.len() != n {
                return Err(format!(
                    "{} missing cell(s): {:?}",
                    view.missing.len(),
                    view.missing
                ));
            }
        }
        Some("lvs") => return Err("magic lvs needs the project: use the Cell menu".into()),
        other => return Err(format!("unknown magic command {other:?}")),
    }
    Ok(())
}
