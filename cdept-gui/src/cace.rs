//! Import CACE Datasheet… / Export CACE Datasheet…: a cell's testbenches and suite from
//! and to a CACE datasheet (`libcdept::cace`, docs/reference/files/cace.md).
//!
//! Both dialogs have a path field (with Browse…) and show below the fields what will
//! happen: for an import the plan and the report of `cdept cace import` (what was
//! mapped, what is only kept, what to look at), for an export the parameters, the notes
//! and the first lines of the datasheet. Confirming writes, then a summary dialog
//! ([`Dialog::ImportSummary`]) says what was written.

use egui::Ui;
use eyre::{Result, eyre};
use libcdept::cace::{export, import};
use libcdept::{Cell, Library, Project};

use crate::dialogs::Dialog;
use crate::pathfield::PathField;
use crate::state::{AppCtx, Selection};

/// The state of "Import CACE Datasheet".
#[derive(Debug, Clone, PartialEq)]
pub struct CaceImport {
    pub library: String,
    pub cell: String,
    /// The datasheet, as typed or browsed (relative to the project folder).
    pub file: String,
    pub prefix: String,
    /// `library/cell` of the testbench schematic; empty: the cell's own.
    pub schematic: String,
    pub overwrite: bool,
}

/// The state of "Export CACE Datasheet".
#[derive(Debug, Clone, PartialEq)]
pub struct CaceExport {
    pub library: String,
    pub cell: String,
    /// The suite or testbench view; empty: the cell's only suite, else its only testbench.
    pub view: String,
    /// Where the datasheet goes (relative to the project folder).
    pub file: String,
    pub results: bool,
}

fn on(v: &str) -> bool {
    matches!(v, "yes" | "on" | "true")
}

impl CaceImport {
    pub fn new(library: &str, cell: &str) -> Self {
        CaceImport {
            library: library.into(),
            cell: cell.into(),
            file: String::new(),
            prefix: String::new(),
            schematic: String::new(),
            overwrite: false,
        }
    }

    /// Set a field by name, for test scripts.
    pub(crate) fn set(&mut self, field: &str, value: &str) -> Result<(), String> {
        match field {
            "file" => self.file = value.into(),
            "cell" => self.cell = value.into(),
            "prefix" => self.prefix = value.into(),
            "schematic" => self.schematic = value.into(),
            "overwrite" => self.overwrite = on(value),
            other => return Err(format!("this dialog has no field '{other}'")),
        }
        Ok(())
    }

    fn plan(&self, project: &Project) -> Result<import::ImportPlan> {
        let file = crate::pathfield::resolve(self.file.trim(), Some(&project.root), None)
            .ok_or_else(|| eyre!("choose the datasheet (a CACE .yaml file)"))?;
        let options = import::ImportOptions {
            library: self.library.clone(),
            cell: self.cell.trim().to_string(),
            prefix: self.prefix.trim().to_string(),
            schematic: self.schematic.trim().to_string(),
            overwrite: self.overwrite,
            source: self.file.trim().to_string(),
        };
        import::plan_file(project, &file, &options)
    }
}

impl CaceExport {
    pub fn new(library: &str, cell: &str, view: &str) -> Self {
        CaceExport {
            library: library.into(),
            cell: cell.into(),
            view: view.into(),
            file: format!("{cell}.yaml"),
            results: true,
        }
    }

    /// Set a field by name, for test scripts.
    pub(crate) fn set(&mut self, field: &str, value: &str) -> Result<(), String> {
        match field {
            "file" => self.file = value.into(),
            "view" => self.view = value.into(),
            "results" => self.results = on(value),
            other => return Err(format!("this dialog has no field '{other}'")),
        }
        Ok(())
    }

    fn export(&self, project: &Project) -> Result<export::Exported> {
        let options = export::ExportOptions {
            view: (!self.view.trim().is_empty()).then(|| self.view.trim().to_string()),
            results: self.results,
        };
        export::export(project, &self.library, &self.cell, &options)
    }

    fn lines(&self, project: &Project) -> Result<Vec<String>> {
        let out = self.export(project)?;
        let mut lines = vec![format!(
            "write {}: {} parameter(s) of {}/{}/{} ({})",
            self.file.trim(),
            out.parameters.len(),
            self.library,
            self.cell,
            out.view,
            if out.suite { "suite" } else { "testbench" }
        )];
        lines.push(format!("  {}", out.parameters.join(", ")));
        lines.extend(out.notes.iter().map(|n| format!("note: {n}")));
        lines.push(String::new());
        lines.extend(out.text.lines().take(14).map(String::from));
        Ok(lines)
    }
}

