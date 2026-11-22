//! The documents of the center area: a schematic drawn by xschem or a layout drawn by
//! KLayout. Everything the shell needs of a document (title, menus, closing, saving,
//! what the test driver looks at) goes through [`Document`].

use std::path::Path;

use egui::Ui;

use super::{CloseChoice, EditAction, EditCaps, LayoutView, SchematicView, View};
use crate::state::AppCtx;

// A handful of documents are open at a time; boxing would only add a hop.
#[allow(clippy::large_enum_variant)]
pub enum Document {
    Schematic(SchematicView),
    Layout(LayoutView),
}

impl From<SchematicView> for Document {
    fn from(view: SchematicView) -> Self {
        Document::Schematic(view)
    }
}

impl From<LayoutView> for Document {
    fn from(view: LayoutView) -> Self {
        Document::Layout(view)
    }
}

impl Document {
    pub fn as_schematic(&self) -> Option<&SchematicView> {
        match self {
            Document::Schematic(view) => Some(view),
            Document::Layout(_) => None,
        }
    }

    pub fn as_schematic_mut(&mut self) -> Option<&mut SchematicView> {
        match self {
            Document::Schematic(view) => Some(view),
            Document::Layout(_) => None,
        }
    }

    pub fn as_layout_mut(&mut self) -> Option<&mut LayoutView> {
        match self {
            Document::Layout(view) => Some(view),
            Document::Schematic(_) => None,
        }
    }

    fn view(&self) -> &dyn View {
        match self {
            Document::Schematic(view) => view,
            Document::Layout(view) => view,
        }
    }

    fn view_mut(&mut self) -> &mut dyn View {
        match self {
            Document::Schematic(view) => view,
            Document::Layout(view) => view,
        }
    }

    /// The file the document was opened for.
    pub fn file(&self) -> &Path {
        match self {
            Document::Schematic(view) => view.file(),
            Document::Layout(view) => view.file(),
        }
    }

    /// The helper has changes that are not saved.
    pub fn is_modified(&self) -> bool {
        match self {
            Document::Schematic(view) => view.is_modified(),
            Document::Layout(view) => view.is_modified(),
        }
    }

    /// Ask the helper to save; the answer comes with [`take_saved`](Self::take_saved).
    pub fn save(&mut self) -> u64 {
        match self {
            Document::Schematic(view) => view.save(),
            Document::Layout(view) => view.save(),
        }
    }

    /// The outcome of a [`save`](Self::save), once the helper has answered.
    pub fn take_saved(&mut self, request: u64) -> Option<Result<(), String>> {
        match self {
            Document::Schematic(view) => view.take_reply(request).map(SchematicView::saved),
            Document::Layout(view) => view.take_done(request).map(|r| r.map(|_| ())),
        }
    }

    /// The helper is up and answering.
    pub fn is_running(&self) -> bool {
        match self {
            Document::Schematic(view) => view.status() == libcdept::xschem::Status::Running,
            Document::Layout(view) => view.is_running(),
        }
    }

    /// Why the helper stopped for good, if it did.
    pub fn failure(&self) -> Option<String> {
        match self {
            Document::Schematic(view) => match view.status() {
                libcdept::xschem::Status::Failed(reason) => Some(reason),
                _ => None,
            },
            Document::Layout(view) => view.failure(),
        }
    }

    /// See [`SchematicView::may_close`].
    pub fn may_close(&mut self) -> bool {
        match self {
            Document::Schematic(view) => view.may_close(),
            Document::Layout(view) => view.may_close(),
        }
    }

    pub fn asks_to_close(&self) -> bool {
        match self {
            Document::Schematic(view) => view.asks_to_close(),
            Document::Layout(view) => view.asks_to_close(),
        }
    }

    pub fn is_closing(&self) -> bool {
        match self {
            Document::Schematic(view) => view.is_closing(),
            Document::Layout(view) => view.is_closing(),
        }
    }

    pub fn close_with(&mut self, choice: CloseChoice, ctx: &mut AppCtx) {
        match self {
            Document::Schematic(view) => view.close_with(choice, ctx),
            Document::Layout(view) => view.close_with(choice, ctx),
        }
    }

    /// The screen position (in points) of the middle of frame pixel (`x`, `y`).
    pub fn frame_to_screen(&self, x: f32, y: f32) -> Option<egui::Pos2> {
        match self {
            Document::Schematic(view) => view.frame_to_screen(x, y),
            Document::Layout(view) => view.frame_to_screen(x, y),
        }
    }

    /// Whether a frame of the running helper is on screen.
    pub fn has_frame(&self) -> bool {
        match self {
            Document::Schematic(view) => view.has_frame(),
            Document::Layout(view) => view.has_frame(),
        }
    }

    pub fn frame_size(&self) -> Option<[u32; 2]> {
        match self {
            Document::Schematic(view) => view.frame_size(),
            Document::Layout(view) => view.frame_size(),
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        match self {
            Document::Schematic(view) => view.pixel(x, y),
            Document::Layout(view) => view.pixel(x, y),
        }
    }

    pub fn compare_with_screen(&self, screen: &egui::ColorImage) -> Result<(), String> {
        match self {
            Document::Schematic(view) => view.compare_with_screen(screen),
            Document::Layout(view) => view.compare_with_screen(screen),
        }
    }

    /// A request that the helper answers after everything sent before: the test driver
    /// waits for it to know the helper has caught up.
    pub fn sync_request(&mut self) -> u64 {
        match self {
            Document::Schematic(view) => view.tcl("expr 1"),
            Document::Layout(view) => view.raw_call(
                libcdept::klayout::protocol::method::INFO,
                serde_json::json!({}),
            ),
        }
    }

    /// The answer to a request of [`sync_request`](Self::sync_request) or a helper call
    /// of the driver, as text.
    pub fn take_reply(&mut self, id: u64) -> Option<Result<String, String>> {
        match self {
            Document::Schematic(view) => view.take_reply(id),
            Document::Layout(view) => view.take_reply(id),
        }
    }
}

impl View for Document {
    fn title(&self, ctx: &AppCtx) -> String {
        self.view().title(ctx)
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        // The notice of a file that changed on disk behind the document's back.
        crate::background::stale_ui(self, ui, ctx);
        self.view_mut().ui(ui, ctx);
    }

    fn menus(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.view_mut().menus(ui, ctx);
    }

    fn toolbar(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.view_mut().toolbar(ui, ctx);
    }

    fn file_menu(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.view_mut().file_menu(ui, ctx);
    }

    fn edit_caps(&self) -> EditCaps {
        self.view().edit_caps()
    }

    fn edit(&mut self, action: EditAction, ctx: &mut AppCtx) {
        self.view_mut().edit(action, ctx);
    }
}
