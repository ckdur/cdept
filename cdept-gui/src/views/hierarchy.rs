//! Hierarchy panel: the cells a schematic is built from, through which instances.
//!
//! It follows the schematic document in front, else the cell selected in the library
//! views. xschem works the hierarchy out in the background
//! (`libcdept::xschem::Engine::hierarchy`); the panel shows the last result.

use std::path::{Path, PathBuf};

use egui::{CollapsingHeader, RichText, Ui};
use libcdept::Format;
use libcdept::xschem::{Hierarchy, Pending};

use super::{View, empty_state};
use crate::state::{Action, AppCtx};

/// Levels below the top that start unfolded.
const OPEN_LEVELS: usize = 2;
/// The most lines [`lines`] gives: a cell used many times at several levels unfolds
/// into very many.
const MAX_LINES: usize = 2000;

#[derive(Default)]
pub struct HierarchyView {
    /// The schematic whose hierarchy is shown, or being worked out.
    top: Option<PathBuf>,
    job: Option<Pending<Hierarchy>>,
    result: Option<Result<Hierarchy, String>>,
    /// Also list what is not a subcircuit: transistors, pins, labels.
    all: bool,
    /// The document in front had unsaved changes when last looked at.
    was_modified: bool,
}

/// The schematic to show the hierarchy of: the document in front if it is one (for a
/// symbol, the schematic of its cell), else the schematic of the selected cell.
fn wanted_top(ctx: &AppCtx) -> Option<PathBuf> {
    let project = ctx.project.as_ref()?;
    if let Some(active) = &ctx.schematic {
        if Format::of_file(&active.file) == Some(Format::XschemSchematic) {
            return Some(active.file.clone());
        }
        let cell = project
            .library(&active.doc.library)?
            .cell(&active.doc.cell)?;
        return crate::app::schematic_file(cell).cloned();
    }
    crate::app::schematic_file(ctx.selected_cell()?).cloned()
}

/// What an instance is, in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A subcircuit with a schematic: has a level below.
    Cell(usize),
    /// A schematic that contains itself, directly or not: not unfolded again.
    Recursive(usize),
    /// A subcircuit whose schematic was not found.
    Missing,
    Primitive,
}

fn kind(item: &libcdept::xschem::HierarchyInstance, path: &[usize]) -> Kind {
    match item.child {
        Some(child) if path.contains(&child) => Kind::Recursive(child),
        Some(child) => Kind::Cell(child),
        None if item.instance.kind == "subcircuit" => Kind::Missing,
        None => Kind::Primitive,
    }
}

/// The text of one line: `x1 buf`, with a note where there is no level below.
fn label(hierarchy: &Hierarchy, item: &libcdept::xschem::HierarchyInstance, kind: Kind) -> String {
    let name = &item.instance.name;
    match kind {
        Kind::Cell(child) => format!("{name} {}", hierarchy.cells[child].name),
        Kind::Recursive(child) => {
            format!("{name} {} (contains itself)", hierarchy.cells[child].name)
        }
        Kind::Missing => format!("{name} {} (no schematic)", item.instance.symbol),
        Kind::Primitive => format!("{name} [{}]", item.instance.kind),
    }
}

/// The whole tree as indented lines, the top cell first.
pub fn lines(hierarchy: &Hierarchy, all: bool) -> Vec<String> {
    fn walk(
        hierarchy: &Hierarchy,
        index: usize,
        all: bool,
        path: &mut Vec<usize>,
        out: &mut Vec<String>,
    ) {
        for item in &hierarchy.cells[index].instances {
            if out.len() >= MAX_LINES {
                return;
            }
            let kind = kind(item, path);
            if kind == Kind::Primitive && !all {
                continue;
            }
            out.push(format!(
                "{}{}",
                "  ".repeat(path.len()),
                label(hierarchy, item, kind)
            ));
            if let Kind::Cell(child) = kind {
                path.push(child);
                walk(hierarchy, child, all, path, out);
                path.pop();
            }
        }
    }
    let Some(top) = hierarchy.cells.first() else {
        return Vec::new();
    };
    let mut out = vec![top.name.clone()];
    walk(hierarchy, 0, all, &mut vec![0], &mut out);
    out
}

