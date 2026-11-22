//! Finding cells: the quick-open box ("Go to Cell", Ctrl+P) and the Search panel.
//!
//! Both use `libcdept::search`. The quick-open box searches names while one types and
//! jumps to the cell (Enter) or opens a view of it (Shift+Enter). The Search panel has
//! the filters as widgets and can look inside the files; that runs behind the window.
//!
//! Ctrl+P is xschem's "add input pin", so while a schematic tab is in front the box
//! opens with Ctrl+Shift+F only (which works everywhere).

use std::sync::{Arc, Mutex};

use egui::{Align2, Context, Id, Key, KeyboardShortcut, Modifiers, RichText, ScrollArea, Ui};
use libcdept::search::{self, Hit, Matched, Query, Results};
use libcdept::{Project, ViewKind};

use super::{View, empty_state};
use crate::dock::Tab;
use crate::state::{Action, AppCtx, Selection};

/// Opens the quick-open box everywhere.
pub(crate) const FIND: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);
/// Opens it unless a schematic tab is in front (xschem: add input pin).
pub(crate) const GO_TO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::P);

/// The most hits the quick-open box asks for.
const PALETTE_HITS: usize = 60;

/// The view of a cell a hit opens: the hit's own, else the schematic, the symbol, the
/// layout, the first that has a viewer.
fn view_to_open(project: &Project, hit: &Hit) -> Option<String> {
    if hit.view.is_some() {
        return hit.view.clone();
    }
    let cell = project.library(&hit.library)?.cell(&hit.cell)?;
    let openable = |kind: Option<ViewKind>| {
        cell.entries
            .iter()
            .find(|e| crate::app::has_viewer(e) && kind.is_none_or(|k| e.kind == k))
    };
    [
        Some(ViewKind::Schematic),
        Some(ViewKind::Symbol),
        Some(ViewKind::Layout),
        None,
    ]
    .into_iter()
    .find_map(openable)
    .map(|e| e.name.clone())
}

fn selection_of(hit: &Hit) -> Selection {
    match &hit.view {
        Some(view) => Selection::view(&hit.library, &hit.cell, view),
        None => Selection::cell(&hit.library, &hit.cell),
    }
}

/// Select the hit in the library views and bring the Library Manager's list to it.
fn jump(ctx: &Context, app: &mut AppCtx, hit: &Hit) {
    app.selection = selection_of(hit);
    reveal(ctx);
}

/// Open a view of the hit in its editor.
fn open(ctx: &Context, app: &mut AppCtx, hit: &Hit) {
    jump(ctx, app, hit);
    let view = app.project.as_ref().and_then(|p| view_to_open(p, hit));
    match view {
        Some(view) => app.request(Action::OpenView(Selection::view(
            &hit.library,
            &hit.cell,
            &view,
        ))),
        None => tracing::info!(
            target: "cdept::library",
            "{}/{} has no view CDePT can open",
            hit.library,
            hit.cell
        ),
    }
}

/// Ask the Library Manager to scroll its cell list to the selected cell.
pub(crate) fn reveal(ctx: &Context) {
    ctx.data_mut(|data| data.insert_temp(Id::new("cdept-reveal-cell"), 2u8));
    ctx.request_repaint();
}

/// For the Library Manager's cell list: the scroll offset that shows row `index`
/// (the selected cell), when a jump asked for it. Asked for during two frames, as the
/// list may only have its rows on the second.
pub(crate) fn reveal_offset(ui: &Ui, index: Option<usize>, row_height: f32) -> Option<f32> {
    let id = Id::new("cdept-reveal-cell");
    let left = ui.ctx().data(|data| data.get_temp::<u8>(id))?;
    if left == 0 {
        return None;
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, left - 1));
    let row = row_height + ui.spacing().item_spacing.y;
    // A few rows above stay in sight.
    Some((index? as f32 - 3.0).max(0.0) * row)
}

/// One line for a hit: where it is, and for a hit inside a file, the line.
fn hit_text(hit: &Hit) -> String {
    let mut text = format!("{} / {}", hit.library, hit.cell);
    if let Some(view) = &hit.view {
        text.push_str(&format!(" / {view}"));
    }
    if let Matched::Content {
        line,
        text: found,
        kind,
    } = &hit.matched
    {
        text.push_str(&format!("  :{line}  [{}]  {found}", kind.label()));
    }
    text
}

