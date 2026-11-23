//! The New Project wizard, and the PDK chooser of the Project Settings.
//!
//! File > New Project… asks for a folder and a PDK on one page, scans the PDK in the
//! background, shows what it found and creates the project with its technology
//! snapshot (`tech.toml`, see `libcdept::tech`). With the defaults accepted that is
//! three clicks: New Project…, Next, Create. "No PDK" creates a bare project as before.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Grid, Key, RichText, ScrollArea, Ui};
use libcdept::tech::discover;
use libcdept::tech::{self, Catalog, PdkDir, Tech, TechLibrary, TechPath};
use libcdept::{PROJECT_FILE, Project};

use crate::dock::WindowId;
use crate::pathfield::PathField;
use crate::pdkscan::{self, Job, PdkEntry, ScanJob};
use crate::state::{Action, AppCtx};
use crate::views::technology::{self as techview, Ask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// Folder, name and PDK.
    Project,
    /// The scan and what it found.
    Technology,
}

/// The result of the scan the wizard ran.
struct Scanned {
    pdk: PdkDir,
    tech: Tech,
    catalog: Catalog,
    summary: Vec<(String, String)>,
    time: Duration,
}

pub struct Wizard {
    page: Page,
    folder: String,
    name: String,
    finding: Option<Job<Vec<PdkEntry>>>,
    pdks: Vec<PdkEntry>,
    /// The PDK chosen (an index into `pdks`); `None` is "No PDK".
    choice: Option<usize>,
    /// The PDK to choose once the list is there; else the first one.
    prefer: Option<String>,
    /// A folder typed in to look for PDKs.
    other: String,
    /// Scan without the helper programs.
    files_only: bool,
    scan: Option<ScanJob>,
    scanned: Option<Scanned>,
    error: Option<String>,
    /// The folder field still has to take the keyboard focus.
    focus: bool,
}

/// What the wizard was told to do in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Open,
    Cancel,
    Next,
    Back,
    Create,
}

/// A folder next to `parent` that does not exist yet: `project`, `project2`, …
fn free_folder(parent: &Path) -> PathBuf {
    (1..)
        .map(|n| match n {
            1 => parent.join("project"),
            n => parent.join(format!("project{n}")),
        })
        .find(|dir| !dir.exists())
        .expect("some number is free")
}

/// The folder a typed text means: `~` and `$VAR` expanded like the path field does.
fn expand_home(text: &str) -> PathBuf {
    crate::pathfield::resolve(text, None, None).unwrap_or_else(|| PathBuf::from(text.trim()))
}

