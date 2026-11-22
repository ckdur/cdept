//! File → Import Project…: look at an existing design folder, show the project CDePT
//! would make of it as a checklist, write it. Also where the shell meets what
//! organises a project: the per-frame work of the quick-open box and the categories,
//! the menu entries, the test-script commands (`organise …`).
//!
//! The scan (`libcdept::import::scan`) runs behind the window: it may start xschem to
//! read an `xschemrc` and walk a large folder.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Context, Id, RichText, ScrollArea, Ui};
use libcdept::import::{self, Options, Plan};
use libcdept::{PROJECT_FILE, Project};

use crate::dock::Tab;
use crate::state::{Action, AppCtx};
use crate::views::{categories, search};

type Scan = Arc<Mutex<Option<Result<Plan, String>>>>;

/// The Import Project window.
#[derive(Clone, Default)]
struct ImportDialog {
    open: bool,
    folder: String,
    /// What the plan (or the running scan) is for.
    scanned: String,
    scan: Option<Scan>,
    plan: Option<Plan>,
    error: Option<String>,
}

fn id() -> Id {
    Id::new("cdept-import-dialog")
}

fn load(ctx: &Context) -> ImportDialog {
    ctx.data(|data| data.get_temp::<ImportDialog>(id()))
        .unwrap_or_default()
}

fn store(ctx: &Context, dialog: ImportDialog) {
    ctx.data_mut(|data| data.insert_temp(id(), dialog));
}

/// Open the Import Project window, with `folder` filled in when given.
pub(crate) fn open(ctx: &Context, folder: Option<&str>) {
    let mut dialog = ImportDialog {
        open: true,
        ..load(ctx)
    };
    if let Some(folder) = folder {
        dialog.folder = folder.to_string();
    }
    store(ctx, dialog);
    ctx.request_repaint();
}

fn start_scan(ctx: &Context, dialog: &mut ImportDialog) {
    let folder = PathBuf::from(dialog.folder.trim());
    dialog.scanned = dialog.folder.trim().to_string();
    dialog.plan = None;
    dialog.error = None;
    let scan: Scan = Arc::default();
    dialog.scan = Some(scan.clone());
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = import::scan(&folder, &Options::default()).map_err(|e| format!("{e:#}"));
        *scan.lock().unwrap() = Some(result);
        ctx.request_repaint();
    });
}

/// The entry of the File menu.
pub(crate) fn file_menu(ui: &mut Ui) {
    if ui
        .button("Import Project…")
        .on_hover_text(
            "Make a project for an existing design folder: its libraries are found from \
             the xschemrc, the folders with design files or a LibMan project file.",
        )
        .clicked()
    {
        open(ui.ctx(), None);
    }
}

/// The entries of the Tools menu.
pub(crate) fn tools_menu(ui: &mut Ui, app: &mut AppCtx) {
    let has_project = app.project.is_some();
    let shortcut = ui.ctx().format_shortcut(&search::FIND);
    if ui
        .add_enabled(
            has_project,
            egui::Button::new("Go to Cell…").shortcut_text(shortcut),
        )
        .on_hover_text("Find a cell by name and jump to it (also Ctrl+P outside schematics).")
        .clicked()
    {
        search::open_palette(ui.ctx(), "");
    }
    if ui
        .button("Search Libraries")
        .on_hover_text("The Search panel: filters, and looking inside the files.")
        .clicked()
    {
        app.request(Action::OpenTab(Tab::Search));
    }
    let libman = app.tools.find("LibMan").filter(|tool| tool.found());
    if ui
        .add_enabled(
            has_project && libman.is_some(),
            egui::Button::new("Open in LibMan"),
        )
        .on_hover_text("Start IHP LibMan on the libraries of this project.")
        .on_disabled_hover_text("LibMan (libman or door) was not found on PATH.")
        .clicked()
        && let Some(project) = &app.project
    {
        match import::libman::open(project) {
            Ok(file) => tracing::info!(
                target: "cdept::tools",
                "LibMan started on {}",
                file.display()
            ),
            Err(error) => app.error(format!("{error:#}")),
        }
    }
}

/// Once per frame, before the windows are drawn.
pub(crate) fn frame(ctx: &Context, app: &mut AppCtx) {
    search::frame(ctx, app);
    categories::frame(ctx, app);
    window(ctx, app);
}

/// What to do with a finished plan.
enum Finish {
    Create,
    Add,
}

