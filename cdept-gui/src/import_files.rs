//! Import Files…: design files from outside the project become cells and views of a
//! library (`libcdept::importer`).
//!
//! The dialog lists the files (a path field with a native "Browse…" button adds one at
//! a time), the library, and what to do with a file that holds many cells and with a
//! view the library has already. Below the fields it shows the plan, like the rename
//! and copy dialogs do; the dialog it leads to ([`Dialog::ImportSummary`]) says what
//! came in and what could not.

use std::sync::Arc;

use egui::{Id, Ui};
use eyre::{Result, eyre};
use libcdept::importer::{self, ImportRequest, OnClash};
use libcdept::{Library, Project};

use crate::dialogs::Dialog;
use crate::pathfield::PathField;
use crate::state::{AppCtx, Selection};

/// The state of the "Import Files" dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportFiles {
    pub library: String,
    /// As typed or browsed: relative to the project folder, `~` and `$VAR` allowed.
    pub files: Vec<String>,
    /// The field for the next file.
    pub next: String,
    pub split: bool,
    pub prefix: String,
    pub on_clash: OnClash,
}

impl ImportFiles {
    pub fn new(library: &str) -> Self {
        ImportFiles {
            library: library.into(),
            files: Vec::new(),
            next: String::new(),
            split: true,
            prefix: String::new(),
            on_clash: OnClash::Skip,
        }
    }

    fn request(&self, project: &Project) -> ImportRequest {
        let files = self
            .files
            .iter()
            .filter_map(|f| crate::pathfield::resolve(f, Some(&project.root), None))
            .collect();
        let mut request = ImportRequest::new(&self.library, files);
        request.split = self.split;
        request.prefix = self.prefix.trim().to_string();
        request.on_clash = self.on_clash;
        request
    }

    /// The text of the next-file field becomes a file of the list.
    fn add_next(&mut self) {
        let text = self.next.trim().to_string();
        if !text.is_empty() && !self.files.contains(&text) {
            self.files.push(text);
        }
        self.next.clear();
    }

    /// Set a field by name, for test scripts.
    pub(crate) fn set(&mut self, field: &str, value: &str) -> Result<(), String> {
        let on = |v: &str| matches!(v, "yes" | "on" | "true");
        match field {
            "add" => {
                self.next = value.to_string();
                self.add_next();
            }
            "clear" => self.files.clear(),
            "library" => self.library = value.to_string(),
            "split" => self.split = on(value),
            "prefix" => self.prefix = value.to_string(),
            "on_clash" => {
                self.on_clash = OnClash::from_name(value)
                    .ok_or("on_clash is skip, rename or overwrite".to_string())?
            }
            other => return Err(format!("this dialog has no field '{other}'")),
        }
        Ok(())
    }
}

fn clash_label(clash: OnClash) -> &'static str {
    match clash {
        OnClash::Skip => "Keep the existing view",
        OnClash::Rename => "Import under another name",
        OnClash::Overwrite => "Replace the existing file",
    }
}

/// The preview kept between frames: the files are read again only when the request
/// changes or an operation changed the project.
#[derive(Clone)]
struct Cache {
    request: ImportRequest,
    epoch: u64,
    shown: Arc<crate::refactor::Shown>,
}

/// The fields of the dialog and the plan below them.
pub fn body(ui: &mut Ui, import: &mut ImportFiles, app: &AppCtx) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    let writable: Vec<&Library> = project
        .libraries
        .iter()
        .filter(|l| !l.read_only && l.error.is_none() && l.layout.editable())
        .collect();
    egui::Grid::new("import-files-grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Into library");
            egui::ComboBox::from_id_salt("import-library")
                .selected_text(import.library.as_str())
                .show_ui(ui, |ui| {
                    for lib in &writable {
                        ui.selectable_value(&mut import.library, lib.name.clone(), &lib.name);
                    }
                });
            ui.end_row();
            ui.label("Prefix");
            ui.add(egui::TextEdit::singleline(&mut import.prefix).desired_width(160.0))
                .on_hover_text("Put in front of every cell name");
            ui.end_row();
            ui.label("If a view exists");
            egui::ComboBox::from_id_salt("import-clash")
                .selected_text(clash_label(import.on_clash))
                .show_ui(ui, |ui| {
                    for c in [OnClash::Skip, OnClash::Rename, OnClash::Overwrite] {
                        ui.selectable_value(&mut import.on_clash, c, clash_label(c));
                    }
                });
            ui.end_row();
        });
    ui.checkbox(
        &mut import.split,
        "One cell per structure, subcircuit, macro, module or cell",
    )
    .on_hover_text(
        "Takes the cells out of a file that holds several (a standard-cell GDS, SPICE, \
         LEF, Liberty or Verilog file). Off: every file is one cell named after it.",
    );
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Files").small().strong());
    let mut remove = None;
    for (i, file) in import.files.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button("✕").on_hover_text("Leave out").clicked() {
                remove = Some(i);
            }
            ui.label(egui::RichText::new(file).small());
        });
    }
    if let Some(i) = remove {
        import.files.remove(i);
    }
    ui.horizontal(|ui| {
        let changed = PathField::file("import-next", &mut import.next)
            .base(&project.root)
            .title("Choose a file to import")
            .width(240.0)
            .show(ui)
            .changed;
        let add = ui
            .add_enabled(!import.next.trim().is_empty(), egui::Button::new("Add"))
            .clicked();
        let _ = changed;
        if add {
            import.add_next();
        }
    });
    preview(ui, app, project, import);
}

