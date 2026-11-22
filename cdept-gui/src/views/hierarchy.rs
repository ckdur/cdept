//! Hierarchy panel: the cells a cell is built from, and the cells that use it.
//!
//! It follows the schematic document in front, else the cell selected in the library
//! views, unless a cell was asked for (Used By… in the library views).
//!
//! Two sources:
//! - the files ([`libcdept::hierarchy::Index`], read on a worker thread, no tool
//!   needed): the hierarchy through schematics, layouts, netlists or HDL views, one
//!   line per cell used with how often and through which instances; and the users of
//!   a cell;
//! - xschem (`libcdept::xschem::Engine::hierarchy`, in the background): the schematic
//!   hierarchy with one line per instance. Used for schematics while "Instances" is
//!   ticked; when xschem cannot do it, the files are shown instead.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use egui::{CollapsingHeader, RichText, Ui};
use libcdept::hierarchy::{Index, Node, NodeKind, Reference, TreeKind};
use libcdept::xschem::{Hierarchy, Pending};
use libcdept::{Format, Project};

use super::{View, empty_state};
use crate::state::{Action, AppCtx, Selection};

/// Levels below the top that start unfolded.
const OPEN_LEVELS: usize = 2;
/// The most lines [`lines`] gives: a cell used many times at several levels unfolds
/// into very many.
const MAX_LINES: usize = 2000;

/// What the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Schematic,
    Layout,
    Netlist,
    Hdl,
    /// The cells that use the cell.
    UsedBy,
}

impl Mode {
    const ALL: [Mode; 5] = [
        Mode::Schematic,
        Mode::Layout,
        Mode::Netlist,
        Mode::Hdl,
        Mode::UsedBy,
    ];

    fn label(self) -> &'static str {
        match self {
            Mode::Schematic => "Schematic",
            Mode::Layout => "Layout",
            Mode::Netlist => "Netlist",
            Mode::Hdl => "HDL",
            Mode::UsedBy => "Used by",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Schematic => "schematic",
            Mode::Layout => "layout",
            Mode::Netlist => "netlist",
            Mode::Hdl => "hdl",
            Mode::UsedBy => "used-by",
        }
    }

    fn tree(self) -> Option<TreeKind> {
        match self {
            Mode::Schematic => Some(TreeKind::Schematic),
            Mode::Layout => Some(TreeKind::Layout),
            Mode::Netlist => Some(TreeKind::Netlist),
            Mode::Hdl => Some(TreeKind::Hdl),
            Mode::UsedBy => None,
        }
    }
}

/// The references of the project, read on a worker thread.
#[derive(Default)]
struct Files {
    index: Option<Arc<Index>>,
    job: Option<Receiver<Index>>,
    /// What the index on show (or being read) was read for.
    stamp: Option<u64>,
    /// The index is older than the files (a save, an operation): read again.
    stale: bool,
}

pub struct HierarchyView {
    /// The schematic whose hierarchy is shown, or being worked out.
    top: Option<PathBuf>,
    job: Option<Pending<Hierarchy>>,
    result: Option<Result<Hierarchy, String>>,
    /// Also list what is not a subcircuit: transistors, pins, labels.
    all: bool,
    /// The document in front had unsaved changes when last looked at.
    was_modified: bool,
    mode: Mode,
    /// Schematics: one line per instance, from xschem.
    instances: bool,
    /// The cell on show when it was asked for; else the panel follows the document in
    /// front and the selection.
    pinned: Option<(String, String)>,
    files: Files,
    /// [`crate::refactor::Shared::epoch`] when the files were last read.
    epoch: u64,
    /// The lines last drawn from the files, for test scripts.
    shown: Vec<String>,
    /// The cells of the tree last drawn from the files, with their files.
    shown_files: Vec<(String, PathBuf)>,
}

impl Default for HierarchyView {
    fn default() -> Self {
        Self {
            top: None,
            job: None,
            result: None,
            all: false,
            was_modified: false,
            mode: Mode::default(),
            instances: true,
            pinned: None,
            files: Files::default(),
            epoch: 0,
            shown: Vec::new(),
            shown_files: Vec::new(),
        }
    }
}

