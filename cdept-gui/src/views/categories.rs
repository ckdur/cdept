//! Categories in the library views: a Category column in the Library Manager, cells
//! grouped by category in the Libraries tree, and the menus that change them.
//!
//! The library views call the functions here at a few places (the toggle, the
//! Category column, the cell filter, the menus); everything else is in this file. The
//! state is kept in egui's memory, so any code with a `Ui` reaches it without it being
//! handed down: what is shown, the category selected in each library, the categories
//! as read from disk (looked at again every second), and the changes asked for, which
//! [`frame`] carries out once per frame (`libcdept::category` does the work).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use egui::{CollapsingHeader, Context, Id, RichText, Ui};
use libcdept::category::{self, LibraryCategories, Node, Store};
use libcdept::{Cell, Library};

use crate::state::AppCtx;

/// How often the category files are looked at again.
const RECHECK: Duration = Duration::from_secs(1);

/// Which cells of a library the Cell column shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    All,
    Uncategorised,
    Category(String),
}

/// A change to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    Add {
        library: String,
        category: String,
    },
    Remove {
        library: String,
        category: String,
    },
    Rename {
        library: String,
        old: String,
        new: String,
    },
    Assign {
        library: String,
        category: String,
        cell: String,
    },
    Unassign {
        library: String,
        category: String,
        cell: String,
    },
    Suggest {
        library: String,
    },
    Adopt {
        library: String,
    },
}

/// A name being asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Prompt {
    /// A new category, below `parent`, with `cell` put into it.
    New {
        library: String,
        parent: Option<String>,
        cell: Option<String>,
        text: String,
    },
    Rename {
        library: String,
        old: String,
        text: String,
    },
}

struct Cached {
    data: Arc<LibraryCategories>,
    stamp: Stamp,
    checked: Instant,
}

/// What tells that the categories of a library changed on disk: the files' times,
/// the folder's (a LibMan file added) and the number of cells.
type Stamp = ([Option<SystemTime>; 3], usize);

#[derive(Default)]
struct Inner {
    /// Show the Category column and group the tree.
    show: bool,
    selected: HashMap<String, Filter>,
    /// By library folder.
    cache: HashMap<PathBuf, Cached>,
    /// The project folder the cache is for.
    root: PathBuf,
    ops: Vec<Op>,
    prompt: Option<Prompt>,
    /// The prompt was opened this frame: its field takes the keyboard.
    prompt_new: bool,
}

/// The categories as the library views see them. Cheap to copy: all copies share one
/// state.
#[derive(Clone, Default)]
pub(crate) struct Categories(Arc<Mutex<Inner>>);

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn stamp(root: &Path, library: &Library) -> Stamp {
    let [lib_file, project_file] = category::files(root, library);
    (
        [
            modified(&lib_file),
            modified(&project_file),
            modified(&library.path),
        ],
        library.cells.len(),
    )
}

impl Categories {
    /// The shared state, from anywhere there is an egui context.
    pub(crate) fn of(ctx: &Context) -> Categories {
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Categories>(Id::new("cdept-categories"))
                .clone()
        })
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The Category column and the grouping of the tree are on.
    pub(crate) fn shown(&self) -> bool {
        self.inner().show
    }

    pub(crate) fn set_shown(&self, show: bool) {
        self.inner().show = show;
    }

    /// The categories of a library, read from disk when they changed.
    pub(crate) fn get(&self, library: &Library) -> Arc<LibraryCategories> {
        let mut inner = self.inner();
        let root = inner.root.clone();
        let now = Instant::now();
        if let Some(cached) = inner.cache.get_mut(&library.path) {
            if now.duration_since(cached.checked) < RECHECK {
                return cached.data.clone();
            }
            cached.checked = now;
            let current = stamp(&root, library);
            if current == cached.stamp {
                return cached.data.clone();
            }
        }
        let data = Arc::new(category::load_at(&root, library));
        for warning in &data.warnings {
            tracing::warn!(target: "cdept::library", "{warning}");
        }
        inner.cache.insert(
            library.path.clone(),
            Cached {
                data: data.clone(),
                stamp: stamp(&root, library),
                checked: now,
            },
        );
        data
    }

    fn forget(&self) {
        self.inner().cache.clear();
    }

    pub(crate) fn filter(&self, library: &str) -> Filter {
        self.inner()
            .selected
            .get(library)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn set_filter(&self, library: &str, filter: Filter) {
        self.inner().selected.insert(library.to_string(), filter);
    }

    pub(crate) fn push(&self, op: Op) {
        self.inner().ops.push(op);
    }

    fn ask(&self, prompt: Prompt) {
        let mut inner = self.inner();
        inner.prompt = Some(prompt);
        inner.prompt_new = true;
    }

    /// Whether the Cell column shows `cell`, by the category selected for its library.
    /// Always true while the Category column is off.
    pub(crate) fn shows(&self, library: &Library, cell: &str) -> bool {
        if !self.shown() {
            return true;
        }
        match self.filter(&library.name) {
            Filter::All => true,
            Filter::Uncategorised => self.get(library).all.is_uncategorised(cell),
            Filter::Category(name) => self.get(library).all.contains(&name, cell),
        }
    }
}