impl Wizard {
    fn new(app: &AppCtx, ctx: &egui::Context) -> Wizard {
        // Next to the open project, else in the folder CDePT was started in.
        let parent = app
            .project
            .as_ref()
            .and_then(|p| p.root.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let prefer = app.pdk().or_else(|| {
            let offered = std::env::var("PDK").ok()?;
            discover::names(&offered).into_iter().next()
        });
        let search = pdkscan::search(app);
        Wizard {
            page: Page::Project,
            folder: free_folder(&parent).to_string_lossy().into_owned(),
            name: String::new(),
            finding: Some(Job::start(ctx, move || pdkscan::find_pdks(&search))),
            pdks: Vec::new(),
            choice: None,
            prefer,
            other: String::new(),
            files_only: false,
            scan: None,
            scanned: None,
            error: None,
            focus: true,
        }
    }

    fn dir(&self) -> PathBuf {
        let dir = expand_home(&self.folder);
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        // `a/../b` reads `b` (only as text: the folder may not exist yet).
        let mut tidy = PathBuf::new();
        for part in dir.components() {
            match part {
                std::path::Component::ParentDir => {
                    tidy.pop();
                }
                std::path::Component::CurDir => {}
                other => tidy.push(other),
            }
        }
        tidy
    }

    fn chosen(&self) -> Option<&PdkEntry> {
        self.pdks.get(self.choice?)
    }

    /// The result of the scan, when it is the chosen PDK's.
    fn scanned(&self) -> Option<&Scanned> {
        let chosen = self.chosen()?;
        self.scanned.as_ref().filter(|s| s.pdk == *chosen.pdk())
    }

    /// Add PDKs to the list (those not in it yet); returns the place of the first.
    fn add_pdks(&mut self, found: Vec<PdkEntry>) -> Option<usize> {
        let mut first = None;
        for entry in found {
            let at = match self
                .pdks
                .iter()
                .position(|known| known.found.real == entry.found.real)
            {
                Some(at) => at,
                None => {
                    self.pdks.push(entry);
                    self.pdks.len() - 1
                }
            };
            first.get_or_insert(at);
        }
        first
    }

    /// Look for PDKs in a folder the user named, and choose the first found there.
    fn add_folder(&mut self, app: &mut AppCtx, folder: &Path) -> Result<(), String> {
        let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
        if !folder.is_dir() {
            return Err(format!("{} is not a folder", folder.display()));
        }
        let pdks = discover::in_folder(&folder);
        if pdks.is_empty() {
            return Err(format!(
                "no PDK in {}: a PDK is a folder with libs.tech in it",
                folder.display()
            ));
        }
        // Remember the folder that holds them, for the next time.
        for root in pdks.iter().map(|pdk| pdk.root.clone()) {
            if !app.settings.pdk_folders.contains(&root) {
                app.settings.pdk_folders.push(root);
            }
        }
        let entries = pdks
            .into_iter()
            .map(|pdk| {
                let real = std::fs::canonicalize(pdk.dir()).unwrap_or_else(|_| pdk.dir());
                let found = discover::Found {
                    pdk,
                    origin: discover::Origin::Given,
                    real,
                };
                PdkEntry {
                    info: found.info(),
                    found,
                }
            })
            .collect();
        self.choice = self.add_pdks(entries);
        self.error = None;
        Ok(())
    }

    /// Take what the background jobs have ready.
    fn poll(&mut self) {
        if let Some(job) = &mut self.finding
            && let Some(found) = job.poll()
        {
            self.finding = None;
            let chosen_before = self.chosen().map(|e| e.found.real.clone());
            // What was found goes first, what the user added stays after it.
            let added = std::mem::take(&mut self.pdks);
            self.add_pdks(found);
            self.add_pdks(added);
            self.choice = match chosen_before {
                Some(real) => self.pdks.iter().position(|e| e.found.real == real),
                None => self
                    .prefer
                    .as_ref()
                    .and_then(|name| self.pdks.iter().position(|e| e.pdk().name == *name))
                    .or(if self.pdks.is_empty() { None } else { Some(0) }),
            };
        }
        if let Some(job) = &mut self.scan
            && let Some((tech, catalog)) = job.poll()
        {
            let job = self.scan.take().expect("checked");
            self.scanned = Some(Scanned {
                summary: techview::summary(&tech, Some(&catalog)),
                pdk: job.pdk.clone(),
                tech,
                catalog,
                time: job.elapsed(),
            });
        }
    }

    /// Start scanning the chosen PDK (unless its result is there already).
    fn start_scan(&mut self, ctx: &egui::Context) {
        let Some(pdk) = self.chosen().map(|chosen| chosen.pdk().clone()) else {
            return;
        };
        if self.scanned().is_some() || self.scan.as_ref().is_some_and(|job| job.pdk == pdk) {
            return;
        }
        self.scanned = None;
        self.scan = Some(ScanJob::start(ctx, pdk, !self.files_only, Vec::new()));
    }

    /// What is wrong with the folder, if anything stops the wizard.
    fn folder_problem(&self) -> Option<String> {
        if self.folder.trim().is_empty() {
            return Some("Name the folder of the project.".into());
        }
        let dir = self.dir();
        if dir.is_file() {
            return Some(format!("{} is a file.", dir.display()));
        }
        None
    }

    fn holds_project(&self) -> bool {
        self.dir().join(PROJECT_FILE).is_file()
    }

    /// Create the project on disk and ask for it to be opened.
    fn create(&mut self, app: &mut AppCtx) -> Result<(), String> {
        if let Some(problem) = self.folder_problem() {
            return Err(problem);
        }
        let dir = self.dir();
        if !self.holds_project() {
            let name = Some(self.name.trim()).filter(|name| !name.is_empty());
            let created = match (self.choice, self.scanned()) {
                (None, _) => Project::create(&dir, name),
                (Some(_), Some(scanned)) => Project::create_for_pdk(
                    &dir,
                    name,
                    &scanned.pdk,
                    &scanned.tech,
                    &scanned.catalog,
                ),
                (Some(_), None) => return Err("The PDK is not scanned yet.".into()),
            };
            let project = created.map_err(|e| format!("{e:#}"))?;
            tracing::info!(
                target: "cdept::project",
                "created project '{}' in {}{}",
                project.name,
                dir.display(),
                match self.scanned() {
                    Some(scanned) if self.choice.is_some() =>
                        format!(" for the PDK {}", scanned.pdk.name),
                    _ => String::new(),
                }
            );
        }
        // Opening goes through the question about unsaved documents.
        app.request(Action::LoadProject(dir));
        Ok(())
    }

    fn project_page(&mut self, ui: &mut Ui, app: &mut AppCtx) -> Outcome {
        let mut outcome = Outcome::Open;
        Grid::new("wizard-project")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Folder");
                let focus = std::mem::take(&mut self.focus);
                // Completion and "Browse…" come with the field (pathfield.rs).
                ui.vertical(|ui| {
                    PathField::folder("wizard-folder", &mut self.folder)
                        .title("Folder for the new project")
                        .width(420.0)
                        .focus(focus)
                        .show(ui)
                });
                ui.end_row();
                ui.label("Name");
                let folder_name = self
                    .dir()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.add(
                    egui::TextEdit::singleline(&mut self.name)
                        .hint_text(folder_name)
                        .desired_width(420.0),
                );
                ui.end_row();
            });
        let opens = self.holds_project();
        match self.folder_problem() {
            Some(problem) => ui.colored_label(ui.visuals().warn_fg_color, problem),
            None if opens => ui.colored_label(
                ui.visuals().warn_fg_color,
                "This folder already holds a project: it is opened as it is.",
            ),
            None if self.dir().is_dir() => {
                ui.small("The folder exists: the project file (cdept.toml) is added to it.")
            }
            None => ui.small("The folder is created."),
        };

