//! The watcher on real folders: a project in a temporary folder, files changed the
//! way an editor, a tool or `git` changes them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use libcdept::watch::{Change, Options, Plan, Watcher};
use libcdept::{PROJECT_FILE, Project};

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(400);

fn options() -> Options {
    Options {
        delay: Duration::from_millis(60),
        max_delay: Duration::from_millis(500),
        reference_recursive: false,
    }
}

/// A project with a `cells` library `a`, a flat library `flat` and a read-only
/// library `pdk` outside the project folder.
fn project(tmp: &Path) -> Project {
    let root = tmp.join("proj");
    let pdk = tmp.join("pdk/prims");
    fs::create_dir_all(root.join("libs/a/inv/schematic")).unwrap();
    fs::create_dir_all(root.join("libs/flat")).unwrap();
    fs::create_dir_all(pdk.join("deep")).unwrap();
    fs::write(root.join("libs/a/inv/schematic/inv.sch"), "v {}\n").unwrap();
    fs::write(root.join("libs/flat/r.sym"), "v {}\n").unwrap();
    fs::write(pdk.join("nmos.sym"), "v {}\n").unwrap();
    fs::write(
        root.join(PROJECT_FILE),
        format!(
            "name = \"t\"\n[libraries]\na = {{ path = \"libs/a\", layout = \"cells\" }}\n\
             flat = \"libs/flat\"\npdk = {{ path = \"{}\", read_only = true }}\n",
            pdk.display()
        ),
    )
    .unwrap();
    Project::load(&root).unwrap()
}

fn library_change(changes: &[Change], library: &str) -> Option<Vec<PathBuf>> {
    changes.iter().find_map(|c| match c {
        Change::Library { name, files } if name == library => Some(files.clone()),
        _ => None,
    })
}

/// Wait for a change of `library`; other changes that come first are dropped.
fn expect_library(watcher: &Watcher, library: &str) -> Vec<PathBuf> {
    let deadline = std::time::Instant::now() + WAIT;
    while std::time::Instant::now() < deadline {
        if let Some(files) = library_change(&watcher.wait(Duration::from_millis(200)), library) {
            return files;
        }
    }
    panic!("no change of library '{library}' within {WAIT:?}");
}

fn expect_quiet(watcher: &Watcher) {
    let changes = watcher.wait(QUIET);
    assert!(changes.is_empty(), "unexpected changes: {changes:?}");
}

#[test]
fn the_plan_covers_the_project() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path());
    let plan = Plan::of(&project, &options());
    let root = &project.root;
    assert!(plan.files.contains(&root.join(PROJECT_FILE)));
    assert!(plan.files.contains(&root.join("tech.toml")));
    let folder = |path: PathBuf| plan.folders.iter().find(|f| f.path == path);
    assert!(!folder(root.clone()).unwrap().recursive);
    assert!(folder(root.join("libs/a")).unwrap().recursive);
    assert!(folder(root.join("libs/flat")).unwrap().recursive);
    // The reference library: its top folder only.
    let pdk = project.library("pdk").unwrap().path.clone();
    assert!(!folder(pdk.clone()).unwrap().recursive);
    let all = Options {
        reference_recursive: true,
        ..options()
    };
    let plan = Plan::of(&project, &all);
    assert!(plan.folders.iter().any(|f| f.path == pdk && f.recursive));
}

#[test]
fn changes_in_the_libraries_are_reported() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path());
    let root = project.root.clone();
    let watcher = Watcher::for_project(&project, options(), || {}).unwrap();
    assert!(watcher.problems().is_empty(), "{:?}", watcher.problems());
    expect_quiet(&watcher);

    // A new view file.
    let sym = root.join("libs/a/inv/symbol");
    fs::create_dir_all(&sym).unwrap();
    fs::write(sym.join("inv.sym"), "v {}\n").unwrap();
    let files = expect_library(&watcher, "a");
    assert!(files.iter().any(|f| f.starts_with(&sym)), "{files:?}");
    expect_quiet(&watcher);

    // A file written in a folder made a moment ago (the watch follows new folders).
    let deep = root.join("libs/a/buf/schematic");
    fs::create_dir_all(&deep).unwrap();
    expect_library(&watcher, "a");
    fs::write(deep.join("buf.sch"), "v {}\n").unwrap();
    assert!(expect_library(&watcher, "a").contains(&deep.join("buf.sch")));
    expect_quiet(&watcher);

    // Written again, renamed, removed.
    fs::write(root.join("libs/flat/r.sym"), "v {}\nG {}\n").unwrap();
    assert!(expect_library(&watcher, "flat").contains(&root.join("libs/flat/r.sym")));
    fs::rename(root.join("libs/flat/r.sym"), root.join("libs/flat/res.sym")).unwrap();
    let files = expect_library(&watcher, "flat");
    assert!(files.contains(&root.join("libs/flat/res.sym")), "{files:?}");
    fs::remove_file(root.join("libs/flat/res.sym")).unwrap();
    expect_library(&watcher, "flat");
    expect_quiet(&watcher);

    // What an editor leaves while it writes is not a change; reading is not either.
    fs::write(root.join("libs/flat/c.sym~"), "x").unwrap();
    fs::write(root.join("libs/flat/.c.sym.swp"), "x").unwrap();
    fs::read(root.join("libs/a/inv/schematic/inv.sch")).unwrap();
    expect_quiet(&watcher);

    // A sidecar next to a layout is a change.
    fs::write(root.join("libs/flat/c.gds.json"), "{}").unwrap();
    expect_library(&watcher, "flat");

    // The reference library: its top folder is watched, not what is below.
    let pdk = project.library("pdk").unwrap().path.clone();
    fs::write(pdk.join("deep/hidden.sym"), "v {}\n").unwrap();
    expect_quiet(&watcher);
    fs::write(pdk.join("pmos.sym"), "v {}\n").unwrap();
    expect_library(&watcher, "pdk");
}

