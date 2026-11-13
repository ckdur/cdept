//! CDePT egui frontend: the docked main window and its views.

mod app;
mod dialogs;
mod dock;
pub mod log;
mod state;
mod views;

use std::path::PathBuf;

pub use log::LogBuffer;

const APP_ID: &str = "org.ckdur.cdept";
const APP_NAME: &str = "Chip Design Project Tool (CDePT)";

/// Startup options for the GUI.
#[derive(Debug, Default)]
pub struct GuiOptions {
    /// Project directory to open.
    pub project: Option<PathBuf>,
}

/// Open the main window and block until it is closed.
///
/// `log` should be the buffer whose [`LogBuffer::layer`] was installed in the tracing
/// subscriber, so that log lines show up in the Console panel.
pub fn run(options: GuiOptions, log: LogBuffer) -> eyre::Result<()> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(APP_NAME)
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        native,
        Box::new(move |cc| Ok(Box::new(app::CdeptApp::new(cc, options, log)))),
    )
    .map_err(|e| eyre::eyre!("failed to run the GUI: {e}"))
}