        ui.add_space(10.0);
        ui.strong("Technology (PDK)");
        if self.finding.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Looking for PDKs…");
            });
        }
        ScrollArea::vertical()
            .max_height(220.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                Grid::new("wizard-pdks")
                    .num_columns(4)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        for (n, entry) in self.pdks.iter().enumerate() {
                            ui.radio_value(&mut self.choice, Some(n), &entry.pdk().name)
                                .on_hover_text(entry.found.real.display().to_string());
                            ui.label(entry.pdk().family.name());
                            ui.label(entry.version());
                            ui.weak(entry.pdk().root.display().to_string());
                            ui.end_row();
                        }
                        ui.radio_value(&mut self.choice, None, "No PDK");
                        ui.weak("");
                        ui.weak("");
                        ui.weak("choose one later in Project > Project Settings…");
                        ui.end_row();
                    });
            });
        if self.finding.is_none() && self.pdks.is_empty() {
            ui.small(
                "No PDK was found. Looked at: $PDK_ROOT, $CDEPT_PDK_PATH, ~/.ciel, ~/.volare, \
                 /usr/share/pdk, /usr/local/share/pdk, ~/share/pdk. Name the folder below.",
            );
        }
        ui.horizontal(|ui| {
            ui.label("Other folder")
                .on_hover_text("A PDK folder, or the folder that holds PDKs");
            let field = ui
                .vertical(|ui| {
                    PathField::folder("wizard-pdk-folder", &mut self.other)
                        .title("A PDK, or the folder that holds PDKs")
                        .width(300.0)
                        .show(ui)
                })
                .inner;
            let named = !self.other.trim().is_empty();
            let entered = field.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            // A folder that was browsed to (not typed) and holds PDKs is added at once.
            let browsed = field.changed
                && !field.response.has_focus()
                && !discover::in_folder(&expand_home(&self.other)).is_empty();
            let add = ui
                .add_enabled(named, egui::Button::new("Add"))
                .on_hover_text("Look for PDKs in this folder")
                .clicked();
            if (entered || add || browsed) && named {
                let folder = expand_home(&self.other);
                match self.add_folder(app, &folder) {
                    Ok(()) => self.other.clear(),
                    Err(e) => self.error = Some(e),
                }
                // Enter was for the folder, not for the wizard.
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
            }
        });
        if self.choice.is_some() {
            ui.checkbox(&mut self.files_only, "Scan files only")
                .on_hover_text(
                    "Do not start KLayout and xschem for the scan: faster, but PCells and \
                     what the xschemrc sets are not found",
                );
        }

        ui.add_space(10.0);
        let ready = self.folder_problem().is_none();
        ui.horizontal(|ui| {
            if opens || self.choice.is_none() {
                let label = if opens { "Open" } else { "Create" };
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    outcome = Outcome::Create;
                }
            } else if ui
                .add_enabled(ready, egui::Button::new("Next ›"))
                .on_hover_text("Scan the PDK and show what was found")
                .clicked()
            {
                outcome = Outcome::Next;
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Cancel;
            }
        });
        if ready && outcome == Outcome::Open && ui.input(|i| i.key_pressed(Key::Enter)) {
            outcome = if opens || self.choice.is_none() {
                Outcome::Create
            } else {
                Outcome::Next
            };
        }
        outcome
    }

    fn technology_page(&mut self, ui: &mut Ui) -> Outcome {
        let mut outcome = Outcome::Open;
        let Some(chosen) = self.chosen().cloned() else {
            return Outcome::Back;
        };
        ui.horizontal(|ui| {
            ui.strong(&chosen.pdk().name);
            ui.weak(chosen.pdk().root.display().to_string());
        });
        ui.add_space(6.0);
        if let Some(job) = &self.scan {
            ui.label("Reading the PDK…");
            pdkscan::progress_ui(ui, job);
        }
        let mut toggled: Option<(TechLibrary, bool)> = None;
        if let Some(scanned) = self.scanned() {
            let pdk = scanned.pdk.clone();
            let resolve_path = move |text: &str| {
                TechPath(text.to_string())
                    .resolve_in(&pdk)
                    .ok()
                    .filter(|path| path.exists())
            };
            let resolve: techview::Resolve = &resolve_path;
            let mut asks: Vec<Ask> = Vec::new();
            Grid::new("wizard-summary")
                .num_columns(2)
                .spacing([16.0, 3.0])
                .show(ui, |ui| {
                    for (label, value) in &scanned.summary {
                        if label == "Libraries" || label == "Notes" || label == "PDK" {
                            continue;
                        }
                        ui.label(RichText::new(label).weak());
                        if value.starts_with("$PDK_ROOT") || value.starts_with('/') {
                            techview::path_label(ui, value, resolve, &mut asks);
                        } else {
                            ui.add(egui::Label::new(value).wrap());
                        }
                        ui.end_row();
                    }
                });
            let libraries = scanned.tech.libraries();
            if !libraries.is_empty() {
                ui.add_space(8.0);
                ui.strong("Libraries of the PDK to list in the project");
                ScrollArea::vertical()
                    .max_height(150.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        Grid::new("wizard-libraries")
                            .num_columns(3)
                            .spacing([14.0, 3.0])
                            .show(ui, |ui| {
                                for lib in &libraries {
                                    let mut on = lib.enabled;
                                    if ui.checkbox(&mut on, &lib.name).changed() {
                                        toggled = Some((lib.clone(), on));
                                    }
                                    ui.weak(&lib.role);
                                    ui.weak(lib.path.as_str());
                                    ui.end_row();
                                }
                            });
                    });
            }
            if !scanned.tech.notes.is_empty() {
                ui.add_space(6.0);
                egui::CollapsingHeader::new(format!(
                    "Notes of the scan ({})",
                    scanned.tech.notes.len()
                ))
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .id_salt("wizard-notes")
                        .max_height(120.0)
                        .show(ui, |ui| {
                            for note in &scanned.tech.notes {
                                ui.label(format!("• {note}"));
                            }
                        });
                });
            }
            ui.small(format!(
                "Scanned in {:.1} s. This goes into tech.toml and tech.catalog.json next to \
                 the project file; Project > Technology… shows all of it.",
                scanned.time.as_secs_f32()
            ));
        }
        if let Some((library, on)) = toggled
            && let Some(scanned) = &mut self.scanned
        {
            techview::set_library_enabled(&mut scanned.tech, &library, on);
            scanned.summary = techview::summary(&scanned.tech, Some(&scanned.catalog));
        }

        ui.add_space(10.0);
        let ready = self.scanned().is_some();
        ui.horizontal(|ui| {
            if ui.button("‹ Back").clicked() {
                outcome = Outcome::Back;
            }
            if ui.add_enabled(ready, egui::Button::new("Create")).clicked() {
                outcome = Outcome::Create;
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Cancel;
            }
        });
        if ready && outcome == Outcome::Open && ui.input(|i| i.key_pressed(Key::Enter)) {
            outcome = Outcome::Create;
        }
        outcome
    }

    /// Do what a page asked for. Returns whether the wizard closes.
    fn act(&mut self, outcome: Outcome, app: &mut AppCtx, ctx: &egui::Context) -> bool {
        match outcome {
            Outcome::Open => false,
            Outcome::Cancel => true,
            Outcome::Back => {
                self.page = Page::Project;
                false
            }
            Outcome::Next => {
                self.error = None;
                self.page = Page::Technology;
                self.start_scan(ctx);
                false
            }
            Outcome::Create => match self.create(app) {
                Ok(()) => true,
                Err(e) => {
                    self.error = Some(e);
                    false
                }
            },
        }
    }
}