/// The "Categories" toggle of the Library Manager's tool row.
pub(crate) fn toggle(ui: &mut Ui) {
    let state = Categories::of(ui.ctx());
    let mut show = state.shown();
    if ui
        .checkbox(&mut show, "Categories")
        .on_hover_text(
            "Show a Category column: pick a category to see its cells. Categories are \
             made and filled from the right-click menus.",
        )
        .changed()
    {
        state.set_shown(show);
    }
}

/// The same toggle as a small button, for the Libraries tree.
pub(crate) fn toggle_button(ui: &mut Ui) {
    let state = Categories::of(ui.ctx());
    let show = state.shown();
    if ui
        .selectable_label(show, "🏷")
        .on_hover_text("Group the cells by category")
        .clicked()
    {
        state.set_shown(!show);
    }
}

/// Whether the library views show categories.
pub(crate) fn shown(ui: &Ui) -> bool {
    Categories::of(ui.ctx()).shown()
}

/// Whether the Cell column shows `cell` (see [`Categories::shows`]).
pub(crate) fn shows(ui: &Ui, library: &Library, cell: &str) -> bool {
    Categories::of(ui.ctx()).shows(library, cell)
}

fn writable_note(found: &LibraryCategories) -> String {
    format!("Kept in {}.", found.store.label())
}

/// Entries of the menu of a category.
fn category_menu(ui: &mut Ui, state: &Categories, library: &Library, path: &str) {
    if ui.button("New Subcategory…").clicked() {
        state.ask(Prompt::New {
            library: library.name.clone(),
            parent: Some(path.to_string()),
            cell: None,
            text: String::new(),
        });
    }
    if ui.button("Rename…").clicked() {
        state.ask(Prompt::Rename {
            library: library.name.clone(),
            old: path.to_string(),
            text: path.to_string(),
        });
    }
    if ui
        .button("Delete Category")
        .on_hover_text("The cells stay in the library.")
        .clicked()
    {
        state.push(Op::Remove {
            library: library.name.clone(),
            category: path.to_string(),
        });
        if state.filter(&library.name) == Filter::Category(path.to_string()) {
            state.set_filter(&library.name, Filter::All);
        }
    }
}

/// Entries about the categories of a whole library (also in the library's menu).
pub(crate) fn library_menu(ui: &mut Ui, library: &Library) {
    let state = Categories::of(ui.ctx());
    ui.menu_button("Categories", |ui| {
        let found = state.get(library);
        if ui.button("New Category…").clicked() {
            state.ask(Prompt::New {
                library: library.name.clone(),
                parent: None,
                cell: None,
                text: String::new(),
            });
        }
        if ui
            .button("Suggest Categories")
            .on_hover_text(
                "By the type of the xschem symbols (nmos, resistor, …) or, for standard \
                 cells, by function (and, nand, dff, …).",
            )
            .clicked()
        {
            state.push(Op::Suggest {
                library: library.name.clone(),
            });
            state.set_shown(true);
        }
        if !found.libman.is_empty()
            && ui
                .button("Adopt LibMan Categories")
                .on_hover_text(
                    "Copy the categories of LibMan's *.group files into CDePT's own, so \
                     they can be changed here.",
                )
                .clicked()
        {
            state.push(Op::Adopt {
                library: library.name.clone(),
            });
        }
        ui.separator();
        ui.weak(writable_note(&found));
    });
}

