//! Background work of the technology: finding the PDKs on this machine, scanning one
//! with progress per section, and the rescan of the open project's technology.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use libcdept::tech::discover::{self, Found, Search};
use libcdept::tech::{self, Catalog, PdkDir, PdkInfo, ScanContext, Tech};

use crate::state::{Action, AppCtx};

/// Something worked out on another thread.
pub struct Job<T> {
    result: Receiver<T>,
    pub started: Instant,
}

impl<T: Send + 'static> Job<T> {
    pub fn start(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Job<T> {
        let (sender, result) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = sender.send(work());
            ctx.request_repaint();
        });
        Job {
            result,
            started: Instant::now(),
        }
    }

    /// The result, once it is there.
    pub fn poll(&mut self) -> Option<T> {
        self.result.try_recv().ok()
    }
}

/// A PDK found on this machine, with what it says about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct PdkEntry {
    pub found: Found,
    pub info: PdkInfo,
}

impl PdkEntry {
    pub fn pdk(&self) -> &PdkDir {
        &self.found.pdk
    }

    /// `1.0.530 (8afc8346a57f)`: version and the start of the commit.
    pub fn version(&self) -> String {
        let commit = self.info.commit.as_deref().map(short);
        match (&self.info.version, commit) {
            (Some(v), Some(c)) => format!("{v} ({c})"),
            (Some(v), None) => v.clone(),
            (None, Some(c)) => c.to_string(),
            (None, None) => String::new(),
        }
    }
}

/// The first 12 digits of a commit; anything else as it is.
pub fn short(text: &str) -> &str {
    if text.len() == 40 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        &text[..12]
    } else {
        text
    }
}

/// Where the GUI looks for PDKs: the open project's PDK folder, the environment, the
/// user's extra folders, the usual places.
pub fn search(app: &AppCtx) -> Search {
    Search::new()
        .project(app.pdk_root().map(PathBuf::from))
        .extra(app.settings.pdk_folders.iter().cloned())
}

/// Every PDK `search` finds, with its description.
pub fn find_pdks(search: &Search) -> Vec<PdkEntry> {
    discover::discover(search)
        .into_iter()
        .map(|found| PdkEntry {
            info: found.info(),
            found,
        })
        .collect()
}

/// The sections of a scan, in the order they are shown.
pub const SECTIONS: [&str; 8] = [
    "pdk", "klayout", "magic", "netgen", "xschem", "models", "refs", "devices",
];

/// What a section of the scan is called on screen.
pub fn section_label(name: &str) -> &'static str {
    match name {
        "pdk" => "PDK description",
        "klayout" => "KLayout technology, layers, decks, PCells",
        "magic" => "Magic technology",
        "netgen" => "Netgen set-up",
        "xschem" => "xschem start-up file and symbols",
        "models" => "Simulation models and corners",
        "refs" => "Reference libraries",
        "devices" => "Devices",
        _ => "",
    }
}

/// How far a section of a running scan is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    Waiting,
    Running,
    Done(Duration),
}

