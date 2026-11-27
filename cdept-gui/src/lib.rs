//! CDePT egui frontend: the docked main window and its views.

mod app;
mod background;
mod cace;
mod dialogs;
mod dock;
mod engines;
mod import;
mod import_files;
mod library_dialog;
pub mod log;
mod newproject;
mod pathfield;
mod pdkscan;
mod refactor;
mod script;
mod state;
mod uberwave;
mod views;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use log::LogBuffer;

/// Whether the waveform tabs have UberWave's data engine (not its stub), for `cdept tools`.
pub fn waveform_engine_built_in() -> bool {
    cdept_wavedata::available()
}

const APP_ID: &str = "org.ckdur.cdept";
const APP_NAME: &str = "Chip Design Project Tool (CDePT)";

/// Startup options for the GUI.
#[derive(Debug, Default)]
pub struct GuiOptions {
    /// Project directory to open.
    pub project: Option<PathBuf>,
    /// A script of commands to run in the GUI, for testing (see `docs/GUI-TESTING.md`).
    pub script: Option<PathBuf>,
}

fn egui_glow_configuration(vsync: bool) -> eframe::egui_glow::GlowConfiguration {
    eframe::egui_glow::GlowConfiguration {
        vsync,
        ..Default::default()
    }
}

/// Open the main window and block until it is closed.
///
/// `log` should be the buffer whose [`LogBuffer::layer`] was installed in the tracing
/// subscriber, so that log lines show up in the Console panel.
pub fn run(options: GuiOptions, log: LogBuffer) -> eyre::Result<()> {
    let failed = Arc::new(AtomicBool::new(false));
    let script = match &options.script {
        Some(file) => {
            let text = std::fs::read_to_string(file)
                .map_err(|e| eyre::eyre!("cannot read the script {}: {e}", file.display()))?;
            // `$PROJECT` in a script is the folder of the project being opened.
            let project = options.project.as_deref().map(|path| {
                let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
                libcdept::Project::find_root(&path).unwrap_or(path)
            });
            let text = match &project {
                Some(root) => text.replace("$PROJECT", &root.to_string_lossy()),
                None => text,
            };
            let script = script::Script::parse(&text, failed.clone())
                .map_err(|e| eyre::eyre!("{}: {e}", file.display()))?;
            Some(script)
        }
        None => None,
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(APP_NAME)
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([800.0, 500.0]),
        // A script should not wait for the screen: a window that is hidden or on a
        // locked desktop is only shown a frame now and then.
        glow_options: egui_glow_configuration(script.is_none()),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        native,
        Box::new(move |cc| Ok(Box::new(app::CdeptApp::new(cc, options, log, script)))),
    )
    .map_err(|e| eyre::eyre!("failed to run the GUI: {e}"))?;
    // The built-in UberWave window (a thread of this process) goes with the GUI.
    libcdept::wave::embedded::close();
    if failed.load(Ordering::SeqCst) {
        eyre::bail!("the script failed; see the messages above");
    }
    Ok(())
}