fn plan_lines_of(project: &Project, import: &CaceImport) -> Result<Vec<String>, String> {
    import.plan(project).map(|p| p.lines(&project.root)).map_err(|e| format!("{e:#}"))
}

/// The lines below the fields, for the dialog and the test scripts.
pub(crate) fn plan_lines(app: &AppCtx, dialog: &Dialog) -> Option<Result<Vec<String>, String>> {
    let project = app.project.as_ref()?;
    match dialog {
        Dialog::CaceImport(i) => Some(plan_lines_of(project, i)),
        Dialog::CaceExport(e) => Some(e.lines(project).map_err(|e| format!("{e:#}"))),
        _ => None,
    }
}

fn show_lines(ui: &mut Ui, lines: Result<Vec<String>, String>) {
    ui.add_space(6.0);
    match lines {
        Ok(lines) => {
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for line in lines {
                    let text = egui::RichText::new(line.as_str()).small().monospace();
                    if line.trim_start().starts_with("blocked") || line.starts_with("warnings") {
                        ui.colored_label(ui.visuals().warn_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                }
            });
        }
        Err(e) => {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
    }
}

/// The fields of the import dialog and the plan below them.
pub fn import_body(ui: &mut Ui, import: &mut CaceImport, app: &AppCtx) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    egui::Grid::new("cace-import-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Datasheet");
        PathField::file("cace-import-file", &mut import.file)
            .base(&project.root)
            .title("Choose a CACE datasheet")
            .width(260.0)
            .show(ui);
        ui.end_row();
        ui.label("Into");
        ui.horizontal(|ui| {
            ui.label(format!("{} /", import.library));
            ui.add(egui::TextEdit::singleline(&mut import.cell).desired_width(140.0));
        });
        ui.end_row();
        ui.label("View prefix");
        ui.add(egui::TextEdit::singleline(&mut import.prefix).desired_width(140.0))
            .on_hover_text("Put in front of every view name");
        ui.end_row();
        ui.label("Schematic");
        ui.add(egui::TextEdit::singleline(&mut import.schematic).hint_text("the cell's own").desired_width(180.0))
            .on_hover_text("library/cell of the testbench schematic every testbench netlists");
        ui.end_row();
    });
    ui.checkbox(&mut import.overwrite, "Replace views that exist");
    if import.file.trim().is_empty() {
        ui.add_space(6.0);
        ui.weak("Choose a CACE datasheet (YAML, cace_format 5.x).");
        return;
    }
    show_lines(ui, plan_lines_of(project, import));
}

/// The fields of the export dialog and what it will write.
pub fn export_body(ui: &mut Ui, export: &mut CaceExport, app: &AppCtx) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    egui::Grid::new("cace-export-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Of");
        ui.horizontal(|ui| {
            ui.label(format!("{}/{} /", export.library, export.cell));
            ui.add(egui::TextEdit::singleline(&mut export.view).hint_text("its suite").desired_width(140.0));
        });
        ui.end_row();
        ui.label("Datasheet");
        PathField::save_file("cace-export-file", &mut export.file)
            .base(&project.root)
            .title("Write the CACE datasheet to")
            .width(260.0)
            .show(ui);
        ui.end_row();
    });
    ui.checkbox(&mut export.results, "With the results of the latest runs");
    show_lines(ui, export.lines(project).map_err(|e| format!("{e:#}")));
}

/// Import: write the views, read the library again, select the suite, say what was written.
pub fn apply_import(app: &mut AppCtx, import: &CaceImport) -> Result<()> {
    let project = app.project.as_mut().ok_or_else(|| eyre!("no project is open"))?;
    let plan = import.plan(project)?;
    let lines = plan.lines(&project.root);
    let written = import::apply(&plan)?;
    for line in &lines {
        tracing::info!(target: "cdept::library", "{line}");
    }
    project.rescan();
    app.refactor.epoch += 1;
    app.selection = Selection::view(&plan.library, &plan.cell, &plan.suite_view);
    app.validate_selection();
    let mut summary = vec![format!("Imported {} file(s) into {}/{}.", written.len(), plan.library, plan.cell)];
    summary.extend(lines.into_iter().skip(1));
    app.open_dialog(Dialog::ImportSummary(summary));
    Ok(())
}