fn window(ctx: &Context, app: &mut AppCtx) {
    let mut dialog = load(ctx);
    if !dialog.open {
        return;
    }
    if let Some(scan) = &dialog.scan {
        let done = scan.lock().unwrap().take();
        match done {
            Some(Ok(plan)) => {
                dialog.plan = Some(plan);
                dialog.scan = None;
            }
            Some(Err(error)) => {
                dialog.error = Some(error);
                dialog.scan = None;
            }
            None => {}
        }
    }
    let mut open = true;
    let mut finish: Option<Finish> = None;
    egui::Window::new("Import Project")
        .id(Id::new("cdept-import-window"))
        .open(&mut open)
        .collapsible(false)
        .default_size([760.0, 520.0])
        .show(ctx, |ui| {
            ui.label(
                "Choose a design folder that has no CDePT project yet. Nothing in it is \
                 moved or changed; a cdept.toml is added.",
            );
            let mut go = false;
            egui::Grid::new("import-folder")
                .num_columns(2)
                .show(ui, |ui| {
                    let changed =
                        crate::pathfield::folder_row(ui, "Folder", &mut dialog.folder, None);
                    let _ = changed;
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                let ready = !dialog.folder.trim().is_empty() && dialog.scan.is_none();
                go |= ui
                    .add_enabled(ready, egui::Button::new("Scan"))
                    .on_hover_text("Look for libraries in the folder.")
                    .clicked();
                if dialog.scan.is_some() {
                    ui.spinner();
                    ui.weak("Looking at the folder…");
                }
            });
            if go {
                start_scan(ui.ctx(), &mut dialog);
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let stale = dialog.scanned != dialog.folder.trim();
            let Some(plan) = &mut dialog.plan else {
                return;
            };
            ui.separator();
            if stale {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "This is the plan for {}. Scan again for the folder above.",
                        plan.source.display()
                    ),
                );
            }
            finish = plan_ui(ui, plan, app.project.as_ref());
        });
    if !open {
        dialog.open = false;
    }
    match (finish, dialog.plan.clone()) {
        (Some(Finish::Create), Some(plan)) => match plan.write() {
            Ok(project) => {
                tracing::info!(
                    target: "cdept::library",
                    "imported {}: {} libraries",
                    project.file.display(),
                    project.libraries.len()
                );
                app.request(Action::LoadProject(project.root.clone()));
                dialog = ImportDialog::default();
            }
            Err(error) => dialog.error = Some(format!("{error:#}")),
        },
        (Some(Finish::Add), Some(plan)) => {
            let result = match &mut app.project {
                Some(project) => plan.add_to(project).map_err(|e| format!("{e:#}")),
                None => Err("no project is open".to_string()),
            };
            match result {
                Ok(added) => {
                    tracing::info!(
                        target: "cdept::library",
                        "libraries added: {}",
                        if added.is_empty() { "none (the project has them already)".into() } else { added.join(", ") }
                    );
                    app.request(Action::ReloadProject);
                    dialog = ImportDialog::default();
                }
                Err(error) => dialog.error = Some(error),
            }
        }
        _ => {}
    }
    store(ctx, dialog);
}