impl HierarchyView {
    /// The lines of the tree on show (see [`lines`]); empty while there is none.
    pub fn lines(&self) -> Vec<String> {
        match &self.result {
            Some(Ok(hierarchy)) => lines(hierarchy, self.all),
            _ => Vec::new(),
        }
    }

    /// The schematic of the first cell named `cell` in the tree on show.
    pub fn schematic_of(&self, cell: &str) -> Option<PathBuf> {
        let Some(Ok(hierarchy)) = &self.result else {
            return None;
        };
        let found = hierarchy.cells.iter().find(|c| c.name == cell)?;
        Some(found.schematic.clone())
    }

    fn start(&mut self, ctx: &mut AppCtx, egui: &egui::Context, top: &Path) {
        let Some(project) = &ctx.project else {
            return;
        };
        self.job = Some(ctx.engines.hierarchy(project, egui, top.to_path_buf()));
    }

    /// One level of the tree. `path` holds the cells from the top down to this one.
    fn level(
        &self,
        ui: &mut Ui,
        hierarchy: &Hierarchy,
        path: &mut Vec<usize>,
        open: &mut Option<PathBuf>,
    ) {
        let index = *path.last().expect("the top is always there");
        for (n, item) in hierarchy.cells[index].instances.iter().enumerate() {
            let kind = kind(item, path);
            let text = label(hierarchy, item, kind);
            match kind {
                Kind::Primitive if !self.all => {}
                Kind::Primitive | Kind::Missing | Kind::Recursive(_) => {
                    ui.weak(text);
                }
                Kind::Cell(child) => {
                    let cell = &hierarchy.cells[child];
                    let has_level = cell
                        .instances
                        .iter()
                        .any(|i| self.all || i.child.is_some() || i.instance.kind == "subcircuit");
                    let hover = "Double-click to open the schematic";
                    let response = if has_level {
                        CollapsingHeader::new(text)
                            .id_salt((path.len(), n))
                            .default_open(path.len() <= OPEN_LEVELS)
                            .show(ui, |ui| {
                                path.push(child);
                                self.level(ui, hierarchy, path, open);
                                path.pop();
                            })
                            .header_response
                    } else {
                        // Nothing below: a plain line, indented like the folding ones.
                        ui.horizontal(|ui| {
                            ui.add_space(ui.spacing().indent);
                            ui.selectable_label(false, text)
                        })
                        .inner
                    };
                    if response.on_hover_text(hover).double_clicked() {
                        *open = Some(cell.schematic.clone());
                    }
                }
            }
        }
    }
}

impl View for HierarchyView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Hierarchy".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        if ctx.project.is_none() {
            *self = Self {
                all: self.all,
                ..Default::default()
            };
            return empty_state(ui, "No project open.", None);
        }
        let top = wanted_top(ctx);

        // Work it out again for another schematic, and after a save.
        let modified = ctx.schematic.as_ref().is_some_and(|s| s.modified);
        let saved = std::mem::replace(&mut self.was_modified, modified) && !modified;
        let mut refresh = top != self.top || saved;
        ui.horizontal(|ui| {
            let button = ui
                .add_enabled(top.is_some(), egui::Button::new("⟳"))
                .on_hover_text("Read the hierarchy again from the files on disk");
            refresh |= button.clicked();
            ui.checkbox(&mut self.all, "All instances")
                .on_hover_text("Also list transistors, pins and other primitives");
            if self.job.is_some() {
                ui.spinner();
                ui.weak("working…");
            }
        });
        ui.separator();
        if refresh {
            if top != self.top {
                self.result = None;
            }
            self.top = top.clone();
            self.job = None;
            if let Some(top) = &top {
                self.start(ctx, ui.ctx(), top);
            }
        }
        if let Some(result) = self.job.as_ref().and_then(Pending::poll) {
            self.job = None;
            self.result = Some(result);
        }

        if top.is_none() {
            return empty_state(
                ui,
                "Open a schematic, or select a cell, to see its hierarchy.",
                None,
            );
        }
        let mut open = None;
        match &self.result {
            None => {}
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            Some(Ok(hierarchy)) => {
                if let Some(cell) = hierarchy.cells.first() {
                    let line = ui
                        .selectable_label(false, RichText::new(&cell.name).strong())
                        .on_hover_text(cell.schematic.display().to_string());
                    if line.double_clicked() {
                        open = Some(cell.schematic.clone());
                    }
                    self.level(ui, hierarchy, &mut vec![0], &mut open);
                }
                if modified {
                    ui.add_space(6.0);
                    ui.weak("As saved: the open schematic has changes that are not saved.");
                }
            }
        }
        if let Some(file) = open {
            ctx.request(Action::OpenFile(file));
        }
    }
}

