//! "Open in UberWave": the waveform viewer is a separate program (`libcdept::wave`);
//! the GUI only starts it on the raw file of a run point.

use std::path::Path;

use libcdept::Project;

/// Start UberWave on the `out.raw` of a run point folder: the action of the button.
pub(crate) fn open_dir(project: &Project, dir: &Path) -> Result<String, String> {
    let raw = dir.join("out.raw");
    if !raw.is_file() {
        return Err(format!("{} has no out.raw (the run left no raw file)", dir.display()));
    }
    libcdept::wave::open_raw(Some(project), &raw, None).map(|s| s.text()).map_err(|e| format!("{e:#}"))
}

/// The button: failures (no viewer, with how to get it; no raw file) go to the log.
pub(crate) fn open_run_dir(project: &Project, dir: &Path) {
    match open_dir(project, dir) {
        Ok(line) => tracing::info!(target: "cdept::sim", "UberWave: {line}"),
        Err(e) => tracing::warn!(target: "cdept::sim", "Open in UberWave: {e}"),
    }
}

/// `wave open <lib> <cell> <view> <point>` of a test script: the same, for a point
/// by name. With `CDEPT_UBERWAVE` naming a fake program a script checks what it got.
pub(crate) fn script(project: Option<&Project>, args: &[String]) -> Result<(), String> {
    let project = project.ok_or("no project is open")?;
    match args {
        [open, lib, cell, view, point] if open == "open" => {
            let dir = libcdept::sim::runs_dir(project, lib, cell, view).join(point);
            let line = open_dir(project, &dir)?;
            tracing::info!(target: "cdept::script", "wave: {line}");
            Ok(())
        }
        _ => Err("usage: wave open <lib> <cell> <view> <point>".into()),
    }
}