/// The plan as a form. Returns what the user chose to do with it.
fn plan_ui(ui: &mut Ui, plan: &mut Plan, project: Option<&Project>) -> Option<Finish> {
    let mut finish = None;
    egui::Grid::new("import-plan")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Project name");
            ui.add(egui::TextEdit::singleline(&mut plan.name).desired_width(240.0));
            ui.end_row();
            ui.label("Project folder");
            ui.label(plan.root.display().to_string());
            ui.end_row();
            if let Some(technology) = &plan.technology {
                ui.label("Technology");
                let root = plan
                    .pdk_root
                    .as_ref()
                    .map(|r| format!("  (PDK folder {})", r.display()))
                    .unwrap_or_default();
                ui.label(format!("{technology}{root}"));
                ui.end_row();
            }
            if let Some(rc) = &plan.xschem_rc {
                ui.label("xschem start-up file");
                ui.label(plan.path_text(rc));
                ui.end_row();
            }
            if let Some(file) = &plan.klayout_tech {
                ui.label("KLayout technology");
                ui.label(plan.path_text(file));
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    let listed = plan.enabled().count();
    ui.horizontal(|ui| {
        ui.strong(format!("Libraries ({listed} of {})", plan.libraries.len()));
        if ui.small_button("All").clicked() {
            plan.libraries.iter_mut().for_each(|l| l.enabled = true);
        }
        if ui.small_button("None").clicked() {
            plan.libraries.iter_mut().for_each(|l| l.enabled = false);
        }
    });
    let paths: Vec<String> = plan
        .libraries
        .iter()
        .map(|l| plan.path_text(&l.folder))
        .collect();
    let height = (ui.available_height() - 150.0).max(120.0);
    ScrollArea::both()
        .id_salt("import-libraries")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("import-libraries-grid")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    for heading in ["", "Name", "Layout", "Folder", "Contents"] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for (library, path) in plan.libraries.iter_mut().zip(&paths) {
                        ui.checkbox(&mut library.enabled, "");
                        ui.add_enabled(
                            library.enabled,
                            egui::TextEdit::singleline(&mut library.name)
                                .min_size(egui::vec2(150.0, 0.0))
                                .desired_width(150.0),
                        );
                        ui.label(library.layout.name());
                        ui.label(path)
                            .on_hover_text(library.folder.display().to_string());
                        let mut contents = library.summary();
                        if library.read_only {
                            contents.push_str(", read-only");
                        }
                        contents.push_str(&format!(", from {}", library.origin.label()));
                        let label = ui.label(contents);
                        if let Some(note) = &library.note {
                            label.on_hover_text(note);
                            ui.weak(note);
                        }
                        ui.end_row();
                    }
                });
        });
    if !plan.skipped.is_empty() {
        ui.collapsing(format!("Left out ({})", plan.skipped.len()), |ui| {
            for skipped in &plan.skipped {
                ui.label(format!(
                    "{}  ({})",
                    plan.path_text(&skipped.folder),
                    skipped.reason
                ));
            }
        });
    }
    if !plan.notes.is_empty() {
        ui.collapsing(format!("Notes ({})", plan.notes.len()), |ui| {
            for note in &plan.notes {
                ui.label(RichText::new(format!("• {note}")).small());
            }
        });
    }
    let problems = plan.problems();
    for problem in &problems {
        ui.colored_label(ui.visuals().warn_fg_color, problem);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let exists = plan.root.join(PROJECT_FILE).is_file();
        let ok = problems.is_empty() && listed > 0;
        if exists {
            // The folder has a project: its libraries can be added to it when it is
            // the one that is open.
            let same = project
                .is_some_and(|p| p.root.canonicalize().ok() == plan.root.canonicalize().ok());
            if ui
                .add_enabled(ok && same, egui::Button::new("Add Libraries to Project"))
                .on_disabled_hover_text(
                    "The folder has a project already. Open it, then its missing \
                     libraries can be added from here.",
                )
                .clicked()
            {
                finish = Some(Finish::Add);
            }
        } else if ui
            .add_enabled(ok, egui::Button::new("Create Project"))
            .on_hover_text(format!(
                "Write {} and open the project.",
                plan.root.join(PROJECT_FILE).display()
            ))
            .clicked()
        {
            finish = Some(Finish::Create);
        }
    });
    finish
}

/// `organise …` of a test script.
///
/// - `find <query>`, `go`, `open`, `expect_selection <library> <cell> [view]`: the
///   quick-open box (see [`search::script`]);
/// - `category <verb> …`: categories (see [`categories::script`]);
/// - `import <folder>`: open the Import window and scan the folder;
///   `import_expect <n>`: the plan lists n libraries (waits for the scan);
///   `import_skip <name>`: untick one; `import_create`: write the project and open it.
pub(crate) fn script(app: &mut AppCtx, ctx: &Context, args: &[String]) -> Result<(), String> {
    let (verb, rest) = args.split_first().ok_or("usage: organise <verb> …")?;
    match verb.as_str() {
        "category" => categories::script(ctx, app, rest),
        "import" => {
            let folder = rest.join(" ");
            let mut dialog = ImportDialog {
                open: true,
                folder,
                ..ImportDialog::default()
            };
            start_scan(ctx, &mut dialog);
            store(ctx, dialog);
            Ok(())
        }
        "import_expect" => {
            let dialog = load(ctx);
            if let Some(error) = &dialog.error {
                return Err(format!("the scan failed: {error}"));
            }
            let plan = dialog.plan.ok_or("the folder is still being scanned")?;
            let wanted: usize = rest
                .first()
                .and_then(|w| w.parse().ok())
                .ok_or("usage: organise import_expect <n>")?;
            let listed = plan.enabled().count();
            if listed != wanted {
                return Err(format!(
                    "the plan lists {listed} libraries, expected {wanted}:\n{}",
                    plan.describe()
                ));
            }
            Ok(())
        }
        "import_skip" => {
            let mut dialog = load(ctx);
            let name = rest.first().ok_or("usage: organise import_skip <name>")?;
            let plan = dialog.plan.as_mut().ok_or("there is no plan")?;
            let library = plan
                .libraries
                .iter_mut()
                .find(|l| &l.name == name)
                .ok_or(format!("the plan has no library '{name}'"))?;
            library.enabled = false;
            store(ctx, dialog);
            Ok(())
        }
        "import_create" => {
            let dialog = load(ctx);
            let plan = dialog.plan.ok_or("there is no plan")?;
            let project = plan.write().map_err(|e| format!("{e:#}"))?;
            app.request(Action::LoadProject(project.root.clone()));
            store(ctx, ImportDialog::default());
            Ok(())
        }
        _ => search::script(ctx, app, args),
    }
}