/// The quick-open box.
#[derive(Clone, Default)]
struct Palette {
    open: bool,
    /// Opened this frame: the field takes the keyboard.
    fresh: bool,
    text: String,
    /// The text the hits are for.
    searched: Option<String>,
    hits: Arc<Vec<Hit>>,
    total: usize,
    index: usize,
}

fn palette_id() -> Id {
    Id::new("cdept-palette")
}

/// Open the quick-open box, with `text` in it.
pub(crate) fn open_palette(ctx: &Context, text: &str) {
    ctx.data_mut(|data| {
        data.insert_temp(
            palette_id(),
            Palette {
                open: true,
                fresh: true,
                text: text.to_string(),
                ..Palette::default()
            },
        )
    });
    ctx.request_repaint();
}

/// Once per frame, before the windows are drawn: the shortcuts and the box itself.
pub(crate) fn frame(ctx: &Context, app: &mut AppCtx) {
    let schematic_in_front = app.schematic.is_some();
    let pressed = ctx.input_mut(|input| {
        input.consume_shortcut(&FIND) || (!schematic_in_front && input.consume_shortcut(&GO_TO))
    });
    if pressed && app.project.is_some() {
        open_palette(ctx, "");
    }
    let Some(mut palette) = ctx
        .data(|data| data.get_temp::<Palette>(palette_id()))
        .filter(|p| p.open)
    else {
        return;
    };
    let Some(project) = &app.project else {
        ctx.data_mut(|data| data.remove::<Palette>(palette_id()));
        return;
    };

    if palette.searched.as_deref() != Some(palette.text.as_str()) {
        let mut query = Query::parse(&palette.text);
        query.content = false; // names only: this runs at every keystroke
        query.limit = PALETTE_HITS;
        let results = search::search(project, &query);
        palette.total = results.total;
        palette.hits = Arc::new(results.hits);
        palette.searched = Some(palette.text.clone());
        palette.index = 0;
    }

    // Keys that are not text: taken before the field sees them.
    let (down, up, enter, shift_enter, escape) = ctx.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::ArrowDown),
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::Enter),
            input.consume_key(Modifiers::SHIFT, Key::Enter)
                || input.consume_key(Modifiers::COMMAND, Key::Enter),
            input.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    let count = palette.hits.len();
    if down && count > 0 {
        palette.index = (palette.index + 1) % count;
    }
    if up && count > 0 {
        palette.index = (palette.index + count - 1) % count;
    }
    let moved = down || up;

    let mut chosen: Option<(usize, bool)> = None;
    if enter || shift_enter {
        chosen = (count > 0).then_some((palette.index, shift_enter));
    }
    let mut close = escape;
    let response = egui::Window::new("Go to Cell")
        .id(Id::new("cdept-palette-window"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(Align2::CENTER_TOP, [0.0, 90.0])
        .fixed_size([620.0, 0.0])
        .show(ctx, |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut palette.text)
                    .hint_text("Go to cell: name, library/cell, lib: kind: cat: …")
                    .desired_width(f32::INFINITY),
            );
            if palette.fresh || !field.has_focus() {
                field.request_focus();
            }
            ui.separator();
            if count == 0 {
                ui.weak(if palette.text.trim().is_empty() {
                    "Type a part of a cell name."
                } else {
                    "No cell matches."
                });
            }
            ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                for (n, hit) in palette.hits.iter().enumerate() {
                    let row = ui.selectable_label(n == palette.index, hit_text(hit));
                    if n == palette.index && moved {
                        row.scroll_to_me(None);
                    }
                    if row.double_clicked() {
                        chosen = Some((n, true));
                    } else if row.clicked() {
                        chosen = Some((n, false));
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.small("Enter: go to the cell   Shift+Enter: open it   Esc: close");
                if palette.total > count {
                    ui.small(format!("   {count} of {} shown", palette.total));
                }
            });
        });
    palette.fresh = false;
    // A click outside closes the box.
    if let Some(window) = response
        && ctx.input(|i| i.pointer.any_pressed())
        && !ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|pos| window.response.rect.contains(pos))
    {
        close = true;
    }
    if let Some((n, open_it)) = chosen {
        let hit = palette.hits[n].clone();
        if open_it {
            open(ctx, app, &hit);
        } else {
            jump(ctx, app, &hit);
            app.request(Action::OpenTab(Tab::LibraryManager));
        }
        close = true;
    }
    if close {
        ctx.data_mut(|data| data.remove::<Palette>(palette_id()));
    } else {
        ctx.data_mut(|data| data.insert_temp(palette_id(), palette));
    }
}

