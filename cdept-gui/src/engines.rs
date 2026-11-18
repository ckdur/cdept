//! xschem work that is not an open document: background jobs (hierarchy, netlists) and
//! the picture of a view in the Inspector.
//!
//! Both belong to the open project. They start on first use and go away with the
//! project.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Context, Pos2, Rect, Sense, Ui, Vec2};
use libcdept::xschem::tcl;
use libcdept::xschem::{
    EngineOptions, Hierarchy, NetlistFormat, Pending, Service, Session, SessionOptions, Status,
};
use libcdept::{Library, Project};

use crate::views::Picture;

/// How long a view must stay selected before its picture is asked for, so that moving
/// through a list with the arrow keys does not load every file on the way.
const PREVIEW_DELAY: Duration = Duration::from_millis(150);
/// Height of the preview as a part of its width.
const PREVIEW_ASPECT: f32 = 0.7;

/// A value that counts only once it has stayed the same for a while.
pub struct Debounce<T> {
    value: Option<(T, Instant)>,
    delay: Duration,
}

impl<T: PartialEq> Debounce<T> {
    pub fn new(delay: Duration) -> Self {
        Self { value: None, delay }
    }

    /// Note the value as of `now`. Setting the same value again does not restart the
    /// wait.
    pub fn set(&mut self, value: T, now: Instant) {
        if self.value.as_ref().is_none_or(|(old, _)| *old != value) {
            self.value = Some((value, now));
        }
    }

    /// The value, if it has been the same for the whole delay.
    pub fn settled(&self, now: Instant) -> Option<&T> {
        let (value, since) = self.value.as_ref()?;
        (now.duration_since(*since) >= self.delay).then_some(value)
    }

    /// How long until the value counts; `None` when it does, or when there is none.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        let (_, since) = self.value.as_ref()?;
        let waited = now.duration_since(*since);
        (waited < self.delay).then(|| self.delay - waited)
    }
}

/// What tells two set-ups of xschem apart: the project folder and its library paths.
type Setup = (Option<PathBuf>, Option<String>);

fn setup(options: &EngineOptions) -> Setup {
    (options.cwd.clone(), options.startup.clone())
}

/// A netlist being written.
struct NetlistJob {
    /// `library/cell`, for messages.
    cell: String,
    pending: Pending<PathBuf>,
}

/// The picture of a view, drawn by an xschem of its own that only ever loads files.
struct Preview {
    setup: Setup,
    session: Session,
    picture: Picture,
    /// Frame size last asked for.
    requested: Option<(u32, u32)>,
    wanted: Debounce<PathBuf>,
    /// The file last sent for loading, the helper it was sent to, and the request.
    loading: Option<(PathBuf, u64, u64)>,
    /// The file the picture shows.
    shown: Option<PathBuf>,
    error: Option<String>,
}

impl Preview {
    fn new(options: EngineOptions, ctx: &Context) -> Self {
        let ctx = ctx.clone();
        Self {
            setup: setup(&options),
            session: Session::open(SessionOptions {
                engine: options,
                file: None,
                wake: Arc::new(move || ctx.request_repaint()),
            }),
            picture: Picture::default(),
            requested: None,
            wanted: Debounce::new(PREVIEW_DELAY),
            loading: None,
            shown: None,
            error: None,
        }
    }

    /// Whether the picture on screen is the one of `file`.
    fn shows(&self, file: &Path) -> bool {
        self.shown.as_deref() == Some(file) && self.picture.texture().is_some()
    }
}

/// The xschem engines of the open project besides its documents.
#[derive(Default)]
pub struct Engines {
    service: Option<(Setup, Service)>,
    netlists: Vec<NetlistJob>,
    preview: Option<Preview>,
}

impl Engines {
    /// The job queue for `project`. A change of the project's libraries starts a new
    /// one, because xschem reads its library path at start.
    fn service(&mut self, project: &Project, ctx: &Context) -> &Service {
        let options = EngineOptions::for_project(project);
        let wanted = setup(&options);
        if self
            .service
            .as_ref()
            .is_none_or(|(have, _)| *have != wanted)
        {
            let ctx = ctx.clone();
            let service = Service::new(options, Arc::new(move || ctx.request_repaint()));
            self.service = Some((wanted, service));
        }
        &self.service.as_ref().expect("just set").1
    }

    /// Work out the hierarchy below the schematic `top`, in the background.
    pub fn hierarchy(
        &mut self,
        project: &Project,
        ctx: &Context,
        top: PathBuf,
    ) -> Pending<Hierarchy> {
        self.service(project, ctx)
            .run(move |engine| engine.hierarchy(&top))
    }

    /// Write the netlist of `cell` into its `netlist` view, in the background. The
    /// outcome is fetched with [`finished_netlists`](Self::finished_netlists).
    pub fn netlist(
        &mut self,
        project: &Project,
        ctx: &Context,
        library: &Library,
        cell: &str,
        format: NetlistFormat,
    ) {
        let name = format!("{}/{cell}", library.name);
        let (library, cell) = (library.clone(), cell.to_string());
        let pending = self
            .service(project, ctx)
            .run(move |engine| engine.netlist_view(&library, &cell, format));
        self.netlists.push(NetlistJob {
            cell: name,
            pending,
        });
    }