#[cfg(test)]
mod tests {
    use libcdept::xschem::{HierarchyCell, HierarchyInstance, Instance};

    use super::*;

    fn instance(name: &str, symbol: &str, kind: &str, child: Option<usize>) -> HierarchyInstance {
        HierarchyInstance {
            instance: Instance {
                name: name.into(),
                symbol: symbol.into(),
                kind: kind.into(),
            },
            child,
        }
    }

    fn cell(name: &str, instances: Vec<HierarchyInstance>) -> HierarchyCell {
        HierarchyCell {
            schematic: PathBuf::from(format!("/p/{name}.sch")),
            name: name.into(),
            instances,
        }
    }

    #[test]
    fn cells_used_twice_appear_under_each_instance() {
        let hierarchy = Hierarchy {
            cells: vec![
                cell(
                    "tb_buf",
                    vec![
                        instance("x1", "logic/buf.sym", "subcircuit", Some(1)),
                        instance("C1", "capa.sym", "capacitor", None),
                        instance("x9", "old/gone.sym", "subcircuit", None),
                    ],
                ),
                cell(
                    "buf",
                    vec![
                        instance("x1", "logic/inv.sym", "subcircuit", Some(2)),
                        instance("x2", "logic/inv.sym", "subcircuit", Some(2)),
                    ],
                ),
                cell("inv", vec![instance("M1", "nmos4.sym", "nmos", None)]),
            ],
        };
        assert_eq!(
            lines(&hierarchy, false),
            [
                "tb_buf",
                "  x1 buf",
                "    x1 inv",
                "    x2 inv",
                "  x9 old/gone.sym (no schematic)",
            ]
        );
        assert_eq!(
            lines(&hierarchy, true),
            [
                "tb_buf",
                "  x1 buf",
                "    x1 inv",
                "      M1 [nmos]",
                "    x2 inv",
                "      M1 [nmos]",
                "  C1 [capacitor]",
                "  x9 old/gone.sym (no schematic)",
            ]
        );
        assert!(lines(&Hierarchy::default(), true).is_empty());
    }

    #[test]
    fn a_schematic_containing_itself_is_not_unfolded_forever() {
        // a contains b, b contains a; c contains itself.
        let hierarchy = Hierarchy {
            cells: vec![
                cell(
                    "a",
                    vec![
                        instance("x1", "b.sym", "subcircuit", Some(1)),
                        instance("x2", "c.sym", "subcircuit", Some(2)),
                    ],
                ),
                cell("b", vec![instance("x1", "a.sym", "subcircuit", Some(0))]),
                cell("c", vec![instance("x1", "c.sym", "subcircuit", Some(2))]),
            ],
        };
        assert_eq!(
            lines(&hierarchy, false),
            [
                "a",
                "  x1 b",
                "    x1 a (contains itself)",
                "  x2 c",
                "    x1 c (contains itself)",
            ]
        );
    }

    #[test]
    fn a_wide_and_deep_design_stops_at_a_bound() {
        // Ten levels, each using the next one ten times: 10^10 lines unfolded.
        let cells = (0..10)
            .map(|level| {
                let below = (level < 9).then_some(level + 1);
                let instances = (0..10)
                    .map(|n| instance(&format!("x{n}"), "s.sym", "subcircuit", below))
                    .collect();
                cell(&format!("l{level}"), instances)
            })
            .collect();
        let all = lines(&Hierarchy { cells }, false);
        assert!(all.len() <= MAX_LINES + 10, "{}", all.len());
    }
}