/// The "Categories" submenu of a cell: a tick for every category, and a new one.
pub(crate) fn cell_menu(ui: &mut Ui, library: &Library, cell: &Cell) {
    let state = Categories::of(ui.ctx());
    ui.menu_button("Categories", |ui| {
        let found = state.get(library);
        let names = found.all.names();
        if names.is_empty() {
            ui.weak("No categories yet");
        }
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for name in &names {
                    let mut inside = found.all.cells(name).contains(&cell.name.as_str());
                    if ui.checkbox(&mut inside, name).changed() {
                        let (library, category, cell) =
                            (library.name.clone(), name.clone(), cell.name.clone());
                        state.push(if inside {
                            Op::Assign {
                                library,
                                category,
                                cell,
                            }
                        } else {
                            Op::Unassign {
                                library,
                                category,
                                cell,
                            }
                        });
                    }
                }
            });
        ui.separator();
        if ui.button("New Category…").clicked() {
            state.ask(Prompt::New {
                library: library.name.clone(),
                parent: None,
                cell: Some(cell.name.clone()),
                text: String::new(),
            });
        }
        ui.weak(writable_note(&found));
    });
}

fn picker_nodes(ui: &mut Ui, state: &Categories, library: &Library, nodes: &[Node], depth: usize) {
    let selected = state.filter(&library.name);
    for node in nodes {
        let active = selected == Filter::Category(node.path.clone());
        let response = ui
            .horizontal(|ui| {
                ui.add_space(12.0 * depth as f32);
                ui.selectable_label(active, format!("{}  ({})", node.name, node.count()))
            })
            .inner
            .on_hover_text(&node.path);
        if response.clicked() {
            state.set_filter(&library.name, Filter::Category(node.path.clone()));
        }
        response.context_menu(|ui| category_menu(ui, state, library, &node.path));
        picker_nodes(ui, state, library, &node.children, depth + 1);
    }
}

/// The Category column of the Library Manager: every cell, the categories as a tree,
/// the cells in none. A click chooses what the Cell column shows.
pub(crate) fn picker(ui: &mut Ui, library: &Library) {
    let state = Categories::of(ui.ctx());
    let found = state.get(library);
    let selected = state.filter(&library.name);
    if ui
        .selectable_label(
            selected == Filter::All,
            format!("All  ({})", library.cells.len()),
        )
        .clicked()
    {
        state.set_filter(&library.name, Filter::All);
    }
    picker_nodes(ui, &state, library, &found.all.tree(), 0);
    let loose = library
        .cells
        .iter()
        .filter(|c| found.all.is_uncategorised(&c.name))
        .count();
    if !found.all.is_empty()
        && ui
            .selectable_label(
                selected == Filter::Uncategorised,
                RichText::new(format!("Uncategorised  ({loose})")).weak(),
            )
            .clicked()
    {
        state.set_filter(&library.name, Filter::Uncategorised);
    }
    if found.all.is_empty() {
        ui.add_space(6.0);
        ui.weak("No categories.");
        ui.small("Right-click here or on a cell to make one.");
    }
    let rest = ui.allocate_response(ui.available_size(), egui::Sense::click());
    rest.context_menu(|ui| {
        if ui.button("New Category…").clicked() {
            state.ask(Prompt::New {
                library: library.name.clone(),
                parent: None,
                cell: None,
                text: String::new(),
            });
        }
        if ui.button("Suggest Categories").clicked() {
            state.push(Op::Suggest {
                library: library.name.clone(),
            });
        }
    });
}