/// Changes when cells or views come and go (not when a file's contents change).
fn stamp(project: &Project) -> u64 {
    let mut hasher = DefaultHasher::new();
    project.root.hash(&mut hasher);
    for lib in &project.libraries {
        lib.name.hash(&mut hasher);
        lib.path.hash(&mut hasher);
        lib.cells.len().hash(&mut hasher);
        // Large reference libraries: the cell count is enough.
        if lib.read_only {
            continue;
        }
        for cell in &lib.cells {
            cell.name.hash(&mut hasher);
            for entry in &cell.entries {
                entry.name.hash(&mut hasher);
                entry.files.len().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
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
            Some(Ok(hierarchy)) if self.shows_xschem() => lines(hierarchy, self.all),
            _ => self.shown.clone(),
        }
    }

    /// The schematic hierarchy comes from xschem (one line per instance).
    fn shows_xschem(&self) -> bool {
        self.mode == Mode::Schematic && self.instances && self.pinned.is_none()
    }

    /// Show a cell whatever is selected (`None`: follow the selection again).
    pub fn pin(&mut self, cell: Option<(String, String)>) {
        self.pinned = cell;
    }

    /// Set the mode by name, for test scripts: `schematic`, `layout`, `netlist`,
    /// `hdl`, `used-by`; `source`: `files` or `instances`.
    pub fn script_mode(&mut self, mode: &str, source: Option<&str>) -> Result<(), String> {
        self.mode = Mode::ALL
            .into_iter()
            .find(|m| m.name() == mode)
            .ok_or(format!("no hierarchy mode '{mode}'"))?;
        match source {
            None => {}
            Some("files") => self.instances = false,
            Some("instances") => self.instances = true,
            Some(other) => return Err(format!("'{other}' is not 'files' or 'instances'")),
        }
        Ok(())
    }

    /// The schematic of the first cell named `cell` in the tree on show.
    pub fn schematic_of(&self, cell: &str) -> Option<PathBuf> {
        if let (true, Some(Ok(hierarchy))) = (self.shows_xschem(), &self.result) {
            let found = hierarchy.cells.iter().find(|c| c.name == cell)?;
            return Some(found.schematic.clone());
        }
        let (_, file) = self.shown_files.iter().find(|(name, _)| name == cell)?;
        Some(file.clone())
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

/// The lines of a list of users: `logic/buf/schematic  instance ×2 (x1, x2)`.
pub fn user_lines(users: &[&Reference]) -> Vec<String> {
    users
        .iter()
        .map(|r| {
            let mut line = format!("{}  {} ×{}", r.from, r.kind.name(), r.count);
            if !r.instances.is_empty() {
                line.push_str(&format!(" ({})", r.instances.join(", ")));
            }
            line
        })
        .collect()
}

impl HierarchyView {
    /// The cell to show: the one asked for, else the document in front, else the
    /// selection.
    fn wanted_cell(&self, ctx: &AppCtx) -> Option<(String, String)> {
        if let Some(pinned) = &self.pinned {
            return Some(pinned.clone());
        }
        if let Some(active) = &ctx.schematic
            && !active.doc.library.is_empty()
        {
            return Some((active.doc.library.clone(), active.doc.cell.clone()));
        }
        Some((ctx.selection.library.clone()?, ctx.selection.cell.clone()?))
    }

    /// Keep the index of the files: read it when there is none, when cells or views
    /// came or went, after a save and after an operation. `force`: read it now.
    fn keep_files(&mut self, ctx: &AppCtx, egui: &egui::Context, force: bool) {
        let Some(project) = &ctx.project else {
            self.files = Files::default();
            return;
        };
        if let Some(job) = &self.files.job {
            match job.try_recv() {
                Ok(index) => {
                    for warning in &index.warnings {
                        tracing::debug!(target: "cdept::library", "hierarchy: {warning}");
                    }
                    self.files.index = Some(Arc::new(index));
                    self.files.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.files.job = None,
            }
        }
        let now = stamp(project);
        if self.epoch != ctx.refactor.epoch {
            self.epoch = ctx.refactor.epoch;
            self.files.stale = true;
        }
        let outdated = self.files.stamp != Some(now) || self.files.stale || force;
        if !outdated || self.files.job.is_some() {
            return;
        }
        self.files.stamp = Some(now);
        self.files.stale = false;
        let (sender, receiver) = mpsc::channel();
        let project = project.clone();
        let egui = egui.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Index::build(&project));
            egui.request_repaint();
        });
        self.files.job = Some(receiver);
    }

    /// One level of the tree read from the files.
    fn node(&self, ui: &mut Ui, node: &Node, depth: usize, open: &mut Option<PathBuf>) {
        for (n, child) in node.children.iter().enumerate() {
            let text = child.label();
            match child.kind {
                NodeKind::Primitive if !self.all => {}
                NodeKind::Primitive | NodeKind::Recursive => {
                    ui.weak(text);
                }
                NodeKind::Missing => {
                    ui.colored_label(ui.visuals().warn_fg_color, text);
                }
                NodeKind::Cell => {
                    let has_level = child
                        .children
                        .iter()
                        .any(|c| self.all || c.kind != NodeKind::Primitive);
                    let response = if has_level {
                        CollapsingHeader::new(text)
                            .id_salt(("files", depth, n))
                            .default_open(depth <= OPEN_LEVELS)
                            .show(ui, |ui| self.node(ui, child, depth + 1, open))
                            .header_response
                    } else {
                        ui.horizontal(|ui| {
                            ui.add_space(ui.spacing().indent);
                            ui.selectable_label(false, text)
                        })
                        .inner
                    };
                    let hover = match &child.view {
                        Some(view) => format!("{view}\nDouble-click to open it"),
                        None => "Double-click to open it".to_string(),
                    };
                    if response.on_hover_text(hover).double_clicked() {
                        *open = child.file.clone();
                    }
                }
            }
        }
    }

    /// The hierarchy of a cell, from the files.
    fn files_tree(
        &mut self,
        ui: &mut Ui,
        ctx: &mut AppCtx,
        index: &Index,
        cell: &(String, String),
        kind: TreeKind,
    ) {
        let Some(project) = &ctx.project else {
            return;
        };
        let Some(tree) = index.tree(project, &cell.0, &cell.1, kind) else {
            self.shown.clear();
            self.shown_files.clear();
            return empty_state(
                ui,
                &format!("{} / {} has no {} view.", cell.0, cell.1, kind.name()),
                None,
            );
        };
        self.shown = tree.lines(self.all);
        self.shown_files.clear();
        fn collect(node: &Node, out: &mut Vec<(String, PathBuf)>) {
            if let (NodeKind::Cell, Some(file)) = (node.kind, &node.file) {
                out.push((node.name.clone(), file.clone()));
            }
            for child in &node.children {
                collect(child, out);
            }
        }
        collect(&tree, &mut self.shown_files);
        let mut open = None;
        let top = ui
            .selectable_label(false, RichText::new(&tree.name).strong())
            .on_hover_text(match &tree.view {
                Some(view) => view.label(),
                None => String::new(),
            });
        if top.double_clicked() {
            open = tree.file.clone();
        }
        self.node(ui, &tree, 1, &mut open);
        if tree.size() >= libcdept::hierarchy::MAX_NODES {
            ui.weak("The hierarchy is larger than what is shown.");
        }
        if let Some(file) = open {
            ctx.request(Action::OpenFile(file));
        }
    }

    /// The users of a cell.
    fn users(&mut self, ui: &mut Ui, ctx: &mut AppCtx, index: &Index, cell: &(String, String)) {
        let mut users = index.used_by(&cell.0, &cell.1, None);
        users.sort_by(|a, b| (&a.from, a.kind).cmp(&(&b.from, b.kind)));
        self.shown = user_lines(&users);
        self.shown_files.clear();
        ui.label(RichText::new(format!("{} / {} is used by", cell.0, cell.1)).strong());
        if users.is_empty() {
            ui.weak("nothing in the project.");
            return;
        }
        for (r, text) in users.iter().zip(&self.shown) {
            let selected =
                ctx.selection == Selection::view(&r.from.library, &r.from.cell, &r.from.view);
            let mut hover = format!(
                "{}\n{} as written: {}",
                r.file.display(),
                r.kind.name(),
                r.text
            );
            if r.read_only {
                hover.push_str("\nIn a read-only library: it cannot follow a rename.");
            }
            hover.push_str("\nClick to select it, double-click to open it");
            let line = ui.selectable_label(selected, text).on_hover_text(hover);
            if line.clicked() {
                ctx.selection = Selection::view(&r.from.library, &r.from.cell, &r.from.view);
            }
            if line.double_clicked() {
                ctx.request(Action::OpenFile(r.file.clone()));
            }
        }
    }

    /// The schematic hierarchy from xschem, one line per instance. Returns false when
    /// xschem could not do it (the files are shown then).
    fn xschem_tree(
        &mut self,
        ui: &mut Ui,
        ctx: &mut AppCtx,
        top: Option<PathBuf>,
        refresh: bool,
    ) -> bool {
        let modified = ctx.schematic.as_ref().is_some_and(|s| s.modified);
        if refresh || top != self.top {
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
            empty_state(
                ui,
                "Open a schematic, or select a cell, to see its hierarchy.",
                None,
            );
            return true;
        }
        let mut open = None;
        match &self.result {
            None => {}
            Some(Err(_)) => return false,
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
        true
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
                mode: self.mode,
                instances: self.instances,
                ..Default::default()
            };
            return empty_state(ui, "No project open.", None);
        }
        // Asked for from the library views: Used By…
        if let Some(show) = ctx.refactor.show.take() {
            self.pinned = Some((show.library, show.cell));
            if show.used_by {
                self.mode = Mode::UsedBy;
            }
        }
        let top = wanted_top(ctx);
        let cell = self.wanted_cell(ctx);

        // Read again after a save.
        let modified = ctx.schematic.as_ref().is_some_and(|s| s.modified);
        let saved = std::mem::replace(&mut self.was_modified, modified) && !modified;
        let mut refresh = saved;
        ui.horizontal_wrapped(|ui| {
            let button = ui
                .button("⟳")
                .on_hover_text("Read the hierarchy again from the files on disk");
            refresh |= button.clicked();
            egui::ComboBox::from_id_salt("hierarchy-mode")
                .selected_text(self.mode.label())
                .show_ui(ui, |ui| {
                    for mode in Mode::ALL {
                        ui.selectable_value(&mut self.mode, mode, mode.label());
                    }
                });
            if self.mode != Mode::UsedBy {
                ui.checkbox(&mut self.all, "All")
                    .on_hover_text("Also list transistors, pins and other primitives");
            }
            if self.mode == Mode::Schematic && self.pinned.is_none() {
                ui.checkbox(&mut self.instances, "Instances").on_hover_text(
                    "One line per instance, worked out by xschem. Without: one line per \
                     cell used, read from the files.",
                );
            }
            if self.job.is_some() || self.files.job.is_some() {
                ui.spinner();
                ui.weak("working…");
            }
        });
        if let Some((library, name)) = &self.pinned {
            let mut unpin = false;
            ui.horizontal(|ui| {
                ui.label(format!("{library} / {name}"));
                unpin = ui
                    .small_button("✖")
                    .on_hover_text("Follow the open document and the selection again")
                    .clicked();
            });
            if unpin {
                self.pinned = None;
            }
        }
        ui.separator();
        self.keep_files(ctx, ui.ctx(), refresh);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.shows_xschem() && self.xschem_tree(ui, ctx, top, refresh) {
                    return;
                }
                let Some(cell) = cell else {
                    self.shown.clear();
                    return empty_state(
                        ui,
                        "Open a document, or select a cell, to see its hierarchy.",
                        None,
                    );
                };
                let Some(index) = self.files.index.clone() else {
                    ui.weak("reading the files…");
                    return;
                };
                match self.mode.tree() {
                    Some(kind) => {
                        if self.shows_xschem()
                            && let Some(Err(error)) = &self.result
                        {
                            ui.weak(format!("xschem: {error}"));
                            ui.weak("From the files:");
                        }
                        self.files_tree(ui, ctx, &index, &cell, kind);
                    }
                    None => self.users(ui, ctx, &index, &cell),
                }
            });
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
