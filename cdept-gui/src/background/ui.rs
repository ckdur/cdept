//! What the background work shows in the window: the status bar part, the
//! "changed on disk" notice of a document, the trash note of the delete dialogs, the
//! recent projects that are gone.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use libcdept::PROJECT_FILE;
use libcdept::trash::{Removal, Trash};
use libcdept::vcs::State;

use super::{Background, disk};
use crate::state::AppCtx;
use crate::views::Document;

/// Status bar: what is being read, the git branch and how much is not committed.
pub(crate) fn status_ui(ui: &mut Ui, app: &AppCtx) {
    let background = &app.background;
    if let Some(root) = background.loading() {
        ui.separator();
        ui.spinner();
        ui.label(format!("Opening {}…", disk::name(root)));
        return;
    }
    if background.scanning() || background.reloading {
        ui.separator();
        ui.spinner();
        ui.weak("scanning…");
    }
    let Some(project) = &app.project else {
        return;
    };
    let status = background.git();
    if let Some(repository) = status
        .repository(&project.root)
        .or(status.repositories.first())
    {
        ui.separator();
        let dirty = status.dirty();
        let text = match dirty {
            0 => format!("git: {}", repository.head()),
            n => format!("git: {} · {n} changed", repository.head()),
        };
        let label = if dirty > 0 {
            ui.colored_label(crate::views::vcs::color(ui, State::Modified), text)
        } else {
            ui.label(text)
        };
        label.on_hover_ui(|ui| {
            for repository in &status.repositories {
                ui.strong(repository.root.display().to_string());
                ui.label(format!("on {}", repository.head()));
                if let Some(upstream) = &repository.upstream {
                    ui.weak(format!("follows {upstream}"));
                }
                match repository.dirty() {
                    0 => ui.weak("nothing to commit"),
                    n => ui.label(format!("{n} file(s) or folder(s) not as committed")),
                };
            }
        });
    }
    if app.settings.watch && !background.watching() {
        ui.separator();
        ui.colored_label(ui.visuals().warn_fg_color, "not watching")
            .on_hover_text(
                "Changes on disk are not followed (see the Console). Use Project ▸ Reload.",
            );
    }
}

/// "Opening …" where a view would say there is no project. `true` when it was shown.
pub(crate) fn loading_ui(ui: &mut Ui, app: &AppCtx) -> bool {
    let Some(root) = app.background.loading() else {
        return false;
    };
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(format!("Opening {}…", root.display()));
    });
    true
}

/// Project menu: the two settings.
pub(crate) fn menu_ui(ui: &mut Ui, app: &mut AppCtx) {
    ui.separator();
    ui.checkbox(&mut app.settings.watch, "Follow Changes on Disk")
        .on_hover_text(
            "Show what other programs change in the libraries and in cdept.toml without \
             Reload, and keep the git marks up to date",
        );
    ui.add_enabled(
        app.settings.watch,
        egui::Checkbox::new(&mut app.settings.auto_reload, "Reload Unchanged Documents"),
    )
    .on_hover_text(
        "A schematic or layout tab without changes of its own opens its file again when \
         the file changed on disk. Off: it asks, like a tab with changes.",
    );
}

/// An entry of the recent projects: a button (menu) or a link (start page). A project
/// whose `cdept.toml` is gone is shown as such; choosing it removes it from the list.
/// `true` when chosen.
pub(crate) fn recent_button(
    ui: &mut Ui,
    background: &mut Background,
    path: &Path,
    link: bool,
) -> bool {
    let now = Instant::now();
    let there = match background.recent.get(path) {
        Some((at, there)) if now.duration_since(*at) < Duration::from_secs(2) => *there,
        _ => {
            let there = path.join(PROJECT_FILE).is_file();
            background.recent.insert(path.to_path_buf(), (now, there));
            there
        }
    };
    let text = path.display().to_string();
    if there {
        if link {
            ui.link(text).clicked()
        } else {
            ui.button(text).clicked()
        }
    } else {
        let text = RichText::new(format!("{text}  (not found)")).weak();
        let response = if link { ui.link(text) } else { ui.button(text) };
        response
            .on_hover_text(
                "There is no project in this folder any more. Click to remove it from the list.",
            )
            .clicked()
    }
}

