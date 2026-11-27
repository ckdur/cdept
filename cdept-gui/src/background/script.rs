//! Commands of the test driver (`cdept gui --script`, docs/GUI-TESTING.md) for what
//! happens on disk: changing files from outside, what the library views then hold,
//! git marks, the trash, the "changed on disk" notice of a document.

use std::path::PathBuf;
use std::process::Command;

use libcdept::vcs::State;

use crate::dialogs::Dialog;
use crate::state::{Action, AppCtx};
use crate::views::Document;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DiskCommand {
    /// Run a shell command in the project folder, as another program would.
    Shell(String),
    /// A cell or a view is (or is not) listed in a library.
    ExpectListed(Vec<String>, bool),
    /// The git state of a library, cell or view (`None`: not in a repository).
    ExpectGit(Option<State>, Vec<String>),
    /// The branch of the project's repository.
    ExpectBranch(String),
    /// Open the Delete Cell / Delete View dialog.
    DeleteDialog(String, String, Option<String>),
    /// Confirm the open dialog, as its button does (`true`: delete permanently).
    ConfirmDialog(bool),
    /// The last delete moved something to the trash whose path ended in this.
    ExpectTrashed(String),
    /// The document shows (or not) the "changed on disk" notice.
    ExpectStale(bool),
    /// Answer the notice: reload, or keep what the tab shows.
    StaleAnswer(bool),
    /// The document has (or not) changes of its own.
    ExpectChanged(bool),
    /// Project ▸ Reload.
    ReloadProject,
    /// Open another project.
    LoadProject(PathBuf),
    /// "Follow Changes on Disk" / "Reload Unchanged Documents".
    Setting(String, bool),
}

impl DiskCommand {
    /// Tried again for a while before it fails (it does nothing when it fails).
    pub fn patient(&self) -> bool {
        matches!(
            self,
            DiskCommand::ExpectListed(..)
                | DiskCommand::ExpectGit(..)
                | DiskCommand::ExpectBranch(_)
                | DiskCommand::ExpectTrashed(_)
                | DiskCommand::ExpectStale(_)
                | DiskCommand::ExpectChanged(_)
        )
    }
}

fn state(name: &str) -> Result<Option<State>, String> {
    Ok(Some(match name {
        "none" => return Ok(None),
        "clean" => State::Clean,
        "ignored" => State::Ignored,
        "staged" => State::Staged,
        "added" => State::Added,
        "untracked" => State::Untracked,
        "modified" => State::Modified,
        "conflict" => State::Conflicted,
        other => return Err(format!("unknown git state '{other}'")),
    }))
}

fn on_off(word: &str) -> Result<bool, String> {
    match word {
        "on" | "yes" => Ok(true),
        "off" | "no" => Ok(false),
        other => Err(format!("expected on or off, not '{other}'")),
    }
}

/// Parse a command of this module; `Ok(None)` when `name` is not one.
pub(crate) fn parse(name: &str, rest: &str) -> Result<Option<DiskCommand>, String> {
    let args: Vec<&str> = rest.split_whitespace().collect();
    let text = |i: usize| args.get(i).map(|a| a.to_string());
    let command = match name {
        "shell" if !rest.trim().is_empty() => DiskCommand::Shell(rest.trim().to_string()),
        "shell" => return Err("usage: shell <command>".into()),
        "expect_listed" | "expect_not_listed" => match args.as_slice() {
            [_, _] | [_, _, _] => DiskCommand::ExpectListed(
                args.iter().map(|a| a.to_string()).collect(),
                name == "expect_listed",
            ),
            _ => return Err(format!("usage: {name} <library> <cell> [<view>]")),
        },
        "expect_git" => match args.as_slice() {
            [what, path @ ..] if (1..=3).contains(&path.len()) => {
                DiskCommand::ExpectGit(state(what)?, path.iter().map(|a| a.to_string()).collect())
            }
            _ => return Err("usage: expect_git <state> <library> [<cell> [<view>]]".into()),
        },
        "expect_branch" => DiskCommand::ExpectBranch(text(0).ok_or("usage: expect_branch <name>")?),
        "delete_dialog" => match args.as_slice() {
            [library, cell] => {
                DiskCommand::DeleteDialog(library.to_string(), cell.to_string(), None)
            }
            [library, cell, view] => DiskCommand::DeleteDialog(
                library.to_string(),
                cell.to_string(),
                Some(view.to_string()),
            ),
            _ => return Err("usage: delete_dialog <library> <cell> [<view>]".into()),
        },
        "confirm_dialog" => match args.as_slice() {
            [] => DiskCommand::ConfirmDialog(false),
            ["permanent"] => DiskCommand::ConfirmDialog(true),
            _ => return Err("usage: confirm_dialog [permanent]".into()),
        },
        "expect_trashed" => {
            DiskCommand::ExpectTrashed(text(0).ok_or("usage: expect_trashed <path ending>")?)
        }
        "expect_stale" => DiskCommand::ExpectStale(on_off(args.first().copied().unwrap_or(""))?),
        "stale_answer" => match args.as_slice() {
            ["reload"] => DiskCommand::StaleAnswer(true),
            ["keep"] => DiskCommand::StaleAnswer(false),
            _ => return Err("usage: stale_answer <reload|keep>".into()),
        },
        "expect_changed" => {
            DiskCommand::ExpectChanged(on_off(args.first().copied().unwrap_or(""))?)
        }
        "reload_project" => DiskCommand::ReloadProject,
        "load_project" if !rest.trim().is_empty() => {
            DiskCommand::LoadProject(PathBuf::from(rest.trim()))
        }
        "setting" => match args.as_slice() {
            [name @ ("watch" | "auto_reload"), value] => {
                DiskCommand::Setting(name.to_string(), on_off(value)?)
            }
            _ => return Err("usage: setting <watch|auto_reload> <on|off>".into()),
        },
        _ => return Ok(None),
    };
    Ok(Some(command))
}

