//! Git status of a project, with a real repository in a temporary folder.

use std::fs;
use std::path::Path;
use std::process::Command;

use libcdept::vcs::{State, Status};
use libcdept::{PROJECT_FILE, Project};

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=CDePT test",
            "-c",
            "user.email=test@example.com",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A project with a `cells` library and a flat one, committed.
fn project(root: &Path) -> Option<Project> {
    fs::create_dir_all(root).unwrap();
    if !git(root, &["init", "-q"]) {
        eprintln!("skipped: no git");
        return None;
    }
    write(&root.join("libs/a/inv/schematic/inv.sch"), "v {}\n");
    write(&root.join("libs/a/inv/symbol/inv.sym"), "v {}\n");
    write(&root.join("libs/a/buf/schematic/buf.sch"), "v {}\n");
    write(&root.join("libs/a/gone/schematic/gone.sch"), "v {}\n");
    write(&root.join("libs/a/gone/symbol/gone.sym"), "v {}\n");
    write(&root.join("libs/flat/r.sym"), "v {}\n");
    write(&root.join("libs/flat/r.sch"), "v {}\n");
    write(&root.join("libs/flat/c.sym"), "v {}\n");
    write(&root.join(".gitignore"), "simulations/\n*.raw\n");
    write(
        &root.join(PROJECT_FILE),
        "name = \"t\"\n[libraries]\na = { path = \"libs/a\", layout = \"cells\" }\nflat = \"libs/flat\"\n",
    );
    assert!(git(root, &["add", "."]));
    assert!(git(root, &["commit", "-q", "-m", "first"]));
    Some(Project::load(root).unwrap())
}

#[test]
fn a_clean_project_is_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(project) = project(&tmp.path().join("p")) else {
        return;
    };
    let status = Status::of_project(&project);
    assert_eq!(status.repositories.len(), 1);
    let repository = &status.repositories[0];
    assert_eq!(repository.root, project.root);
    assert_eq!(repository.branch.as_deref(), Some("main"));
    assert!(repository.commit.is_some());
    assert_eq!(status.dirty(), 0);
    for library in &project.libraries {
        assert_eq!(status.library(library).unwrap().state(), State::Clean);
        for cell in &library.cells {
            assert_eq!(
                status.cell(cell).unwrap().state(),
                State::Clean,
                "{}",
                cell.name
            );
        }
    }
}

#[test]
fn changes_show_per_view_cell_and_library() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(mut project) = project(&tmp.path().join("p")) else {
        return;
    };
    let root = project.root.clone();
    // Modified, staged, added, untracked, deleted, ignored.
    write(&root.join("libs/a/inv/schematic/inv.sch"), "v {}\nC {x}\n");
    write(&root.join("libs/a/inv/symbol/inv.sym"), "v {}\nG {}\n");
    assert!(git(&root, &["add", "libs/a/inv/symbol/inv.sym"]));
    write(&root.join("libs/a/nand/schematic/nand.sch"), "v {}\n");
    write(&root.join("libs/a/nor/schematic/nor.sch"), "v {}\n");
    assert!(git(&root, &["add", "libs/a/nor"]));
    fs::remove_file(root.join("libs/a/gone/schematic/gone.sch")).unwrap();
    write(&root.join("libs/a/buf/tb/results.raw"), "x");
    write(&root.join("libs/flat/r.sch"), "v {}\nchanged\n");
    write(&root.join("libs/flat/l.sym"), "v {}\n");
    project.rescan();

    let status = Status::of_project(&project);
    let a = project.library("a").unwrap();
    let cell = |name: &str| status.cell(a.cell(name).unwrap()).unwrap();
    let view = |cell: &str, view: &str| {
        status
            .entry(a.cell(cell).unwrap().entry(view).unwrap())
            .unwrap()
            .state()
    };

    assert_eq!(view("inv", "schematic"), State::Modified);
    assert_eq!(view("inv", "symbol"), State::Staged);
    assert_eq!(cell("inv").state(), State::Modified);
    assert_eq!(cell("inv").describe(), "1 modified, 1 staged");
    assert_eq!(cell("nand").state(), State::Untracked);
    assert_eq!(view("nand", "schematic"), State::Untracked);
    assert_eq!(cell("nor").state(), State::Added);
    // The deleted schematic counts for its cell; the symbol that is left is clean.
    assert_eq!(cell("gone").state(), State::Modified);
    assert_eq!(view("gone", "symbol"), State::Clean);
    // An ignored file inside does not make the cell anything.
    assert_eq!(cell("buf").state(), State::Clean);
    assert_eq!(
        status.file(&root.join("libs/a/buf/tb/results.raw")),
        Some(State::Ignored)
    );
    assert_eq!(status.library(a).unwrap().state(), State::Modified);
    assert_eq!(status.library(a).unwrap().changed(), 5);

    // A flat library: a cell is the files sharing its name.
    let flat = project.library("flat").unwrap();
    let cell = |name: &str| status.cell(flat.cell(name).unwrap()).unwrap().state();
    assert_eq!(cell("r"), State::Modified);
    assert_eq!(cell("c"), State::Clean);
    assert_eq!(cell("l"), State::Untracked);

    assert_eq!(status.dirty(), 7);
    assert_eq!(status.repositories[0].head(), "main");
}