/// A search running behind the window.
type Job = Arc<Mutex<Option<Results>>>;

/// The Search panel.
#[derive(Default)]
pub struct SearchView {
    text: String,
    library: Option<String>,
    kind: Option<ViewKind>,
    category: Option<String>,
    content: bool,
    /// What the results are for.
    searched: Option<String>,
    results: Results,
    job: Option<Job>,
    selected: Option<usize>,
}

impl SearchView {
    /// The query of the widgets, as text (also what tells that it changed).
    fn query_text(&self) -> String {
        let mut text = self.text.trim().to_string();
        if let Some(library) = &self.library {
            text.push_str(&format!(" lib:{library}"));
        }
        if let Some(kind) = self.kind {
            text.push_str(&format!(" kind:{}", kind.name()));
        }
        if let Some(category) = &self.category {
            text.push_str(&format!(" cat:{category}"));
        }
        if self.content {
            text.push_str(" in:content");
        }
        text
    }

    fn start(&mut self, ctx: &Context, project: &Project) {
        let text = self.query_text();
        let mut query = Query::parse(&text);
        query.limit = 2000;
        self.searched = Some(text);
        self.selected = None;
        if !query.content {
            self.results = search::search(project, &query);
            self.job = None;
            return;
        }
        // Looking inside files takes time: on a copy of the project, behind the window.
        let job: Job = Arc::default();
        self.job = Some(job.clone());
        let (project, ctx) = (project.clone(), ctx.clone());
        std::thread::spawn(move || {
            let results = search::search(&project, &query);
            *job.lock().unwrap() = Some(results);
            ctx.request_repaint();
        });
    }

    /// The hits, for the test scripts.
    pub(crate) fn script_hits(&self) -> Vec<String> {
        self.results.hits.iter().map(hit_text).collect()
    }

    /// Set the query and search, for the test scripts.
    pub(crate) fn script_search(&mut self, ctx: &Context, project: &Project, text: &str) {
        let query = Query::parse(text);
        self.content = query.content;
        self.library = None;
        self.kind = None;
        self.category = None;
        self.text = text.replace("in:content", "").trim().to_string();
        self.start(ctx, project);
    }

    pub(crate) fn script_busy(&self) -> bool {
        self.job.is_some()
    }
}