/// The delete dialogs: where the files go, what git has of them, and the choice to
/// delete for good.
pub(crate) fn delete_note(
    ui: &mut Ui,
    app: &AppCtx,
    permanent: &mut bool,
    library: &str,
    cell: &str,
    view: Option<&str>,
) {
    let has_trash = Trash::home().is_ok();
    if !has_trash {
        *permanent = true;
    }
    let what = if view.is_some() {
        "Its files"
    } else {
        "All its views"
    };
    if *permanent {
        ui.label(RichText::new(format!("{what} are deleted from disk for good.")).strong());
    } else {
        ui.label(format!(
            "{what} are moved to the trash; the desktop's file manager can put them back."
        ));
    }

    // What git has of it.
    let found = app
        .project
        .as_ref()
        .and_then(|p| p.library(library))
        .and_then(|l| l.cell(cell));
    let summary = found.and_then(|cell| match view {
        Some(view) => app.background.git().entry(cell.entry(view)?),
        None => app.background.git().cell(cell),
    });
    if let Some(summary) = summary {
        let lost = summary.modified + summary.untracked + summary.added + summary.staged;
        if summary.ignored || (summary.untracked > 0 && lost == summary.untracked) {
            ui.weak("Git does not have these files.");
        } else if lost == 0 && summary.conflicted == 0 {
            ui.weak("Tracked by git and as committed: `git restore` brings it back as well.");
        } else {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "Git has the committed version only: {} here is not committed.",
                    summary.describe()
                ),
            );
        }
    }

    ui.add_space(4.0);
    ui.add_enabled(
        has_trash,
        egui::Checkbox::new(permanent, "Delete permanently (do not use the trash)"),
    )
    .on_disabled_hover_text("There is no trash on this system.");
}

/// How the delete that the dialog is about to run deletes.
pub(crate) fn removal(permanent: bool) -> Removal {
    if permanent {
        Removal::Permanent
    } else {
        Removal::Trash
    }
}

/// The notice above a document whose file changed on disk behind its back.
pub(crate) fn stale_ui(doc: &mut Document, ui: &mut Ui, app: &mut AppCtx) {
    let file = doc.file().to_path_buf();
    let Some(stale) = app.background.docs.stale(&file).cloned() else {
        return;
    };
    let names: Vec<String> = stale.files.iter().map(|f| disk::name(f)).collect();
    let modified = doc.is_modified();
    let mut answer = None;
    egui::Frame::new()
        .fill(ui.visuals().warn_fg_color.gamma_multiply(0.18))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let what = if stale.gone {
                    format!("⚠ {} was deleted on disk.", disk::name(&file))
                } else {
                    format!("⚠ {} changed on disk.", names.join(", "))
                };
                ui.label(RichText::new(what).strong());
                if stale.gone {
                    ui.label("Saving writes it again.");
                    if ui.button("OK").clicked() {
                        answer = Some(false);
                    }
                    return;
                }
                if modified {
                    ui.label("This tab has changes that are not saved.");
                }
                let reload = if modified {
                    "Reload (drop my changes)"
                } else {
                    "Reload"
                };
                if ui
                    .button(reload)
                    .on_hover_text("Open the file again as it is on disk")
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui
                    .button("Keep Mine")
                    .on_hover_text("Keep what this tab shows; saving writes over the file on disk")
                    .clicked()
                {
                    answer = Some(false);
                }
            });
        });
    if let Some(reload) = answer {
        answer_stale(doc, app, reload);
    }
}

/// Answer the notice of `doc`: reload, or keep what the tab shows.
pub(crate) fn answer_stale(doc: &mut Document, app: &mut AppCtx, reload: bool) {
    let file = doc.file().to_path_buf();
    if reload {
        disk::reload(doc);
        tracing::info!(target: super::TARGET, "{} opened again", disk::name(&file));
    }
    app.background.docs.answered(&file);
}

/// Whether `doc` shows the "changed on disk" notice.
pub(crate) fn is_stale(doc: &Document, app: &AppCtx) -> bool {
    app.background.docs.stale(doc.file()).is_some()
}