/// Export: write the datasheet, say where.
pub fn apply_export(app: &mut AppCtx, export: &CaceExport) -> Result<()> {
    let project = app.project.as_ref().ok_or_else(|| eyre!("no project is open"))?;
    let file = crate::pathfield::resolve(export.file.trim(), Some(&project.root), None)
        .ok_or_else(|| eyre!("choose where the datasheet goes"))?;
    let out = export.export(project)?;
    std::fs::write(&file, &out.text).map_err(|e| eyre!("cannot write {}: {e}", file.display()))?;
    tracing::info!(target: "cdept::library", "wrote the CACE datasheet {}", file.display());
    let mut summary = vec![format!(
        "Exported {} parameter(s) of {}/{}/{} to {}.",
        out.parameters.len(),
        export.library,
        export.cell,
        out.view,
        file.display()
    )];
    summary.extend(out.notes.iter().map(|n| format!("note: {n}")));
    app.open_dialog(Dialog::ImportSummary(summary));
    Ok(())
}

/// Entries of a cell's context menu: import into it, export its suite or testbench.
pub fn menu(ui: &mut Ui, lib: &Library, cell: &Cell) -> Option<Dialog> {
    let rw = !lib.read_only && lib.error.is_none() && lib.layout == libcdept::library::LibraryLayout::Cells;
    let mut dialog = None;
    if ui
        .add_enabled(rw, egui::Button::new("Import CACE Datasheet…"))
        .on_hover_text("Testbench views and a suite of this cell from a CACE datasheet (YAML)")
        .on_disabled_hover_text("Needs a writable cells library")
        .clicked()
    {
        dialog = Some(Dialog::CaceImport(CaceImport::new(&lib.name, &cell.name)));
    }
    let runnable = cell.entries.iter().any(|e| {
        e.path.join(libcdept::sim::suite::SUITE_FILE).is_file() || e.path.join(libcdept::sim::setup::SETUP_FILE).is_file()
    });
    if ui
        .add_enabled(runnable, egui::Button::new("Export CACE Datasheet…"))
        .on_hover_text("This cell's suite or testbench, with the latest results, as a CACE datasheet")
        .on_disabled_hover_text("The cell has no suite or testbench")
        .clicked()
    {
        dialog = Some(Dialog::CaceExport(CaceExport::new(&lib.name, &cell.name, "")));
    }
    dialog
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::log::LogBuffer;
    use crate::state::Settings;

    #[test]
    fn the_dialogs_import_and_export() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("libs/cells")).unwrap();
        fs::write(
            root.join(libcdept::PROJECT_FILE),
            "name = \"demo\"\n[libraries]\ncells = { path = \"libs/cells\", layout = \"cells\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("sheet.yaml"),
            "name: amp\ndescription: d\nPDK: x\ncace_format: 5.2\nparameters:\n  gain:\n    spec:\n      a0:\n        minimum:\n          value: 40\n    tool:\n      ngspice:\n        template: ac.sch\n    conditions:\n      vdd:\n        typical: 1.2\n  drc:\n    tool: magic_drc\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(&root);
        let mut import = CaceImport::new("cells", "amp");
        import.set("file", "sheet.yaml").unwrap();
        assert!(import.set("bogus", "x").is_err());
        let lines = plan_lines(&app, &Dialog::CaceImport(import.clone())).unwrap().unwrap();
        assert!(lines.iter().any(|l| l.contains("amp/gain/setup.toml")), "{lines:#?}");
        assert!(lines.iter().any(|l| l.contains("drc: tool magic_drc")), "{lines:#?}");
        apply_import(&mut app, &import).unwrap();
        assert!(root.join("libs/cells/amp/datasheet/suite.toml").is_file());
        assert_eq!(app.selection.view.as_deref(), Some("datasheet"));
        let mut export = CaceExport::new("cells", "amp", "");
        export.set("file", "back.yaml").unwrap();
        let lines = plan_lines(&app, &Dialog::CaceExport(export.clone())).unwrap().unwrap();
        assert!(lines[0].contains("2 parameter(s)"), "{lines:#?}");
        apply_export(&mut app, &export).unwrap();
        let back = fs::read_to_string(root.join("back.yaml")).unwrap();
        assert!(back.contains("magic_drc") && back.contains("value: 40"), "{back}");
    }
}