fn preview(ui: &mut Ui, app: &AppCtx, project: &Project, import: &ImportFiles) {
    if import.files.is_empty() {
        ui.add_space(6.0);
        ui.weak("Add the files to import: GDSII, SPICE, CDL, LEF, Liberty, Verilog, xschem, …");
        return;
    }
    let request = import.request(project);
    let id = Id::new("cdept-import-preview");
    let cached: Option<Cache> = ui.data(|d| d.get_temp(id));
    let cache = match cached {
        Some(c) if c.request == request && c.epoch == app.refactor.epoch => c,
        _ => Cache {
            shown: Arc::new(crate::refactor::shown(
                project,
                importer::plan_import(project, &request),
            )),
            request,
            epoch: app.refactor.epoch,
        },
    };
    ui.add_space(6.0);
    crate::refactor::draw(ui, &cache.shown);
    ui.data_mut(|d| d.insert_temp(id, cache));
}

/// The lines of the plan, for test scripts.
pub(crate) fn plan_lines(app: &AppCtx, import: &ImportFiles) -> Result<Vec<String>, String> {
    let project = app.project.as_ref().ok_or("no project is open")?;
    let plan =
        importer::plan_import(project, &import.request(project)).map_err(|e| format!("{e:#}"))?;
    Ok(plan.lines(&project.root))
}

/// Import: write the files, read the libraries again, select the first view that came
/// in and say what happened in a dialog.
pub fn apply(app: &mut AppCtx, import: &ImportFiles) -> Result<()> {
    const TARGET: &str = "cdept::library";
    let project = app
        .project
        .as_mut()
        .ok_or_else(|| eyre!("no project is open"))?;
    let planned = importer::plan_import_report(project, &import.request(project))?;
    if planned.report.imported.is_empty() {
        return Err(eyre!(
            "nothing to import{}",
            planned
                .report
                .problems
                .first()
                .or(planned.report.skipped.first())
                .map(|s| format!(": {s}"))
                .unwrap_or_default()
        ));
    }
    let outcome = libcdept::refactor::apply(project, &planned.plan)?;
    for line in planned.report.lines().iter().chain(&outcome.lines) {
        tracing::info!(target: TARGET, "{line}");
    }
    for problem in &outcome.problems {
        tracing::warn!(target: TARGET, "{problem}");
    }
    project.rescan();
    app.refactor.epoch += 1;
    app.refresh_links();
    let result = &planned.plan.result;
    app.selection = Selection::view(&result.library, &result.cell, &result.view);
    app.validate_selection();
    let mut lines = planned.report.lines();
    lines.splice(
        1..1,
        planned
            .report
            .cells()
            .iter()
            .take(12)
            .map(|c| format!("  {}/{c}", import.library)),
    );
    let more = planned.report.cells().len().saturating_sub(12);
    if more > 0 {
        lines.insert(13, format!("  … and {more} more"));
    }
    app.open_dialog(Dialog::ImportSummary(lines));
    Ok(())
}

/// Entry of a library's context menu.
pub fn menu(ui: &mut Ui, lib: &Library) -> Option<Dialog> {
    let rw = !lib.read_only && lib.error.is_none() && lib.layout.editable();
    let clicked = ui
        .add_enabled(rw, egui::Button::new("Import Files…"))
        .on_hover_text(
            "Bring GDSII, SPICE, LEF, Liberty, Verilog, xschem … files from anywhere in as \
             cells and views",
        )
        .on_disabled_hover_text("The library is read-only")
        .clicked();
    clicked.then(|| Dialog::ImportFiles(ImportFiles::new(&lib.name)))
}

/// The files of a request as paths, for the tests.
#[cfg(test)]
fn paths(import: &ImportFiles, project: &Project) -> Vec<std::path::PathBuf> {
    import.request(project).files
}

#[cfg(test)]
mod tests {
    use std::fs;

    use libcdept::PROJECT_FILE;

    use super::*;
    use crate::log::LogBuffer;
    use crate::state::Settings;

    #[test]
    fn the_dialog_plans_and_imports_files() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("libs/lib")).unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"demo\"\n[libraries]\nlib = \"libs/lib\"\n",
        )
        .unwrap();
        fs::write(
            root.join("in.spice"),
            ".subckt inv A Y\nR1 A Y 1\n.ends\n.subckt buf A Y\nX1 A Y inv\n.ends\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(&root);
        let mut import = ImportFiles::new("lib");
        import.set("add", "in.spice").unwrap();
        import.set("prefix", "p_").unwrap();
        assert!(import.set("bogus", "x").is_err());
        let project = app.project.as_ref().unwrap();
        assert_eq!(paths(&import, project), [root.join("in.spice")]);
        let lines = plan_lines(&app, &import).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("p_buf/netlist/p_buf.spice")),
            "{lines:#?}"
        );
        apply(&mut app, &import).unwrap();
        assert!(root.join("libs/lib/p_inv/netlist/p_inv.spice").is_file());
        assert_eq!(app.selection.library.as_deref(), Some("lib"));
        assert!(matches!(
            crate::dialogs::open(&app),
            Some(Dialog::ImportSummary(lines)) if lines[0].contains("2 view(s) of 2 cell(s)")
        ));
        // Importing again keeps what is there: nothing is left to import.
        let error = apply(&mut app, &import).unwrap_err().to_string();
        assert!(error.contains("nothing to import"), "{error}");
    }
}