impl View for SearchView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Search".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let Some(project) = &ctx.project else {
            return empty_state(ui, "No project open.", None);
        };
        if let Some(job) = &self.job {
            let done = job.lock().unwrap().take();
            if let Some(results) = done {
                self.results = results;
                self.job = None;
            }
        }
        let mut go = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.text)
                    .hint_text("🔍 Cells by name; patterns with * ?; lib: kind: cat: …")
                    .desired_width(ui.available_width() - 220.0),
            );
            go |= field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui
                .checkbox(&mut self.content, "Inside files")
                .on_hover_text(
                    "Also look for the words inside schematics, symbols, netlists and HDL \
                     files: pins, instances and text lines. Press Enter to search.",
                )
                .changed()
            {
                go = true;
            }
            go |= ui.button("Search").clicked();
        });
        ui.horizontal(|ui| {
            let any = "Any";
            egui::ComboBox::from_id_salt("search-library")
                .selected_text(format!(
                    "Library: {}",
                    self.library.as_deref().unwrap_or(any)
                ))
                .show_ui(ui, |ui| {
                    go |= ui.selectable_value(&mut self.library, None, any).changed();
                    for library in &project.libraries {
                        go |= ui
                            .selectable_value(
                                &mut self.library,
                                Some(library.name.clone()),
                                &library.name,
                            )
                            .changed();
                    }
                });
            egui::ComboBox::from_id_salt("search-kind")
                .selected_text(format!("View: {}", self.kind.map_or(any, |k| k.name())))
                .show_ui(ui, |ui| {
                    go |= ui.selectable_value(&mut self.kind, None, any).changed();
                    for kind in ViewKind::ALL {
                        go |= ui
                            .selectable_value(&mut self.kind, Some(kind), kind.name())
                            .changed();
                    }
                });
            let state = super::categories::Categories::of(ui.ctx());
            let mut names: Vec<String> = project
                .libraries
                .iter()
                .filter(|l| self.library.as_ref().is_none_or(|name| &l.name == name))
                .flat_map(|l| state.get(l).all.names())
                .collect();
            names.sort();
            names.dedup();
            egui::ComboBox::from_id_salt("search-category")
                .selected_text(format!(
                    "Category: {}",
                    self.category.as_deref().unwrap_or(any)
                ))
                .show_ui(ui, |ui| {
                    go |= ui.selectable_value(&mut self.category, None, any).changed();
                    for name in names {
                        go |= ui
                            .selectable_value(&mut self.category, Some(name.clone()), name)
                            .changed();
                    }
                });
        });
        // Names are searched while typing; inside files only when asked.
        let changed = self.searched.as_deref() != Some(self.query_text().as_str());
        if go || (changed && !self.content) {
            let egui_ctx = ui.ctx().clone();
            self.start(&egui_ctx, project);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if self.job.is_some() {
                ui.spinner();
                ui.weak("Looking inside the files…");
            } else if changed && self.content {
                ui.weak("Press Enter to search inside the files.");
            } else {
                let shown = self.results.hits.len();
                let mut text = format!("{} hit(s)", self.results.total);
                if shown < self.results.total {
                    text.push_str(&format!(", {shown} shown"));
                }
                if self.results.files_scanned > 0 {
                    text.push_str(&format!(", {} files read", self.results.files_scanned));
                }
                ui.weak(text);
                if self.results.truncated {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Stopped early: narrow the search with a library or a view kind.",
                    );
                }
            }
        });
        let query = Query::parse(&self.query_text());
        for problem in &query.problems {
            ui.colored_label(ui.visuals().warn_fg_color, problem);
        }

        let mut act: Option<(Hit, bool)> = None;
        let row_height = ui.spacing().interact_size.y;
        ScrollArea::both().auto_shrink(false).show_rows(
            ui,
            row_height,
            self.results.hits.len(),
            |ui, range| {
                for n in range {
                    let hit = &self.results.hits[n];
                    let text = match &hit.matched {
                        Matched::Name => RichText::new(hit_text(hit)),
                        Matched::Content { .. } => RichText::new(hit_text(hit)).monospace(),
                    };
                    let row = ui
                        .selectable_label(self.selected == Some(n), text)
                        .on_hover_text("Click: show in the library views. Double-click: open.");
                    if row.double_clicked() {
                        act = Some((hit.clone(), true));
                        self.selected = Some(n);
                    } else if row.clicked() {
                        act = Some((hit.clone(), false));
                        self.selected = Some(n);
                    }
                }
            },
        );
        if let Some((hit, open_it)) = act {
            let egui_ctx = ui.ctx().clone();
            if open_it {
                open(&egui_ctx, ctx, &hit);
            } else {
                jump(&egui_ctx, ctx, &hit);
            }
        }
    }
}

/// For the test scripts: `organise find <query>` opens the quick-open box with the
/// query; `organise go` and `organise open` choose its first hit.
pub(crate) fn script(ctx: &Context, app: &mut AppCtx, args: &[String]) -> Result<(), String> {
    let (verb, rest) = args.split_first().ok_or("usage: organise <verb> …")?;
    let text = rest.join(" ");
    match verb.as_str() {
        "find" => open_palette(ctx, &text),
        "go" | "open" => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let palette = ctx
                .data(|data| data.get_temp::<Palette>(palette_id()))
                .filter(|p| p.open)
                .ok_or("the quick-open box is not open")?;
            // The box may not have been drawn yet: search here.
            let mut query = Query::parse(&palette.text);
            query.limit = PALETTE_HITS;
            let hit = search::search(project, &query)
                .hits
                .into_iter()
                .next()
                .ok_or(format!("'{}' finds nothing", palette.text))?;
            if verb == "open" {
                open(ctx, app, &hit);
            } else {
                jump(ctx, app, &hit);
            }
            ctx.data_mut(|data| data.remove::<Palette>(palette_id()));
        }
        // expect_selection <library> <cell> [view]
        "expect_selection" => {
            let wanted = match rest {
                [library, cell] => Selection::cell(library, cell),
                [library, cell, view] => Selection::view(library, cell, view),
                _ => return Err("usage: organise expect_selection <library> <cell> [view]".into()),
            };
            if app.selection != wanted {
                return Err(format!(
                    "the selection is {:?}, expected {wanted:?}",
                    app.selection
                ));
            }
        }
        other => return Err(format!("organise: unknown verb '{other}'")),
    }
    Ok(())
}