/// Scan a PDK, saying when each section starts and ends.
///
/// This is `libcdept::tech::scan` with the reports added: the same sections, put
/// together the same way (a test compares the two).
pub fn scan_with_progress(
    ctx: &ScanContext,
    report: &(dyn Fn(&'static str, bool) + Sync),
) -> (Tech, Catalog) {
    tech::scan_with(ctx, report)
}

/// Each section of a running scan: how far it is and when it started.
type Stages = Arc<Mutex<Vec<(&'static str, Stage, Instant)>>>;

/// A scan of a PDK running on another thread.
pub struct ScanJob {
    pub pdk: PdkDir,
    /// With the helper programs (KLayout, xschem), or reading files only.
    pub helpers: bool,
    stages: Stages,
    job: Job<(Tech, Catalog)>,
}

impl ScanJob {
    /// `env`: more variables for the programs the scan starts (a project's own).
    pub fn start(
        ctx: &egui::Context,
        pdk: PdkDir,
        helpers: bool,
        env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> ScanJob {
        let now = Instant::now();
        let stages = Arc::new(Mutex::new(
            SECTIONS
                .iter()
                .map(|name| (*name, Stage::Waiting, now))
                .collect::<Vec<_>>(),
        ));
        let mut scan = ScanContext::new(pdk.clone());
        scan.helpers = helpers;
        scan.env.extend(env);
        let shared = stages.clone();
        let repaint = ctx.clone();
        let job = Job::start(ctx, move || {
            scan_with_progress(&scan, &|name, done| {
                let mut stages = shared.lock().unwrap();
                if let Some(stage) = stages.iter_mut().find(|(n, _, _)| *n == name) {
                    if done {
                        stage.1 = Stage::Done(stage.2.elapsed());
                    } else {
                        stage.1 = Stage::Running;
                        stage.2 = Instant::now();
                    }
                }
                repaint.request_repaint();
            })
        });
        ScanJob {
            pdk,
            helpers,
            stages,
            job,
        }
    }

    /// Each section and how far it is.
    pub fn stages(&self) -> Vec<(&'static str, Stage)> {
        self.stages
            .lock()
            .unwrap()
            .iter()
            .map(|(name, stage, _)| (*name, *stage))
            .collect()
    }

    pub fn elapsed(&self) -> Duration {
        self.job.started.elapsed()
    }

    /// The snapshot and the catalog, once every section is done.
    pub fn poll(&mut self) -> Option<(Tech, Catalog)> {
        self.job.poll()
    }
}

/// Rows of spinners and check marks for a running scan.
pub fn progress_ui(ui: &mut egui::Ui, job: &ScanJob) {
    egui::Grid::new("scan-progress")
        .num_columns(3)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, stage) in job.stages() {
                match stage {
                    Stage::Waiting => ui.weak("·"),
                    Stage::Running => ui.spinner(),
                    Stage::Done(_) => ui.label("✔"),
                };
                ui.label(section_label(name));
                match stage {
                    Stage::Done(time) => ui.weak(format!("{:.1} s", time.as_secs_f32())),
                    _ => ui.weak(""),
                };
                ui.end_row();
            }
        });
    if !job.helpers {
        ui.weak("Files only: KLayout and xschem are not started.");
    }
    // The spinners turn, and the times of running helpers are not reported.
    ui.ctx().request_repaint_after(Duration::from_millis(100));
}

/// The technology part of the GUI's state: the New Project wizard, a rescan of the
/// open project's technology, what to say after it.
#[derive(Default)]
pub struct TechState {
    /// The New Project wizard and the window it belongs to.
    pub(crate) wizard: Option<(crate::dock::WindowId, crate::newproject::Wizard)>,
    /// The rescan of the open project's technology, and the root of that project.
    pub(crate) rescan: Option<(PathBuf, ScanJob)>,
    /// The technology changed while documents were open: offer to restart them.
    pub(crate) restart_offer: bool,
    /// The last rescan: `Ok(PDK name)` or what went wrong (for scripts and the tab).
    pub(crate) last: Option<Result<String, String>>,
    /// Rescans read files only (the "Files only" box of the Technology tab).
    pub(crate) files_only: bool,
    /// The page of the Technology tab a test script asked for.
    pub(crate) page: Option<String>,
    /// A rescan was asked for where the egui context was not at hand.
    pub(crate) pending: bool,
    /// Counts the changes of the project's technology, so views can drop what they
    /// keep of it.
    pub(crate) generation: u64,
}

impl TechState {
    pub fn rescanning(&self) -> Option<&ScanJob> {
        self.rescan.as_ref().map(|(_, job)| job)
    }
}

/// The Project Settings dialog asks for a rescan when it is confirmed.
static RESCAN_WANTED: AtomicBool = AtomicBool::new(false);

pub fn want_rescan(wanted: bool) {
    RESCAN_WANTED.store(wanted, Ordering::SeqCst);
}

pub fn rescan_wanted() -> bool {
    RESCAN_WANTED.load(Ordering::SeqCst)
}

/// Scan the open project's PDK again, in the background. `ctx` is needed to wake the
/// window when it is done.
pub fn start_rescan(app: &mut AppCtx, ctx: &egui::Context, helpers: bool) -> Result<(), String> {
    if app.tech.rescan.is_some() {
        return Ok(());
    }
    let project = app.project.as_ref().ok_or("no project is open")?;
    let pdk = project.pdk.dir().ok_or(
        "the project has no PDK: choose one in Project > Project Settings… \
         (or set PDK and PDK_ROOT in the environment)",
    )?;
    if !discover::is_pdk(&pdk.dir()) {
        return Err(format!(
            "{} is not a PDK folder (no libs.tech in it): set the PDK folder in Project > \
             Project Settings…",
            pdk.dir().display()
        ));
    }
    tracing::info!(target: "cdept::tech", "scanning the PDK {} in {} …", pdk.name, pdk.root.display());
    let job = ScanJob::start(ctx, pdk, helpers, project.tool_env());
    app.tech.rescan = Some((project.root.clone(), job));
    Ok(())
}

/// After the Project Settings were saved: scan again when the PDK changed or the
/// dialog asked for it. Needs the egui context, so the scan itself starts with the
/// next frame ([`poll`]).
pub fn after_settings(app: &mut AppCtx, pdk_changed: bool) {
    let wanted = RESCAN_WANTED.swap(false, Ordering::SeqCst);
    let has_pdk = app
        .project
        .as_ref()
        .and_then(|p| p.pdk.dir())
        .is_some_and(|pdk| discover::is_pdk(&pdk.dir()));
    if (pdk_changed || wanted) && has_pdk {
        app.tech.pending = true;
    } else if pdk_changed {
        // No PDK folder to scan: only say that open documents keep the old one.
        app.tech.restart_offer = app.open_documents > 0;
    }
}

/// Once per frame: start a rescan that was asked for, take the result of a running
/// one and make it the project's technology.
pub fn poll(app: &mut AppCtx, ctx: &egui::Context) {
    if std::mem::take(&mut app.tech.pending)
        && let Err(e) = start_rescan(app, ctx, !app.tech.files_only)
    {
        app.tech.last = Some(Err(e.clone()));
        tracing::warn!(target: "cdept::tech", "the technology was not scanned: {e}");
    }
    let Some((root, job)) = &mut app.tech.rescan else {
        return;
    };
    let Some((tech, catalog)) = job.poll() else {
        return;
    };
    let (root, job) = (root.clone(), app.tech.rescan.take().expect("checked").1);
    let outcome = apply(app, &root, &job.pdk, &tech, &catalog);
    match &outcome {
        Ok(name) => tracing::info!(
            target: "cdept::tech",
            "technology of {name} scanned in {:.1} s: {} note(s)",
            job.elapsed().as_secs_f32(),
            tech.notes.len()
        ),
        Err(e) => tracing::error!(target: "cdept::tech", "the technology was not saved: {e}"),
    }
    app.tech.last = Some(outcome);
}

/// Make a scan's result the technology of the open project (if it still is the
/// project the scan was started for).
fn apply(
    app: &mut AppCtx,
    root: &std::path::Path,
    pdk: &PdkDir,
    tech: &Tech,
    catalog: &Catalog,
) -> Result<String, String> {
    let project = app
        .project
        .as_mut()
        .filter(|project| project.root == root)
        .ok_or("the project was closed while its PDK was scanned")?;
    project
        .set_technology(pdk, tech, catalog)
        .map_err(|e| format!("{e:#}"))?;
    technology_changed(app);
    Ok(pdk.name.clone())
}

/// The project's technology files changed: look at the libraries again and offer to
/// restart the documents that still run with the old set-up.
pub fn technology_changed(app: &mut AppCtx) {
    app.tech.generation += 1;
    app.refresh_links();
    app.validate_selection();
    if app.open_documents > 0 {
        app.tech.restart_offer = true;
        tracing::warn!(
            target: "cdept::tech",
            "the technology changed: the open schematic and layout tabs still run with the \
             old set-up until they are reopened"
        );
    }
}

/// The offer to restart the open documents after the technology changed.
pub fn restart_offer_ui(ctx: &egui::Context, app: &mut AppCtx) {
    if !app.tech.restart_offer {
        return;
    }
    if app.open_documents == 0 {
        app.tech.restart_offer = false;
        return;
    }
    let mut close = false;
    let response = egui::Modal::new(egui::Id::new("cdept-tech-restart")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading("The technology changed");
        ui.add_space(6.0);
        ui.label(format!(
            "{} open schematic or layout tab(s) still run with the set-up they were opened \
             with (PDK, start-up files, layers).",
            app.open_documents
        ));
        ui.label("Reopening starts them again with the new one. Tabs with unsaved changes are left alone.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Reopen Tabs").clicked() {
                app.request(Action::RestartDocuments);
                close = true;
            }
            if ui.button("Later").clicked() {
                close = true;
            }
        });
    });
    if close || response.should_close() {
        app.tech.restart_offer = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small made-up PDK: enough folders for the sections to look at.
    fn fake_pdk(root: &std::path::Path) -> PdkDir {
        let dir = root.join("fake130");
        for tool in ["klayout/tech", "xschem", "magic", "netgen", "ngspice"] {
            std::fs::create_dir_all(dir.join("libs.tech").join(tool)).unwrap();
        }
        std::fs::create_dir_all(dir.join("libs.ref/fake_sc/gds")).unwrap();
        std::fs::write(dir.join("libs.tech/xschem/xschemrc"), "# nothing\n").unwrap();
        PdkDir::new(root, "fake130")
    }

    #[test]
    fn the_scan_with_progress_is_the_scan() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ScanContext::new(fake_pdk(dir.path())).files_only();
        let seen = Mutex::new(Vec::new());
        let (tech, catalog) = scan_with_progress(&ctx, &|name, done| {
            seen.lock().unwrap().push((name, done));
        });
        let (plain_tech, plain_catalog) = tech::scan(&ctx);
        assert_eq!(tech, plain_tech);
        assert_eq!(catalog, plain_catalog);
        let seen = seen.into_inner().unwrap();
        for name in SECTIONS {
            assert!(seen.contains(&(name, false)), "{name} started");
            assert!(seen.contains(&(name, true)), "{name} ended");
            assert!(!section_label(name).is_empty(), "{name} has a label");
        }
        assert_eq!(seen.last(), Some(&("devices", true)), "devices come last");
    }

    #[test]
    fn the_real_scan_is_the_same_too() {
        let root = std::path::Path::new("/opt/ext/OpenPDKs");
        if !root.join("ihp-sg13g2/libs.tech").is_dir() {
            eprintln!("skipped: no ihp-sg13g2 in {}", root.display());
            return;
        }
        let ctx = ScanContext::new(PdkDir::new(root, "ihp-sg13g2")).files_only();
        let (tech, catalog) = scan_with_progress(&ctx, &|_, _| {});
        let (plain_tech, plain_catalog) = tech::scan(&ctx);
        assert_eq!(tech, plain_tech);
        assert_eq!(catalog, plain_catalog);
    }

    #[test]
    fn changing_the_pdk_in_the_settings_scans_it() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = fake_pdk(&dir.path().join("pdks"));
        let root = dir.path().join("project");
        libcdept::Project::create(&root, Some("demo")).unwrap();
        let mut app = AppCtx::new(
            crate::state::Settings::default(),
            crate::log::LogBuffer::new(),
        );
        app.load_project(&root);
        assert!(app.project.as_ref().unwrap().tech.is_none());
        app.tech.files_only = true;
        // A tab is open: it will keep the old set-up.
        app.open_documents = 1;

        let settings = libcdept::Settings {
            name: "demo".into(),
            technology: pdk.name.clone(),
            pdk_root: pdk.root.to_string_lossy().into_owned(),
            ..Default::default()
        };
        app.change_settings(&settings).unwrap();
        assert!(app.tech.pending, "the scan starts with the next frame");
        let ctx = egui::Context::default();
        for _ in 0..500 {
            poll(&mut app, &ctx);
            if app.tech.last.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.tech.last, Some(Ok("fake130".to_string())));
        let project = app.project.as_ref().unwrap();
        assert!(project.tech_in_use);
        assert_eq!(project.tech.as_ref().unwrap().pdk.name, "fake130");
        assert!(root.join(tech::TECH_FILE).is_file());
        assert!(root.join(tech::CATALOG_FILE).is_file());
        assert!(app.tech.restart_offer, "the open tab is offered a restart");
        assert_eq!(app.tech.generation, 1);

        // Saving the settings without a change scans nothing, unless asked.
        app.tech.last = None;
        app.change_settings(&settings).unwrap();
        assert!(!app.tech.pending);
        want_rescan(true);
        app.change_settings(&settings).unwrap();
        assert!(app.tech.pending);
        assert!(!rescan_wanted(), "the wish is used up");
    }

    #[test]
    fn a_project_without_a_pdk_cannot_be_rescanned() {
        let dir = tempfile::tempdir().unwrap();
        libcdept::Project::create(dir.path(), Some("demo")).unwrap();
        let mut app = AppCtx::new(
            crate::state::Settings::default(),
            crate::log::LogBuffer::new(),
        );
        app.load_project(dir.path());
        if app.pdk().is_some() {
            eprintln!("skipped: the environment names a PDK");
            return;
        }
        let error = start_rescan(&mut app, &egui::Context::default(), false).unwrap_err();
        assert!(error.contains("no PDK"), "{error}");
        assert!(app.tech.rescan.is_none());
    }

    #[test]
    fn commits_are_shortened() {
        assert_eq!(
            short("8afc8346a57fe1ab7934ba5a6056ea8b43078e71"),
            "8afc8346a57f"
        );
        assert_eq!(short("1.0.530"), "1.0.530");
    }
}