/// Open the wizard over the window being built (File > New Project…).
pub(crate) fn open(app: &mut AppCtx, ctx: &egui::Context) {
    let wizard = Wizard::new(app, ctx);
    app.tech.wizard = Some((app.window, wizard));
}

/// Once per window and frame, after the dialogs: the wizard when it belongs to this
/// window; in the main window also the technology's background work and the offer to
/// restart documents after the technology changed.
pub(crate) fn show(ctx: &egui::Context, app: &mut AppCtx) {
    if app.window == WindowId::Main {
        if app.dialog.is_none() {
            // A Project Settings dialog that was cancelled leaves no wish behind.
            pdkscan::want_rescan(false);
        }
        pdkscan::poll(app, ctx);
        pdkscan::replace_ui(ctx, app);
        pdkscan::restart_offer_ui(ctx, app);
    }
    if app
        .tech
        .wizard
        .as_ref()
        .is_none_or(|(win, _)| *win != app.window)
    {
        return;
    }
    let Some((win, mut wizard)) = app.tech.wizard.take() else {
        return;
    };
    wizard.poll();
    let mut outcome = Outcome::Open;
    let response = egui::Modal::new(egui::Id::new("cdept-new-project")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.heading(match wizard.page {
            Page::Project => "New Project",
            Page::Technology => "New Project: Technology",
        });
        ui.add_space(6.0);
        outcome = match wizard.page {
            Page::Project => wizard.project_page(ui, app),
            Page::Technology => wizard.technology_page(ui),
        };
        if let Some(error) = &wizard.error {
            ui.add_space(6.0);
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    });
    // A click outside or Escape cancels.
    if response.should_close() && outcome == Outcome::Open {
        outcome = Outcome::Cancel;
    }
    if !wizard.act(outcome, app, ctx) {
        app.tech.wizard = Some((win, wizard));
    }
}