    /// The netlists finished since the last call: the cell (`library/cell`) and the
    /// file written, or why none was.
    pub fn finished_netlists(&mut self) -> Vec<(String, Result<PathBuf, String>)> {
        let mut done = Vec::new();
        self.netlists.retain(|job| match job.pending.poll() {
            Some(result) => {
                done.push((job.cell.clone(), result));
                false
            }
            None => true,
        });
        done
    }

    /// Whether a netlist is being written.
    pub fn netlisting(&self) -> bool {
        !self.netlists.is_empty()
    }

    /// The project was closed or replaced: end its engines.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether the Inspector's picture shows `file`.
    pub fn preview_shows(&self, file: &Path) -> bool {
        self.preview.as_ref().is_some_and(|p| p.shows(file))
    }

    /// Draw the picture of the schematic or symbol `file`, as wide as the space left.
    ///
    /// The xschem behind it gets no input and no command but "load this file and zoom
    /// to fit", so it cannot change a file.
    pub fn preview(&mut self, ui: &mut Ui, project: &Project, file: &Path) {
        let options = EngineOptions::for_project(project);
        if self
            .preview
            .as_ref()
            .is_none_or(|p| p.setup != setup(&options))
        {
            self.preview = Some(Preview::new(options, ui.ctx()));
        }
        let preview = self.preview.as_mut().expect("just set");
        let now = Instant::now();

        // Whole pixels: the frame is drawn one pixel per screen pixel.
        let scale = ui.ctx().pixels_per_point();
        let width = ((ui.available_width() * scale).floor()).max(64.0);
        let height = (width * PREVIEW_ASPECT).round();
        let size = (width as u32, height as u32);
        if preview.requested != Some(size) {
            preview.requested = Some(size);
            preview.session.resize(size.0, size.1);
        }

        // Nobody draws this xschem's menus and dialogs.
        preview.session.take_updates();
        for reply in preview.session.take_replies() {
            if let Some((file, _, id)) = &preview.loading
                && reply.id == *id
            {
                match reply.result {
                    Ok(_) => {
                        preview.shown = Some(file.clone());
                        preview.error = None;
                    }
                    Err(error) => preview.error = Some(error),
                }
            }
        }

        let status = preview.session.status();
        preview.wanted.set(file.to_path_buf(), now);
        let epoch = preview.session.epoch();
        let sent = preview
            .loading
            .as_ref()
            .is_some_and(|(f, e, _)| f == file && *e == epoch);
        if !sent && status == Status::Running {
            if let Some(file) = preview.wanted.settled(now) {
                // Looking at a file is not opening it: keep it out of xschem's list
                // of recent files.
                let script = format!(
                    "proc update_recent_file {{args}} {{}}; xschem load {}; xschem zoom_full",
                    tcl::quote(&file.to_string_lossy())
                );
                let id = preview.session.tcl(&script);
                preview.loading = Some((file.clone(), epoch, id));
            } else if let Some(wait) = preview.wanted.remaining(now) {
                ui.ctx().request_repaint_after(wait);
            }
        }

        preview
            .picture
            .update(&preview.session, ui.ctx(), "xschem-preview");
        let area = Vec2::new(width, height) / scale;
        let (rect, _) = ui.allocate_exact_size(area, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::BLACK);
        let note = |text: &str| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_gray(150),
            );
        };
        match (&status, preview.picture.texture()) {
            (Status::Failed(reason), _) => note(&format!("No picture: {reason}")),
            (_, Some((texture, pixels))) if preview.shown.as_deref() == Some(file) => {
                // Pixel for pixel, from a corner on a whole pixel.
                let origin = Pos2::new(
                    (rect.left() * scale).ceil() / scale,
                    (rect.top() * scale).ceil() / scale,
                );
                let size = Vec2::new(pixels[0] as f32, pixels[1] as f32) / scale;
                painter.image(
                    texture.id(),
                    Rect::from_min_size(origin, size),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            _ => match &preview.error {
                Some(error) => note(error),
                None => note("…"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_counts_once_it_has_stayed() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut debounce = Debounce::new(Duration::from_millis(150));
        assert_eq!(debounce.settled(at(0)), None);
        assert_eq!(debounce.remaining(at(0)), None);

        debounce.set("a", at(0));
        assert_eq!(debounce.settled(at(100)), None);
        assert_eq!(debounce.remaining(at(100)), Some(Duration::from_millis(50)));
        // The same value again does not start the wait over.
        debounce.set("a", at(100));
        assert_eq!(debounce.settled(at(150)), Some(&"a"));
        assert_eq!(debounce.remaining(at(150)), None);

        // Stepping through values quickly: none of them settles on the way.
        debounce.set("b", at(160));
        debounce.set("c", at(220));
        assert_eq!(debounce.settled(at(300)), None);
        debounce.set("d", at(300));
        assert_eq!(debounce.settled(at(449)), None);
        assert_eq!(debounce.settled(at(450)), Some(&"d"));
    }
}
