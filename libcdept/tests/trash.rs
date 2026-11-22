//! Deleting cells and views to the trash. One test only: it points the user's trash
//! at a temporary folder through the environment.

use std::fs;
use std::path::Path;

use libcdept::trash::Removal;
use libcdept::{PROJECT_FILE, Project};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn cells_and_views_go_to_the_trash() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    // Safety: the only test of this program, so no other thread reads the environment.
    unsafe { std::env::set_var("XDG_DATA_HOME", &data) };

    let root = tmp.path().join("p");
    write(&root.join("libs/a/inv/schematic/inv.sch"), "v {}\n");
    write(&root.join("libs/a/inv/symbol/inv.sym"), "v {}\n");
    write(&root.join("libs/a/buf/schematic/buf.sch"), "v {}\n");
    write(&root.join("libs/flat/r.sym"), "v {}\n");
    write(&root.join("libs/flat/r.gds"), "");
    write(&root.join("libs/flat/r.gds.json"), "{}");
    write(&root.join("libs/ro/x.sym"), "v {}\n");
    write(
        &root.join(PROJECT_FILE),
        "name = \"t\"\n[libraries]\na = { path = \"libs/a\", layout = \"cells\" }\n\
         flat = \"libs/flat\"\nro = { path = \"libs/ro\", read_only = true }\n",
    );
    let mut project = Project::load(&root).unwrap();
    let files = data.join("Trash/files");

    // A view: its folder.
    let a = project.library("a").unwrap();
    let trashed = a.remove_view("inv", "symbol", Removal::Trash).unwrap();
    assert_eq!(trashed.len(), 1);
    assert!(!root.join("libs/a/inv/symbol").exists());
    assert!(files.join("symbol/inv.sym").is_file());
    assert_eq!(trashed[0].original, root.join("libs/a/inv/symbol"));

    // A cell: its folder with everything in it.
    let trashed_cell = a.remove_cell("inv", Removal::Trash).unwrap();
    assert!(!root.join("libs/a/inv").exists());
    assert!(files.join("inv/schematic/inv.sch").is_file());
    let info = fs::read_to_string(data.join("Trash/info/inv.trashinfo")).unwrap();
    assert!(info.contains("/libs/a/inv\n"), "{info}");

    // A cell of a flat library: its files, and the sidecar of the layout.
    let flat = project.library("flat").unwrap();
    assert_eq!(flat.cell_paths("r").unwrap().len(), 3);
    let trashed_flat = flat.remove_cell("r", Removal::Trash).unwrap();
    assert_eq!(trashed_flat.len(), 3);
    for name in ["r.sym", "r.gds", "r.gds.json"] {
        assert!(!root.join("libs/flat").join(name).exists(), "{name}");
        assert!(files.join(name).is_file(), "{name}");
    }

    // Read-only libraries and unknown cells are refused.
    let ro = project.library("ro").unwrap();
    assert!(ro.remove_cell("x", Removal::Trash).is_err());
    assert!(root.join("libs/ro/x.sym").is_file());
    assert!(a.remove_cell("nope", Removal::Trash).is_err());

    // For good, when asked.
    assert!(a.remove_cell("buf", Removal::Permanent).unwrap().is_empty());
    assert!(!root.join("libs/a/buf").exists());
    assert!(!files.join("buf").exists());

    // And back again.
    trashed_cell[0].restore().unwrap();
    project.rescan();
    let a = project.library("a").unwrap();
    assert!(a.cell("inv").unwrap().entry("schematic").is_some());
    assert!(a.cell("inv").unwrap().entry("symbol").is_none());
}
