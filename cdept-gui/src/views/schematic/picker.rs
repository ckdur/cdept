//! The library picker: CDePT's own dialog where xschem would show its file browser.
//!
//! xschem asks for a schematic or symbol to open, insert, merge or save as. The answer
//! is a file name, chosen here by library, cell and view like in the Library Manager.

use std::path::{Path, PathBuf};

use egui::{Context, Id, Rect, ScrollArea, Ui};
use libcdept::library::validate_name;
use libcdept::xschem::protocol::Window;
use libcdept::{Cell, Library, LibraryLayout, Project};

/// What the user has chosen so far in one picker.
#[derive(Default)]
pub struct Picker {
    library: Option<String>,
    cell: Option<String>,
    file: Option<PathBuf>,
    /// Cell name typed for "save as".
    name: String,
    started: bool,
    /// The answer went out; the window is about to close.
    pub answered: bool,
}

pub enum Picked {
    Nothing,
    Cancel,
    File(PathBuf),
    /// Use the file chooser of the desktop instead.
    Elsewhere,
}

struct Request<'a> {
    mode: &'a str,
    extensions: Vec<&'a str>,
}

impl Request<'_> {
    fn wants(&self, file: &Path) -> bool {
        let name = file.file_name().map(|n| n.to_string_lossy());
        name.is_some_and(|name| {
            self.extensions
                .iter()
                .any(|ext| name.ends_with(&format!(".{ext}")))
        })
    }

    fn files<'c>(&self, cell: &'c Cell) -> Vec<(&'c str, &'c PathBuf)> {
        cell.entries
            .iter()
            .flat_map(|entry| entry.files.iter().map(move |f| (entry.name.as_str(), f)))
            .filter(|(_, file)| self.wants(file))
            .collect()
    }

    fn verb(&self) -> &'static str {
        match self.mode {
            "save" => "Save",
            "symbol" => "Insert",
            "merge" => "Merge",
            _ => "Open",
        }
    }
}

/// Where "save as" puts cell `name` in `library`, for a file with extension `ext`.
fn save_target(library: &Library, name: &str, ext: &str) -> PathBuf {
    let file = format!("{name}.{ext}");
    match library.layout {
        LibraryLayout::Flat | LibraryLayout::Ref => library.path.join(file),
        LibraryLayout::Cells => {
            let view = match ext {
                "sch" => "schematic",
                "sym" => "symbol",
                other => other,
            };
            library.path.join(name).join(view).join(file)
        }
    }
}

/// Size of one of the three lists, in points.
const COLUMN: egui::Vec2 = egui::Vec2::new(190.0, 250.0);

fn column(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    ui.allocate_ui(COLUMN + egui::Vec2::new(0.0, 24.0), |ui| {
        ui.vertical(|ui| {
            ui.strong(title);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_size(COLUMN);
                ui.set_max_size(COLUMN);
                ScrollArea::vertical()
                    .id_salt(title)
                    .auto_shrink([false, false])
                    .show(ui, content);
            });
        });
    });
}

/// Start from the file xschem suggests, when it is inside a library.
fn preselect(picker: &mut Picker, project: &Project, dir: &str, file: &str) {
    let suggested = Path::new(dir).join(file);
    for library in &project.libraries {
        if !suggested.starts_with(&library.path) {
            continue;
        }
        picker.library = Some(library.name.clone());
        let cell = library
            .cells
            .iter()
            .find(|cell| cell.entries.iter().any(|e| e.files.contains(&suggested)));
        picker.cell = cell.map(|c| c.name.clone());
    }
    let stem = Path::new(file).file_stem().map(|s| s.to_string_lossy());
    picker.name = stem.map(|s| s.into_owned()).unwrap_or_default();
}

/// Draws the picture of a schematic or symbol file into the space given.
pub type ShowPicture<'a> = dyn FnMut(&mut Ui, &Path) + 'a;