/// Run a command. `request` hands an [`Action`] to the shell.
pub(crate) fn run(
    command: DiskCommand,
    app: &mut AppCtx,
    doc: Option<&mut Document>,
    egui: &egui::Context,
    request: &mut Vec<Action>,
) -> Result<(), String> {
    match command {
        DiskCommand::Shell(line) => {
            // without a project: the current folder
            let root = match app.project.as_ref() {
                Some(p) => p.root.clone(),
                None => {
                    std::env::current_dir().map_err(|e| format!("no project, no folder: {e}"))?
                }
            };
            let output = Command::new("sh")
                .arg("-c")
                .arg(&line)
                .current_dir(&root)
                .env("PROJECT", &root)
                .output()
                .map_err(|e| format!("sh: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "`{line}` failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        DiskCommand::ExpectListed(path, wanted) => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let library = project
                .library(&path[0])
                .ok_or(format!("no library '{}'", path[0]))?;
            let cell = library.cell(&path[1]);
            let found = match path.get(2) {
                Some(view) => cell.is_some_and(|c| c.entry(view).is_some()),
                None => cell.is_some(),
            };
            if found != wanted {
                return Err(if wanted {
                    format!("{} is not listed", path.join("/"))
                } else {
                    format!("{} is still listed", path.join("/"))
                });
            }
        }
        DiskCommand::ExpectGit(wanted, path) => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let status = app.background.git();
            let library = project
                .library(&path[0])
                .ok_or(format!("no library '{}'", path[0]))?;
            let summary = match path.as_slice() {
                [_] => status.library(library),
                [_, cell] => {
                    let cell = library.cell(cell).ok_or(format!("no cell '{cell}'"))?;
                    status.cell(cell)
                }
                [_, cell, view] => {
                    let entry = library
                        .cell(cell)
                        .and_then(|c| c.entry(view))
                        .ok_or(format!("no view '{cell}/{view}'"))?;
                    status.entry(entry)
                }
                _ => None,
            };
            let found = summary.map(|s| s.state());
            if found != wanted {
                let text = |s: Option<State>| s.map_or("none", State::label);
                return Err(format!(
                    "git says {} about {}, not {}",
                    text(found),
                    path.join("/"),
                    text(wanted)
                ));
            }
        }
        DiskCommand::ExpectBranch(name) => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let status = app.background.git();
            let head = status
                .repository(&project.root)
                .map(|r| r.head())
                .ok_or("the project is not in a git repository (as far as is known yet)")?;
            if !head.contains(&name) {
                return Err(format!("the repository is on '{head}'"));
            }
        }
        DiskCommand::DeleteDialog(library, cell, view) => {
            app.open_dialog(match view {
                Some(view) => Dialog::DeleteView {
                    library,
                    cell,
                    view,
                },
                None => Dialog::DeleteCell { library, cell },
            });
        }
        DiskCommand::ConfirmDialog(permanent) => crate::dialogs::confirm(app, permanent)?,
        DiskCommand::ExpectTrashed(ending) => {
            let found = app
                .background
                .trashed
                .iter()
                .find(|t| t.original.ends_with(&ending))
                .ok_or(format!("nothing ending in '{ending}' went to the trash"))?;
            if !found.file.exists() || !found.info.is_file() {
                return Err(format!("{} is not in the trash", found.file.display()));
            }
            if found.original.exists() {
                return Err(format!("{} is still there", found.original.display()));
            }
            tracing::info!(target: "cdept::script", "in the trash: {}", found.file.display());
        }
        DiskCommand::ExpectStale(wanted) => {
            let doc = doc.ok_or("no document")?;
            if super::ui::is_stale(doc, app) != wanted {
                return Err(if wanted {
                    "the document shows no 'changed on disk' notice".into()
                } else {
                    "the document shows the 'changed on disk' notice".into()
                });
            }
        }
        DiskCommand::StaleAnswer(reload) => {
            let doc = doc.ok_or("no document")?;
            if !super::ui::is_stale(doc, app) {
                return Err("the document shows no 'changed on disk' notice".into());
            }
            super::ui::answer_stale(doc, app, reload);
        }
        DiskCommand::ExpectChanged(wanted) => {
            let doc = doc.ok_or("no document")?;
            if doc.is_modified() != wanted {
                return Err(if wanted {
                    "the document has no changes of its own".into()
                } else {
                    "the document has changes of its own".into()
                });
            }
        }
        DiskCommand::ReloadProject => super::reload(app, egui),
        DiskCommand::LoadProject(path) => request.push(Action::LoadProject(path)),
        DiskCommand::Setting(name, value) => match name.as_str() {
            "watch" => app.settings.watch = value,
            _ => app.settings.auto_reload = value,
        },
    }
    Ok(())
}