#[test]
fn a_library_can_be_a_repository_of_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(mut project) = project(&tmp.path().join("p")) else {
        return;
    };
    // A library outside the project, with its own repository on another branch.
    let own = tmp.path().join("shared/own");
    write(&own.join("x/schematic/x.sch"), "v {}\n");
    assert!(git(&own, &["init", "-q"]));
    assert!(git(&own, &["checkout", "-q", "-b", "devel"]));
    assert!(git(&own, &["add", "."]));
    assert!(git(&own, &["commit", "-q", "-m", "x"]));
    write(&own.join("x/schematic/x.sch"), "v {}\nchanged\n");
    // And one in no repository at all.
    let loose = tmp.path().join("loose");
    write(&loose.join("y.sym"), "v {}\n");
    project
        .add_library(
            "own",
            own.to_str().unwrap(),
            libcdept::AddLibrary::default(),
        )
        .unwrap();
    project
        .add_library(
            "loose",
            loose.to_str().unwrap(),
            libcdept::AddLibrary::default(),
        )
        .unwrap();

    let status = Status::of_project(&project);
    assert_eq!(status.repositories.len(), 2);
    let own_lib = project.library("own").unwrap();
    assert_eq!(status.repository(&own_lib.path).unwrap().head(), "devel");
    assert_eq!(status.library(own_lib).unwrap().state(), State::Modified);
    assert_eq!(status.cell(own_lib.cell("x").unwrap()).unwrap().modified, 1);
    // cdept.toml changed (two libraries added): the project's repository says so.
    assert_eq!(status.file(&project.file), Some(State::Modified));
    assert_eq!(status.repository(&project.root).unwrap().head(), "main");

    let loose_lib = project.library("loose").unwrap();
    assert!(status.library(loose_lib).is_none());
    assert!(status.cell(loose_lib.cell("y").unwrap()).is_none());
    assert_eq!(status.file(&loose.join("y.sym")), None);
}

#[test]
fn outside_a_repository_there_is_no_status() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("p");
    write(&root.join("libs/a/inv/schematic/inv.sch"), "v {}\n");
    write(
        &root.join(PROJECT_FILE),
        "name = \"t\"\n[libraries]\na = \"libs/a\"\n",
    );
    // The temporary folder may itself sit in a repository on some machines.
    if libcdept::watch::repository_of(&root).is_some() {
        return;
    }
    let project = Project::load(&root).unwrap();
    let status = Status::of_project(&project);
    assert!(status.is_empty());
    assert_eq!(status.dirty(), 0);
    assert!(status.library(&project.libraries[0]).is_none());
}