/// Show the picker for `window` and return what was decided in this frame. `picture`
/// draws the view chosen.
pub fn show(
    ctx: &Context,
    id: Id,
    area: Rect,
    window: &Window,
    project: Option<&Project>,
    picker: &mut Picker,
    picture: &mut ShowPicture<'_>,
) -> Picked {
    let option = |name: &str| window.root.options.get(name).map_or("", String::as_str);
    let mut extensions: Vec<&str> = option("extensions").split_whitespace().collect();
    if extensions.is_empty() {
        extensions = vec!["sch", "sym"];
    }
    let request = Request {
        mode: option("mode"),
        extensions,
    };
    let Some(project) = project else {
        return Picked::Elsewhere;
    };
    if !std::mem::replace(&mut picker.started, true) {
        preselect(picker, project, option("initialdir"), option("initialfile"));
    }
    let title = match (option("title"), window.title.as_str()) {
        ("", "") => request.verb(),
        ("", title) | (title, _) => title,
    };
    let saving = request.mode == "save";

    let mut picked = Picked::Nothing;
    let mut open = true;
    egui::Window::new(title)
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(area.center())
        .show(ctx, |ui| {
            let library = picker.library.as_deref().and_then(|n| project.library(n));
            let cell = library.and_then(|l| l.cell(picker.cell.as_deref()?));
            ui.horizontal_top(|ui| {
                column(ui, "Library", |ui| {
                    for lib in project.libraries.iter().filter(|l| l.error.is_none()) {
                        let usable = !saving || !lib.read_only;
                        let selected = library.is_some_and(|l| l.name == lib.name);
                        let label = egui::Button::selectable(selected, lib.name.as_str());
                        if ui.add_enabled(usable, label).clicked() {
                            picker.library = Some(lib.name.clone());
                            picker.cell = None;
                            picker.file = None;
                        }
                    }
                });
                column(ui, "Cell", |ui| {
                    let Some(lib) = library else {
                        ui.weak("Select a library");
                        return;
                    };
                    for c in &lib.cells {
                        if !saving && request.files(c).is_empty() {
                            continue;
                        }
                        let selected = cell.is_some_and(|s| s.name == c.name);
                        if ui.selectable_label(selected, &c.name).clicked() {
                            picker.cell = Some(c.name.clone());
                            picker.file = None;
                            picker.name = c.name.clone();
                        }
                    }
                });
                if !saving {
                    column(ui, "View", |ui| {
                        let Some(cell) = cell else {
                            ui.weak("Select a cell");
                            return;
                        };
                        for (view, file) in request.files(cell) {
                            let name = file.file_name().map(|n| n.to_string_lossy());
                            let label = format!("{view}  ({})", name.unwrap_or_default());
                            let selected = picker.file.as_ref() == Some(file);
                            let response = ui.selectable_label(selected, label);
                            if response.double_clicked() {
                                picked = Picked::File(file.clone());
                            } else if response.clicked() {
                                picker.file = Some(file.clone());
                            }
                        }
                    });
                    column(ui, "Picture", |ui| match &picker.file {
                        Some(file) => picture(ui, file),
                        None => {
                            ui.weak("Select a view");
                        }
                    });
                }
            });

            // "Save as": a cell name in the chosen library.
            let mut target = picker.file.clone();
            if saving {
                ui.horizontal(|ui| {
                    ui.label("Cell name:");
                    ui.text_edit_singleline(&mut picker.name);
                });
                let name = picker.name.trim();
                target = match (library, validate_name("cell", name)) {
                    (Some(lib), Ok(())) => {
                        let file = save_target(lib, name, request.extensions[0]);
                        ui.weak(project.portable_path(&file));
                        Some(file)
                    }
                    (Some(_), Err(error)) if !name.is_empty() => {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("{error:#}"));
                        None
                    }
                    _ => None,
                };
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let confirm = egui::Button::new(request.verb());
                if ui.add_enabled(target.is_some(), confirm).clicked()
                    && let Some(file) = target
                {
                    picked = Picked::File(file);
                }
                if ui.button("Cancel").clicked() {
                    picked = Picked::Cancel;
                }
                if ui
                    .button("Other File…")
                    .on_hover_text("Choose a file that is not in a library of the project")
                    .clicked()
                {
                    picked = Picked::Elsewhere;
                }
            });
        });
    if !open {
        picked = Picked::Cancel;
    }
    // xschem writes the file itself; the folders of a new cell have to be there.
    if let (true, Picked::File(file)) = (saving, &picked)
        && let Some(folder) = file.parent()
        && let Err(error) = std::fs::create_dir_all(folder)
    {
        tracing::error!(target: "cdept::library", "cannot create {}: {error}", folder.display());
        return Picked::Nothing;
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_targets_follow_the_library_layout() {
        let library = |layout| Library {
            name: "analog".into(),
            path: PathBuf::from("/p/libs/analog"),
            layout,
            read_only: false,
            description: None,
            defined_in: PathBuf::new(),
            error: None,
            cells: Vec::new(),
            preferred: Default::default(),
        };
        let cells = library(LibraryLayout::Cells);
        assert_eq!(
            save_target(&cells, "ota", "sch"),
            PathBuf::from("/p/libs/analog/ota/schematic/ota.sch")
        );
        assert_eq!(
            save_target(&cells, "ota", "sym"),
            PathBuf::from("/p/libs/analog/ota/symbol/ota.sym")
        );
        assert_eq!(
            save_target(&library(LibraryLayout::Flat), "ota", "sym"),
            PathBuf::from("/p/libs/analog/ota.sym")
        );
    }

    #[test]
    fn only_wanted_files_are_offered() {
        let request = Request {
            mode: "symbol",
            extensions: vec!["sym", "tcl"],
        };
        assert!(request.wants(Path::new("/x/nmos.sym")));
        assert!(request.wants(Path::new("/x/gen.tcl")));
        assert!(!request.wants(Path::new("/x/nmos.sch")));
        assert!(!request.wants(Path::new("/x/sym")));
        assert_eq!(request.verb(), "Insert");
    }
}