fn grouped_nodes<'a>(
    ui: &mut Ui,
    state: &Categories,
    library: &Library,
    nodes: &[Node],
    cells: &[&'a Cell],
    each: &mut dyn FnMut(&mut Ui, &'a Cell),
) {
    for node in nodes {
        // Only what the cell filter left.
        let inside = |node: &Node| -> usize {
            fn count(node: &Node, cells: &[&Cell]) -> usize {
                cells
                    .iter()
                    .filter(|c| node.cells.contains(&c.name))
                    .count()
                    + node.children.iter().map(|n| count(n, cells)).sum::<usize>()
            }
            count(node, cells)
        };
        let shown = inside(node);
        let header = CollapsingHeader::new(RichText::new(format!("🏷 {}  ({shown})", node.name)))
            .id_salt(("category", &library.name, &node.path))
            .show(ui, |ui| {
                grouped_nodes(ui, state, library, &node.children, cells, each);
                for cell in cells.iter().filter(|c| node.cells.contains(&c.name)) {
                    // The same cell may show under several categories.
                    each(ui, cell);
                }
            })
            .header_response;
        header.context_menu(|ui| category_menu(ui, state, library, &node.path));
    }
}

/// The cells of a library in the Libraries tree: one after the other, or under their
/// categories when those are shown (cells in none come last).
pub(crate) fn grouped<'a>(
    ui: &mut Ui,
    library: &Library,
    cells: &[&'a Cell],
    mut each: impl FnMut(&mut Ui, &'a Cell),
) {
    let state = Categories::of(ui.ctx());
    let found = state.shown().then(|| state.get(library));
    let Some(found) = found.filter(|f| !f.all.is_empty()) else {
        for cell in cells {
            each(ui, cell);
        }
        return;
    };
    grouped_nodes(ui, &state, library, &found.all.tree(), cells, &mut each);
    for cell in cells.iter().filter(|c| found.all.is_uncategorised(&c.name)) {
        each(ui, cell);
    }
}

/// Carry out one change. Returns what to tell the user, if anything.
fn run(project: &libcdept::Project, op: &Op) -> eyre::Result<Option<String>> {
    match op {
        Op::Add { library, category } => category::add(project, library, category)?,
        Op::Remove { library, category } => category::remove(project, library, category)?,
        Op::Rename { library, old, new } => category::rename(project, library, old, new)?,
        Op::Assign {
            library,
            category,
            cell,
        } => category::assign(project, library, category, &[cell.as_str()])?,
        Op::Unassign {
            library,
            category,
            cell,
        } => category::unassign(project, library, category, &[cell.as_str()])?,
        Op::Suggest { library } => {
            let lib = project
                .library(library)
                .ok_or_else(|| eyre::eyre!("no library '{library}'"))?;
            let suggested = category::suggest(lib);
            if suggested.is_empty() {
                eyre::bail!(
                    "Nothing to suggest for '{library}': its cells have no xschem symbols \
                     with a type, and their names do not look like a standard-cell library's."
                );
            }
            let added = category::add_all(project, library, &suggested)?;
            let store = match category::store_of(lib) {
                Store::Library => "its lib.toml",
                Store::Project => "the project's categories.toml",
            };
            return Ok(Some(format!(
                "{library}: {} categories suggested, {added} cell(s) put into them (in {store})",
                suggested.names().len()
            )));
        }
        Op::Adopt { library } => {
            let added = category::adopt_libman(project, library)?;
            return Ok(Some(format!(
                "{library}: {added} cell(s) put into categories copied from LibMan's files"
            )));
        }
    }
    Ok(None)
}

/// Once per frame: carry out the changes asked for and show the name prompt.
pub(crate) fn frame(ctx: &Context, app: &mut AppCtx) {
    let state = Categories::of(ctx);
    // Another project: nothing of the old one is right any more.
    let root = app
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .unwrap_or_default();
    {
        let mut inner = state.inner();
        if inner.root != root {
            *inner = Inner {
                show: inner.show,
                root,
                ..Inner::default()
            };
        }
    }
    let ops = std::mem::take(&mut state.inner().ops);
    if !ops.is_empty() {
        if let Some(project) = &app.project {
            let mut errors = Vec::new();
            for op in &ops {
                match run(project, op) {
                    Ok(Some(message)) => tracing::info!(target: "cdept::library", "{message}"),
                    Ok(None) => {}
                    Err(error) => errors.push(format!("{error:#}")),
                }
            }
            if !errors.is_empty() {
                app.error(errors.join("\n"));
            }
        }
        state.forget();
        ctx.request_repaint();
    }
    prompt(ctx, &state);
}

/// The window that asks for a category name.
fn prompt(ctx: &Context, state: &Categories) {
    let (Some(mut prompt), focus) = ({
        let mut inner = state.inner();
        (inner.prompt.clone(), std::mem::take(&mut inner.prompt_new))
    }) else {
        return;
    };
    let mut done = false;
    let mut cancel = false;
    let title = match &prompt {
        Prompt::New {
            parent: Some(_), ..
        } => "New Subcategory",
        Prompt::New { .. } => "New Category",
        Prompt::Rename { .. } => "Rename Category",
    };
    egui::Window::new(title)
        .id(Id::new("cdept-category-prompt"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let (library, parent, text) = match &mut prompt {
                Prompt::New {
                    library,
                    parent,
                    text,
                    ..
                } => (library.clone(), parent.clone(), text),
                Prompt::Rename { library, text, .. } => (library.clone(), None, text),
            };
            ui.label(format!("Library: {library}"));
            if let Some(parent) = &parent {
                ui.label(format!("Inside: {parent}"));
            }
            let field = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text("name, or outer/inner")
                    .desired_width(260.0),
            );
            if focus {
                field.request_focus();
            }
            let problem = category::normalise(text).err().map(|e| e.to_string());
            let empty = text.trim().is_empty();
            if let (Some(problem), false) = (&problem, empty) {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
            }
            ui.small("A / makes a category inside another: analog/amplifiers.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = ui.add_enabled(problem.is_none(), egui::Button::new("OK"));
                if (ok.clicked() || enter) && problem.is_none() {
                    done = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
    if cancel {
        state.inner().prompt = None;
        return;
    }
    if !done {
        state.inner().prompt = Some(prompt);
        return;
    }
    state.inner().prompt = None;
    match prompt {
        Prompt::New {
            library,
            parent,
            cell,
            text,
        } => {
            let category = match parent {
                Some(parent) => format!("{parent}/{}", text.trim()),
                None => text.trim().to_string(),
            };
            state.push(match cell {
                Some(cell) => Op::Assign {
                    library,
                    category,
                    cell,
                },
                None => Op::Add { library, category },
            });
            state.set_shown(true);
        }
        Prompt::Rename { library, old, text } => {
            let new = text.trim().to_string();
            if new != old {
                if state.filter(&library) == Filter::Category(old.clone()) {
                    state.set_filter(&library, Filter::Category(new.clone()));
                }
                state.push(Op::Rename { library, old, new });
            }
        }
    }
}

/// For the test scripts: `organise category <verb> …` (see [`crate::import::script`]).
pub(crate) fn script(ctx: &Context, app: &AppCtx, args: &[String]) -> Result<(), String> {
    let state = Categories::of(ctx);
    let (verb, rest) = args
        .split_first()
        .ok_or("usage: organise category <verb> …")?;
    let word = |at: usize| -> Result<String, String> {
        rest.get(at)
            .cloned()
            .ok_or(format!("organise category {verb}: an argument is missing"))
    };
    let library = |name: &str| -> Result<&Library, String> {
        app.project
            .as_ref()
            .and_then(|p| p.library(name))
            .ok_or(format!("no library '{name}'"))
    };
    match verb.as_str() {
        "show" => state.set_shown(word(0)? != "off"),
        "pick" => {
            let filter = match word(1)?.as_str() {
                "all" => Filter::All,
                "none" => Filter::Uncategorised,
                name => Filter::Category(name.to_string()),
            };
            state.set_filter(&word(0)?, filter);
        }
        "add" => state.push(Op::Add {
            library: word(0)?,
            category: word(1)?,
        }),
        "remove" => state.push(Op::Remove {
            library: word(0)?,
            category: word(1)?,
        }),
        "rename" => state.push(Op::Rename {
            library: word(0)?,
            old: word(1)?,
            new: word(2)?,
        }),
        "assign" => state.push(Op::Assign {
            library: word(0)?,
            category: word(1)?,
            cell: word(2)?,
        }),
        "unassign" => state.push(Op::Unassign {
            library: word(0)?,
            category: word(1)?,
            cell: word(2)?,
        }),
        "suggest" => state.push(Op::Suggest { library: word(0)? }),
        // expect <library> <category> <cell,cell,…|none>: the cells of a category (and
        // of those below it), as the views show them.
        "expect" => {
            state.forget();
            let lib = library(&word(0)?)?;
            let found = state.get(lib);
            let mut cells: Vec<&str> = found.all.cells_under(&word(1)?).into_iter().collect();
            cells.sort_unstable();
            let wanted = word(2)?;
            let mut wanted: Vec<&str> = if wanted == "none" {
                Vec::new()
            } else {
                wanted.split(',').collect()
            };
            wanted.sort_unstable();
            if cells != wanted {
                return Err(format!(
                    "category '{}' of {} has {cells:?}, expected {wanted:?}",
                    rest[1], rest[0]
                ));
            }
        }
        // expect_shown <library> <n>: how many cells the Cell column shows.
        "expect_shown" => {
            let lib = library(&word(0)?)?;
            let shown = lib
                .cells
                .iter()
                .filter(|c| state.shows(lib, &c.name))
                .count();
            let wanted: usize = word(1)?.parse().map_err(|_| "not a number".to_string())?;
            if shown != wanted {
                return Err(format!(
                    "{shown} cells of {} are shown, expected {wanted}",
                    rest[0]
                ));
            }
        }
        other => return Err(format!("organise category: unknown verb '{other}'")),
    }
    Ok(())
}