/// The PDKs found, kept for a few seconds between frames of a dialog.
fn cached_pdks(ui: &Ui, app: &AppCtx) -> std::sync::Arc<Vec<PdkEntry>> {
    type Cached = (std::time::Instant, std::sync::Arc<Vec<PdkEntry>>);
    let id = egui::Id::new("cdept-pdks-found");
    if let Some((at, found)) = ui.data(|d| d.get_temp::<Cached>(id))
        && at.elapsed() < Duration::from_secs(5)
    {
        return found;
    }
    let found = std::sync::Arc::new(pdkscan::find_pdks(&pdkscan::search(app)));
    ui.data_mut(|d| d.insert_temp::<Cached>(id, (std::time::Instant::now(), found.clone())));
    found
}

/// In the Project Settings dialog, below its fields: choose the technology and the
/// PDK folder from the PDKs found on this machine, and ask for a rescan.
pub(crate) fn settings_ui(ui: &mut Ui, settings: &mut libcdept::Settings, app: &AppCtx) {
    ui.horizontal(|ui| {
        ui.menu_button("Installed PDKs ▾", |ui| {
            let found = cached_pdks(ui, app);
            if found.is_empty() {
                ui.weak("None found.");
                ui.small("Type the PDK folder above, or add one in File > New Project…");
            }
            for entry in found.iter() {
                let text = format!(
                    "{}   {}   {}",
                    entry.pdk().name,
                    entry.version(),
                    entry.pdk().root.display()
                );
                if ui.button(text).clicked() {
                    settings.technology = entry.pdk().name.clone();
                    settings.pdk_root = entry.pdk().root.to_string_lossy().into_owned();
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Fill in Technology and PDK folder from a PDK found on this machine");
        let mut rescan = pdkscan::rescan_wanted();
        if ui
            .checkbox(&mut rescan, "Rescan technology")
            .on_hover_text(
                "Read the PDK's files again when the settings are saved, and write tech.toml. \
                 Done anyway when the PDK changes.",
            )
            .changed()
        {
            pdkscan::want_rescan(rescan);
        }
    });
    if let Some(job) = app.tech.rescanning() {
        ui.small(format!("Scanning {} …", job.pdk.name));
    }
}

/// List or leave out a library of the PDK in the open project: the snapshot's
/// `tech.toml` is written and the project read again.
pub(crate) fn list_library(
    app: &mut AppCtx,
    library: &TechLibrary,
    on: bool,
) -> Result<(), String> {
    let project = app.project.as_mut().ok_or("no project is open")?;
    let mut snapshot = project
        .tech
        .clone()
        .ok_or("the project has no technology snapshot")?;
    if !techview::set_library_enabled(&mut snapshot, library, on) {
        return Err(format!(
            "'{}' cannot be {} here; edit tech.toml",
            library.name,
            if on { "listed" } else { "left out" }
        ));
    }
    let text = tech::file::tech_text(&snapshot).map_err(|e| format!("{e:#}"))?;
    let file = tech::file::tech_file(&project.root);
    std::fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    project.reload().map_err(|e| format!("{e:#}"))?;
    tracing::info!(
        target: "cdept::tech",
        "library '{}' of the PDK is {}",
        library.name,
        if on { "listed in the project" } else { "left out of the project" }
    );
    app.tech.generation += 1;
    app.refresh_links();
    app.validate_selection();
    Ok(())
}

/// The summary lines and both files of the technology a script looks at: the
/// wizard's scan when it has one, else the open project's.
fn script_snapshot(app: &AppCtx) -> Result<(Tech, Option<Catalog>), String> {
    if let Some((_, wizard)) = &app.tech.wizard
        && let Some(scanned) = wizard.scanned()
    {
        return Ok((scanned.tech.clone(), Some(scanned.catalog.clone())));
    }
    if app.tech.wizard.is_some() {
        return Err("the wizard has not scanned a PDK (yet)".into());
    }
    let project = app.project.as_ref().ok_or("no project is open")?;
    let snapshot = project
        .tech
        .clone()
        .ok_or("the project has no technology snapshot")?;
    Ok((snapshot, project.catalog().ok().flatten()))
}

/// `tech <verb> …` of a test script (docs/GUI-TESTING.md). Every verb either does
/// its thing or fails without having done anything, so the script may try again.
pub(crate) fn script(app: &mut AppCtx, ctx: &egui::Context, args: &[String]) -> Result<(), String> {
    let (verb, rest) = args.split_first().ok_or("usage: tech <verb> …")?;
    let text = rest.join(" ");
    let on_off = |word: Option<&String>| match word.map(String::as_str) {
        Some("on" | "yes") => Ok(true),
        Some("off" | "no") => Ok(false),
        _ => Err(format!("tech {verb}: expected on or off")),
    };
    // Verbs that work without the wizard.
    match verb.as_str() {
        "wizard" => {
            app.window = WindowId::Main;
            open(app, ctx);
            return Ok(());
        }
        "page" => {
            app.tech.page = Some(text.trim().to_string());
            return Ok(());
        }
        "rescan" => {
            return pdkscan::start_rescan(app, ctx, rest.first().is_none_or(|w| w != "files"));
        }
        "expect_replace_question" => {
            return if app.tech.confirm.is_some() {
                Ok(())
            } else {
                Err("tech.toml is not being asked about".into())
            };
        }
        "answer" => {
            let yes = on_off(rest.first())?;
            return if pdkscan::answer_replace(app, yes) {
                Ok(())
            } else {
                Err("no question about replacing tech.toml is open".into())
            };
        }
        "expect_rescanned" => {
            if app.tech.rescan.is_some() || app.tech.pending {
                return Err("the technology is still being scanned".into());
            }
            if app.tech.confirm.is_some() {
                return Err("the question about replacing tech.toml is not answered".into());
            }
            return match &app.tech.last {
                Some(Ok(_)) => Ok(()),
                Some(Err(e)) => Err(format!("the rescan failed: {e}")),
                None => Err("no rescan was run".into()),
            };
        }
        "expect_offer" => {
            let wanted = on_off(rest.first())?;
            return if app.tech.restart_offer == wanted {
                Ok(())
            } else {
                Err(format!(
                    "the offer to reopen the tabs is {}",
                    if app.tech.restart_offer {
                        "shown"
                    } else {
                        "not shown"
                    }
                ))
            };
        }
        "restart" => {
            if !app.tech.restart_offer {
                return Err("nothing offers to reopen the tabs".into());
            }
            if on_off(rest.first())? {
                app.request(Action::RestartDocuments);
            }
            app.tech.restart_offer = false;
            return Ok(());
        }
        "expect_line" => {
            let (snapshot, catalog) = script_snapshot(app)?;
            let lines: Vec<String> = techview::summary(&snapshot, catalog.as_ref())
                .into_iter()
                .map(|(label, value)| format!("{label}: {value}"))
                .collect();
            return if lines.iter().any(|line| line.contains(text.trim())) {
                Ok(())
            } else {
                Err(format!("no line with '{text}' in the summary: {lines:?}"))
            };
        }
        "expect" => {
            let [path, wanted @ ..] = rest else {
                return Err("usage: tech expect <path> <value>".into());
            };
            let wanted = wanted.join(" ");
            let (snapshot, catalog) = script_snapshot(app)?;
            let value = match path.strip_prefix("catalog.") {
                Some(path) => {
                    let catalog = catalog.ok_or("there is no catalog")?;
                    techview::lookup(&techview::to_json(&catalog), path).map(techview::text_of)
                }
                None => {
                    techview::lookup(&techview::to_json(&snapshot), path).map(techview::text_of)
                }
            };
            return match value {
                Some(value) if value == wanted => Ok(()),
                Some(value) => Err(format!("{path} is '{value}', expected '{wanted}'")),
                None => Err(format!("the snapshot has no {path}")),
            };
        }
        "library" if app.tech.wizard.is_none() => {
            let [name, state] = rest else {
                return Err("usage: tech library <name> <on|off>".into());
            };
            let on = on_off(Some(state))?;
            let (snapshot, _) = script_snapshot(app)?;
            let library = snapshot
                .libraries()
                .into_iter()
                .find(|lib| lib.name == *name)
                .ok_or(format!("the PDK has no library '{name}'"))?;
            return list_library(app, &library, on);
        }
        _ => {}
    }
    // The rest drives the wizard.
    let Some((win, mut wizard)) = app.tech.wizard.take() else {
        return Err(format!("tech {verb}: the New Project wizard is not open"));
    };
    wizard.poll();
    let mut close = false;
    let result = (|| -> Result<(), String> {
        match verb.as_str() {
            "folder" => wizard.folder = text.clone(),
            "name" => wizard.name = text.clone(),
            "root" => wizard.add_folder(app, Path::new(text.trim()))?,
            "files_only" => wizard.files_only = on_off(rest.first())?,
            "expect_pdks" => {
                if wizard.finding.is_some() {
                    return Err("still looking for PDKs".into());
                }
                for name in rest {
                    if !wizard.pdks.iter().any(|e| e.pdk().name == *name) {
                        let found: Vec<&str> =
                            wizard.pdks.iter().map(|e| e.pdk().name.as_str()).collect();
                        return Err(format!("the PDK {name} is not listed: {found:?}"));
                    }
                }
            }
            "expect_no_pdk" => {
                if wizard.finding.is_some() {
                    return Err("still looking for PDKs".into());
                }
                if let Some(entry) = wizard.pdks.iter().find(|e| {
                    e.found.real.to_string_lossy().contains(text.trim())
                        || e.pdk().root.to_string_lossy().contains(text.trim())
                }) {
                    return Err(format!(
                        "{} is listed from {}",
                        entry.pdk().name,
                        entry.found.real.display()
                    ));
                }
            }
            "pdk" => {
                if wizard.finding.is_some() {
                    return Err("still looking for PDKs".into());
                }
                wizard.choice = match text.trim() {
                    "none" => None,
                    name => Some(
                        wizard
                            .pdks
                            .iter()
                            .position(|e| e.pdk().name == name)
                            .ok_or(format!("the PDK {name} is not listed"))?,
                    ),
                };
            }
            "expect_pdk" => {
                if wizard.finding.is_some() {
                    return Err("still looking for PDKs".into());
                }
                let chosen = wizard.chosen().map_or("none", |e| e.pdk().name.as_str());
                if chosen != text.trim() {
                    return Err(format!("the chosen PDK is {chosen}"));
                }
            }
            "next" => {
                if wizard.choice.is_none() {
                    return Err("no PDK is chosen".into());
                }
                close = wizard.act(Outcome::Next, app, ctx);
            }
            "back" => close = wizard.act(Outcome::Back, app, ctx),
            "expect_scanned" => {
                if wizard.scanned().is_none() {
                    return Err("the PDK is not scanned (yet)".into());
                }
            }
            "library" => {
                let [name, state] = rest else {
                    return Err("usage: tech library <name> <on|off>".into());
                };
                let on = on_off(Some(state))?;
                let scanned = wizard.scanned.as_mut().ok_or("the PDK is not scanned")?;
                let library = scanned
                    .tech
                    .libraries()
                    .into_iter()
                    .find(|lib| lib.name == *name)
                    .ok_or(format!("the PDK has no library '{name}'"))?;
                techview::set_library_enabled(&mut scanned.tech, &library, on);
            }
            "create" => {
                wizard.create(app)?;
                close = true;
            }
            "cancel" => close = true,
            other => return Err(format!("unknown: tech {other}")),
        }
        Ok(())
    })();
    if !close {
        app.tech.wizard = Some((win, wizard));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_folder_is_proposed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(free_folder(dir.path()), dir.path().join("project"));
        std::fs::create_dir(dir.path().join("project")).unwrap();
        assert_eq!(free_folder(dir.path()), dir.path().join("project2"));
    }
}