#[test]
fn many_writes_give_one_change() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path());
    let watcher = Watcher::for_project(&project, options(), || {}).unwrap();
    for i in 0..20 {
        fs::write(project.root.join(format!("libs/flat/c{i}.sym")), "v {}\n").unwrap();
    }
    let files = expect_library(&watcher, "flat");
    assert_eq!(files.len(), 20, "{files:?}");
    expect_quiet(&watcher);
}

#[test]
fn the_project_files_are_watched() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path());
    let root = project.root.clone();
    let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = woken.clone();
    let watcher = Watcher::for_project(&project, options(), move || {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    })
    .unwrap();

    // Saved the way CDePT and most editors do: a temporary file, then a rename.
    let text = fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    fs::write(root.join("cdept.toml.tmp"), format!("{text}\n# edited\n")).unwrap();
    fs::rename(root.join("cdept.toml.tmp"), root.join(PROJECT_FILE)).unwrap();
    let changes = watcher.wait(WAIT);
    assert_eq!(
        changes,
        [Change::ProjectFile {
            file: root.join(PROJECT_FILE)
        }]
    );
    assert!(woken.load(std::sync::atomic::Ordering::SeqCst) >= 1);

    // tech.toml appearing next to it counts; any other file there does not.
    fs::write(root.join("notes.txt"), "x").unwrap();
    expect_quiet(&watcher);
    fs::write(root.join("tech.toml"), "schema = \"cdept-tech\"\n").unwrap();
    let changes = watcher.wait(WAIT);
    assert_eq!(
        changes,
        [Change::ProjectFile {
            file: root.join("tech.toml")
        }]
    );
}

#[test]
fn a_new_plan_watches_new_libraries() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(tmp.path());
    let root = project.root.clone();
    let mut watcher = Watcher::for_project(&project, options(), || {}).unwrap();
    fs::create_dir_all(root.join("libs/more")).unwrap();
    expect_quiet(&watcher);
    project
        .add_library("more", "libs/more", libcdept::AddLibrary::default())
        .unwrap();
    // CDePT's own save of cdept.toml.
    assert!(matches!(
        watcher.wait(WAIT).as_slice(),
        [Change::ProjectFile { .. }]
    ));
    watcher.set_plan(Plan::of(&project, &options()));
    fs::write(root.join("libs/more/x.sym"), "v {}\n").unwrap();
    expect_library(&watcher, "more");

    // And lets go of the ones that left.
    project.remove_library("flat").unwrap();
    watcher.wait(WAIT);
    watcher.set_plan(Plan::of(&project, &options()));
    fs::write(root.join("libs/flat/y.sym"), "v {}\n").unwrap();
    expect_quiet(&watcher);
}

#[test]
fn a_git_repository_reports_its_index() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path());
    let root = project.root.clone();
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !git(&["init", "-q"]) {
        eprintln!("skipped: no git");
        return;
    }
    let watcher = Watcher::for_project(&project, options(), || {}).unwrap();
    assert_eq!(watcher.plan().repositories.len(), 1);
    expect_quiet(&watcher);
    assert!(git(&["add", "libs"]));
    let changes = watcher.wait(WAIT);
    assert!(
        changes.contains(&Change::Repository { root: root.clone() }),
        "{changes:?}"
    );
    // What git keeps in its folder otherwise is not CDePT's business.
    fs::write(root.join(".git/description"), "x").unwrap();
    expect_quiet(&watcher);
}

#[test]
fn one_library_is_scanned_again() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(tmp.path());
    fs::write(project.root.join("libs/flat/c.sym"), "v {}\n").unwrap();
    fs::create_dir_all(project.root.join("libs/a/buf/schematic")).unwrap();
    assert!(project.rescan_library("flat"));
    assert!(!project.rescan_library("nope"));
    assert!(project.library("flat").unwrap().cell("c").is_some());
    // The other library is as it was.
    assert!(project.library("a").unwrap().cell("buf").is_none());
}
