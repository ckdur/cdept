//! The reference index (`libcdept::hierarchy`) and the operations that follow
//! references (`libcdept::refactor`), on copies of the example projects and on small
//! projects made here. No tool is started.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use libcdept::hierarchy::{Index, NodeKind, RefKind, Target, TreeKind, ViewId};
use libcdept::layout::{self, meta::Meta};
use libcdept::refactor::{self, ChangeKind, Operation, Options};
use libcdept::xschem::symbol;
use libcdept::{AddLibrary, LibraryLayout, Project};

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A copy of `examples/buffer`: libraries `logic` (inv, buf, rdiv, rpoly) and `bench`
/// (tb_buf). buf places inv twice in its schematic and once in its layout; rdiv places
/// the PCell view `logic/rpoly`.
fn buffer() -> (tempfile::TempDir, Project) {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/buffer");
    let dir = tempfile::tempdir().unwrap();
    // The folder is canonical, as the project root will be.
    let root = dir.path().canonicalize().unwrap().join("buffer");
    copy_tree(&example, &root);
    let project = Project::load(&root).unwrap();
    (dir, project)
}

fn run(project: &mut Project, operation: Operation) -> refactor::Outcome {
    let plan = refactor::plan(project, &operation, &Options::default()).unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    let outcome = refactor::apply(project, &plan).unwrap();
    project.reload().unwrap();
    outcome
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Nothing in the project's own (writable) views points nowhere.
fn assert_sound(project: &Project) {
    let index = Index::build(project);
    let dangling: Vec<String> = index
        .unresolved()
        .iter()
        .map(|r| format!("{} -> {} ({})", r.from, r.text, r.kind.name()))
        .collect();
    assert!(dangling.is_empty(), "{dangling:#?}");
    let links = symbol::check(project, None, None);
    assert!(links.is_empty(), "{links:#?}");
    for lib in &project.libraries {
        for cell in &lib.cells {
            if cell
                .entries
                .iter()
                .any(|e| e.main_file.as_deref().is_some_and(layout::is_readable))
            {
                let findings = layout::refs::check(project, &lib.name, &cell.name).unwrap();
                let errors: Vec<_> = findings.iter().filter(|f| f.is_error()).collect();
                assert!(errors.is_empty(), "{}/{}: {errors:#?}", lib.name, cell.name);
            }
        }
    }
}

#[test]
fn the_index_knows_who_uses_what() {
    let (_dir, project) = buffer();
    let index = Index::build(&project);
    assert!(index.warnings.is_empty(), "{:?}", index.warnings);

    // Schematics: tb_buf places buf, buf places inv twice.
    let tree = index
        .tree(&project, "bench", "tb_buf", TreeKind::Schematic)
        .unwrap();
    assert_eq!(
        tree.lines(false),
        ["tb_buf", "  buf (x1)", "    inv ×2 (x1, x2)"]
    );
    let all = tree.lines(true);
    assert!(
        all.iter().any(|l| l.trim_start().starts_with("nmos4.sym")),
        "{all:#?}"
    );
    let buf = &tree.children.iter().find(|c| c.name == "buf").unwrap();
    assert_eq!(buf.kind, NodeKind::Cell);
    assert_eq!(buf.view, Some(ViewId::new("logic", "buf", "schematic")));

    // Layouts: buf places inv; rdiv places the PCell view rpoly.
    let tree = index
        .tree(&project, "logic", "buf", TreeKind::Layout)
        .unwrap();
    assert_eq!(tree.lines(false), ["buf", "  inv ×2"]);
    let tree = index
        .tree(&project, "logic", "rdiv", TreeKind::Layout)
        .unwrap();
    assert_eq!(tree.lines(true), ["rdiv", "  rpoly ×2"]);
    assert!(
        index
            .tree(&project, "logic", "rpoly", TreeKind::Layout)
            .is_none()
    );

    // Who uses inv: the schematic and the layout of buf.
    let users: BTreeSet<(String, RefKind, usize)> = index
        .used_by("logic", "inv", None)
        .iter()
        .map(|r| (r.from.label(), r.kind, r.count))
        .collect();
    assert_eq!(
        users,
        BTreeSet::from([
            ("logic/buf/layout".to_string(), RefKind::Placement, 2),
            ("logic/buf/schematic".to_string(), RefKind::Instance, 2),
        ])
    );
    assert_eq!(index.used_by("logic", "inv", Some("layout")).len(), 1);
    let pcell = index.used_by("logic", "rpoly", None);
    assert_eq!(pcell.len(), 1);
    assert_eq!(pcell[0].kind, RefKind::Pcell);
    assert_eq!(pcell[0].text, "logic/rpoly");
    assert!(index.used_by("bench", "tb_buf", None).is_empty());

    // xschem's own devices are files outside the project; nothing is unresolved.
    assert!(
        index
            .references
            .iter()
            .any(|r| r.text == "ipin.sym" && matches!(r.target, Target::External(_)))
    );
    assert_sound(&project);
}

#[test]
fn renaming_a_cell_takes_its_users_along() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    let operation = Operation::RenameCell {
        library: "logic".into(),
        cell: "inv".into(),
        new: "inv_x1".into(),
    };
    // The dry run says what will happen, and changes nothing.
    let plan = refactor::plan(&project, &operation, &Options::default()).unwrap();
    let lines = plan.lines(&root).join("\n");
    assert!(
        lines.contains("move libs/logic/inv -> libs/logic/inv_x1"),
        "{lines}"
    );
    assert!(
        lines.contains(
            "2 instance(s) of logic/inv/symbol/inv.sym -> logic/inv_x1/symbol/inv_x1.sym"
        ),
        "{lines}"
    );
    assert!(lines.contains("structure inv -> inv_x1"), "{lines}");
    assert!(root.join("libs/logic/inv/schematic/inv.sch").is_file());
    assert!(plan.not_followed().is_empty());

    let outcome = refactor::apply(&project, &plan).unwrap();
    assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
    project.reload().unwrap();

    let cell = root.join("libs/logic/inv_x1");
    assert!(!root.join("libs/logic/inv").exists());
    for file in [
        "schematic/inv_x1.sch",
        "symbol/inv_x1.sym",
        "layout/inv_x1.gds",
        "layout/inv_x1.gds.json",
    ] {
        assert!(cell.join(file).is_file(), "{file}");
    }
    // Its own layout: the sidecar and the top structure are named after it.
    let own = Meta::read_for(&cell.join("layout/inv_x1.gds"))
        .unwrap()
        .unwrap();
    assert_eq!((own.cell.as_str(), own.top.as_str()), ("inv_x1", "inv_x1"));
    assert!(
        layout::read_summary(&cell.join("layout/inv_x1.gds"))
            .unwrap()
            .defines("inv_x1")
    );
    // The schematic of buf places the renamed symbol; nothing else changed in it.
    let before = read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/buffer/libs/logic/buf/schematic/buf.sch"),
    );
    let after = read(root.join("libs/logic/buf/schematic/buf.sch"));
    assert_eq!(
        after,
        before.replace("logic/inv/symbol/inv.sym", "logic/inv_x1/symbol/inv_x1.sym")
    );
    // The layout of buf places it under its new name.
    let buf = root.join("libs/logic/buf/layout/buf.gds");
    let meta = Meta::read_for(&buf).unwrap().unwrap();
    assert_eq!(meta.references["inv_x1"].cell, "inv_x1");
    assert!(meta.references["inv_x1"].bbox.is_some());
    assert_eq!(layout::read_summary(&buf).unwrap().ghosts(), ["inv_x1"]);
    assert_sound(&project);
    let index = Index::build(&project);
    assert_eq!(index.used_by("logic", "inv_x1", None).len(), 2);
}

#[test]
fn a_pcell_view_keeps_its_users_when_renamed_or_moved() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    run(
        &mut project,
        Operation::RenameCell {
            library: "logic".into(),
            cell: "rpoly".into(),
            new: "rpoly_hv".into(),
        },
    );
    assert!(
        root.join("libs/logic/rpoly_hv/pcell/rpoly_hv.pcell.py")
            .is_file()
    );
    let index = Index::build(&project);
    let users = index.used_by("logic", "rpoly_hv", None);
    assert_eq!(users.len(), 1, "{users:#?}");
    assert_eq!(users[0].text, "logic/rpoly_hv");
    assert_eq!(users[0].count, 2);

    // Into another library: the layout names that library.
    run(
        &mut project,
        Operation::MoveCell {
            library: "logic".into(),
            cell: "rpoly_hv".into(),
            to_library: "bench".into(),
            new: "rpoly".into(),
        },
    );
    let index = Index::build(&project);
    let users = index.used_by("bench", "rpoly", None);
    assert_eq!(users.len(), 1, "{users:#?}");
    assert_eq!(users[0].text, "bench/rpoly");
    assert_eq!(users[0].from, ViewId::new("logic", "rdiv", "layout"));
    assert_sound(&project);
}

#[test]
fn moving_a_cell_to_another_library() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    run(
        &mut project,
        Operation::MoveCell {
            library: "logic".into(),
            cell: "inv".into(),
            to_library: "bench".into(),
            new: "inv".into(),
        },
    );
    assert!(root.join("libs/bench/inv/symbol/inv.sym").is_file());
    assert!(!root.join("libs/logic/inv").exists());
    let sch = read(root.join("libs/logic/buf/schematic/buf.sch"));
    assert_eq!(
        sch.matches("C {bench/inv/symbol/inv.sym}").count(),
        2,
        "{sch}"
    );
    // The symbol finds its schematic in the new library folder.
    let sym = read(root.join("libs/bench/inv/symbol/inv.sym"));
    assert_eq!(
        symbol::schematic(&sym).as_deref(),
        Some("bench/@symname/schematic/@symname.sch")
    );
    let meta = Meta::read_for(&root.join("libs/logic/buf/layout/buf.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(meta.references["inv"].library, "bench");
    let own = Meta::read_for(&root.join("libs/bench/inv/layout/inv.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(own.library, "bench");
    assert_sound(&project);
}

#[test]
fn a_copy_points_at_itself_and_leaves_the_original_alone() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    let original = read(root.join("libs/logic/buf/schematic/buf.sch"));
    run(
        &mut project,
        Operation::CopyCell {
            library: "logic".into(),
            cell: "buf".into(),
            to_library: "bench".into(),
            new: "buf2".into(),
            hierarchy: false,
        },
    );
    let copy = root.join("libs/bench/buf2");
    assert_eq!(
        read(root.join("libs/logic/buf/schematic/buf.sch")),
        original
    );
    // The copy still places the original inv.
    assert_eq!(read(copy.join("schematic/buf2.sch")), original);
    let sym = read(copy.join("symbol/buf2.sym"));
    assert_eq!(
        symbol::schematic(&sym).as_deref(),
        Some("bench/@symname/schematic/@symname.sch")
    );
    let meta = Meta::read_for(&copy.join("layout/buf2.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (meta.library.as_str(), meta.cell.as_str(), meta.top.as_str()),
        ("bench", "buf2", "buf2")
    );
    assert_eq!(meta.references["inv"].library, "logic");
    let original_meta = Meta::read_for(&root.join("libs/logic/buf/layout/buf.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(original_meta.cell, "buf");
    assert_sound(&project);

    // With the cells below: the copy places the copied inv.
    run(
        &mut project,
        Operation::CopyCell {
            library: "logic".into(),
            cell: "buf".into(),
            to_library: "bench".into(),
            new: "buf".into(),
            hierarchy: true,
        },
    );
    assert!(root.join("libs/bench/inv/schematic/inv.sch").is_file());
    let sch = read(root.join("libs/bench/buf/schematic/buf.sch"));
    assert_eq!(
        sch.matches("C {bench/inv/symbol/inv.sym}").count(),
        2,
        "{sch}"
    );
    let meta = Meta::read_for(&root.join("libs/bench/buf/layout/buf.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(meta.references["inv"].library, "bench");
    assert_eq!(
        read(root.join("libs/logic/buf/schematic/buf.sch")),
        original
    );
    assert_sound(&project);
    // In the same library the cells below would keep their names.
    let same = Operation::CopyCell {
        library: "logic".into(),
        cell: "buf".into(),
        to_library: "logic".into(),
        new: "buf3".into(),
        hierarchy: true,
    };
    assert!(refactor::plan(&project, &same, &Options::default()).is_err());
}

#[test]
fn views_can_be_renamed_and_copied() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    run(
        &mut project,
        Operation::RenameView {
            library: "logic".into(),
            cell: "inv".into(),
            view: "schematic".into(),
            new: "sch_rc".into(),
        },
    );
    let sym = read(root.join("libs/logic/inv/symbol/inv.sym"));
    assert_eq!(
        symbol::schematic(&sym).as_deref(),
        Some("logic/@symname/sch_rc/@symname.sch")
    );
    run(
        &mut project,
        Operation::RenameView {
            library: "logic".into(),
            cell: "inv".into(),
            view: "layout".into(),
            new: "layout_v2".into(),
        },
    );
    let own = Meta::read_for(&root.join("libs/logic/inv/layout_v2/inv.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(own.view, "layout_v2");
    let meta = Meta::read_for(&root.join("libs/logic/buf/layout/buf.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(meta.references["inv"].view, "layout_v2");
    assert_sound(&project);

    // A view copied into another cell is named after that cell.
    run(
        &mut project,
        Operation::CopyView {
            library: "logic".into(),
            cell: "inv".into(),
            view: "layout_v2".into(),
            to_library: "bench".into(),
            to_cell: "tb_buf".into(),
            new: "layout".into(),
        },
    );
    let copy = root.join("libs/bench/tb_buf/layout/tb_buf.gds");
    let meta = Meta::read_for(&copy).unwrap().unwrap();
    assert_eq!(
        (
            meta.library.as_str(),
            meta.cell.as_str(),
            meta.view.as_str(),
            meta.top.as_str()
        ),
        ("bench", "tb_buf", "layout", "tb_buf")
    );
    assert!(layout::read_summary(&copy).unwrap().defines("tb_buf"));
    assert!(root.join("libs/logic/inv/layout_v2/inv.gds").is_file());
    assert_sound(&project);

    // A taken name, and a view that is a single file, are refused.
    let taken = Operation::RenameView {
        library: "logic".into(),
        cell: "inv".into(),
        view: "symbol".into(),
        new: "sch_rc".into(),
    };
    assert!(refactor::plan(&project, &taken, &Options::default()).is_err());
}

#[test]
fn renaming_a_library_with_and_without_its_folder() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    // The name only: xschem keeps naming the folder; layouts name the library.
    let outcome = run(
        &mut project,
        Operation::RenameLibrary {
            library: "logic".into(),
            new: "core".into(),
            folder: false,
        },
    );
    assert!(outcome.problems.is_empty());
    assert!(project.library("core").is_some() && project.library("logic").is_none());
    let toml = read(root.join("cdept.toml"));
    assert!(
        toml.contains("core = \"libs/logic\"\nbench = \"libs/bench\""),
        "{toml}"
    );
    let sch = read(root.join("libs/bench/tb_buf/schematic/tb_buf.sch"));
    assert!(sch.contains("C {logic/buf/symbol/buf.sym}"));
    let meta = Meta::read_for(&root.join("libs/logic/buf/layout/buf.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(meta.library, "core");
    assert_eq!(meta.references["inv"].library, "core");
    let index = Index::build(&project);
    assert_eq!(index.used_by("core", "rpoly", None)[0].text, "core/rpoly");
    assert_sound(&project);

    // With the folder: the schematics and the symbols follow.
    run(
        &mut project,
        Operation::RenameLibrary {
            library: "core".into(),
            new: "cells".into(),
            folder: true,
        },
    );
    assert!(root.join("libs/cells/inv/symbol/inv.sym").is_file());
    assert!(!root.join("libs/logic").exists());
    let toml = read(root.join("cdept.toml"));
    assert!(toml.contains("cells = \"libs/cells\""), "{toml}");
    let sch = read(root.join("libs/bench/tb_buf/schematic/tb_buf.sch"));
    assert!(sch.contains("C {cells/buf/symbol/buf.sym}"), "{sch}");
    let sch = read(root.join("libs/cells/buf/schematic/buf.sch"));
    assert_eq!(sch.matches("C {cells/inv/symbol/inv.sym}").count(), 2);
    let sym = read(root.join("libs/cells/inv/symbol/inv.sym"));
    assert_eq!(
        symbol::schematic(&sym).as_deref(),
        Some("cells/@symname/schematic/@symname.sch")
    );
    assert_sound(&project);
}

#[test]
fn a_failed_step_puts_everything_back() {
    let (_dir, project) = buffer();
    let root = project.root.clone();
    let operation = Operation::RenameCell {
        library: "logic".into(),
        cell: "inv".into(),
        new: "inv2".into(),
    };
    let plan = refactor::plan(&project, &operation, &Options::default()).unwrap();
    // Behind the plan's back: a sidecar that will be rewritten becomes a folder.
    let sidecar = root.join("libs/logic/buf/layout/buf.gds.json");
    let saved = read(sidecar.clone());
    fs::remove_file(&sidecar).unwrap();
    fs::create_dir(&sidecar).unwrap();
    let before = read(root.join("libs/logic/buf/schematic/buf.sch"));

    let error = refactor::apply(&project, &plan).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("everything was put back"), "{text}");
    assert!(root.join("libs/logic/inv/schematic/inv.sch").is_file());
    assert!(root.join("libs/logic/inv/layout/inv.gds.json").is_file());
    assert!(!root.join("libs/logic/inv2").exists());
    assert_eq!(read(root.join("libs/logic/buf/schematic/buf.sch")), before);
    let own = Meta::read_for(&root.join("libs/logic/inv/layout/inv.gds"))
        .unwrap()
        .unwrap();
    assert_eq!(own.cell, "inv");
    assert!(
        layout::read_summary(&root.join("libs/logic/inv/layout/inv.gds"))
            .unwrap()
            .defines("inv")
    );

    // With the sidecar back, the same plan applies.
    fs::remove_dir(&sidecar).unwrap();
    fs::write(&sidecar, saved).unwrap();
    refactor::apply(&project, &plan).unwrap();
    assert!(root.join("libs/logic/inv2/schematic/inv2.sch").is_file());
}

#[test]
fn unsaved_files_block_and_read_only_users_are_reported() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    let operation = Operation::RenameCell {
        library: "logic".into(),
        cell: "inv".into(),
        new: "inv2".into(),
    };
    // buf's schematic is open with unsaved changes: it must not be rewritten.
    let busy = Options {
        busy: BTreeSet::from([root.join("libs/logic/buf/schematic/buf.sch")]),
        ..Options::default()
    };
    let plan = refactor::plan(&project, &operation, &busy).unwrap();
    assert_eq!(plan.blockers.len(), 1, "{:?}", plan.blockers);
    assert!(refactor::apply(&project, &plan).is_err());
    assert!(root.join("libs/logic/inv").is_dir());
    // So must not a file of the cell itself.
    let busy = Options {
        busy: BTreeSet::from([root.join("libs/logic/inv/schematic/inv.sch")]),
        ..Options::default()
    };
    let plan = refactor::plan(&project, &operation, &busy).unwrap();
    assert_eq!(plan.blockers.len(), 1, "{:?}", plan.blockers);

    // A read-only library that uses the cell: reported, not touched.
    let toml = read(root.join("cdept.toml")).replace(
        "bench = \"libs/bench\"",
        "bench = { path = \"libs/bench\", read_only = true }",
    );
    fs::write(root.join("cdept.toml"), toml).unwrap();
    project.reload().unwrap();
    let operation = Operation::RenameCell {
        library: "logic".into(),
        cell: "buf".into(),
        new: "buffer".into(),
    };
    let plan = refactor::plan(&project, &operation, &Options::default()).unwrap();
    let skipped = plan.not_followed();
    assert_eq!(skipped.len(), 1, "{:#?}", plan.changes);
    assert_eq!(skipped[0].label, "bench/tb_buf/schematic");
    assert!(skipped[0].details[0].contains("read-only"));
    let tb = read(root.join("libs/bench/tb_buf/schematic/tb_buf.sch"));
    let outcome = refactor::apply(&project, &plan).unwrap();
    assert_eq!(outcome.problems.len(), 1);
    assert_eq!(
        read(root.join("libs/bench/tb_buf/schematic/tb_buf.sch")),
        tb
    );
    // References can also be left alone on purpose.
    project.reload().unwrap();
    let keep = Options {
        keep_references: true,
        ..Options::default()
    };
    let operation = Operation::RenameCell {
        library: "logic".into(),
        cell: "inv".into(),
        new: "inv2".into(),
    };
    let plan = refactor::plan(&project, &operation, &keep).unwrap();
    assert!(plan.changes.iter().all(|c| c.kind == ChangeKind::Move));
}

/// A project with a `cells` library `a` and a flat library `f`.
fn two_layouts() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut project = Project::create(&root, Some("demo")).unwrap();
    project
        .add_library(
            "a",
            "libs/a",
            AddLibrary {
                create: true,
                ..Default::default()
            },
        )
        .unwrap();
    fs::create_dir_all(root.join("libs/f")).unwrap();
    project
        .add_library(
            "f",
            "libs/f",
            AddLibrary {
                layout: Some(LibraryLayout::Flat),
                ..Default::default()
            },
        )
        .unwrap();
    (dir, project)
}

const SYMBOL: &str = "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {type=subcircuit\n\
                      format=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"\n}\nV {}\nS {}\nE {}\n";
const SCHEMATIC: &str =
    "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {}\nV {}\nS {}\nE {}\n";

#[test]
fn flat_and_cells_libraries_keep_their_schematic_links() {
    let (_dir, mut project) = two_layouts();
    let root = project.root.clone();
    // A flat cell: the schematic is next to the symbol, no attribute. A user of it.
    fs::write(root.join("libs/f/amp.sym"), SYMBOL).unwrap();
    fs::write(root.join("libs/f/amp.sch"), SCHEMATIC).unwrap();
    fs::write(
        root.join("libs/f/top.sch"),
        format!(
            "{SCHEMATIC}C {{f/amp.sym}} 0 0 0 0 {{name=x1}}\nC {{f/amp.sym}} 9 0 0 0 {{name=x2}}\n"
        ),
    )
    .unwrap();
    project.reload().unwrap();
    let index = Index::build(&project);
    assert_eq!(
        index
            .tree(&project, "f", "top", TreeKind::Schematic)
            .unwrap()
            .lines(false),
        ["top", "  amp ×2 (x1, x2)"]
    );

    // Renamed in the flat library: the user follows.
    run(
        &mut project,
        Operation::RenameCell {
            library: "f".into(),
            cell: "amp".into(),
            new: "ota".into(),
        },
    );
    assert!(root.join("libs/f/ota.sym").is_file() && root.join("libs/f/ota.sch").is_file());
    assert!(read(root.join("libs/f/top.sch")).contains("C {f/ota.sym} 9 0 0 0 {name=x2}"));
    assert_eq!(symbol::schematic(&read(root.join("libs/f/ota.sym"))), None);

    // Moved into view folders: the symbol has to say where its schematic is.
    run(
        &mut project,
        Operation::MoveCell {
            library: "f".into(),
            cell: "ota".into(),
            to_library: "a".into(),
            new: "ota".into(),
        },
    );
    assert!(!root.join("libs/f/ota.sym").exists());
    let sym = read(root.join("libs/a/ota/symbol/ota.sym"));
    assert_eq!(
        symbol::schematic(&sym).as_deref(),
        Some("a/@symname/schematic/@symname.sch")
    );
    assert!(read(root.join("libs/f/top.sch")).contains("C {a/ota/symbol/ota.sym} 0 0 0 0"));
    assert_sound(&project);

    // Copied back into the flat library: next to each other again, no attribute.
    run(
        &mut project,
        Operation::CopyCell {
            library: "a".into(),
            cell: "ota".into(),
            to_library: "f".into(),
            new: "ota2".into(),
            hierarchy: false,
        },
    );
    assert_eq!(symbol::schematic(&read(root.join("libs/f/ota2.sym"))), None);
    assert!(root.join("libs/f/ota2.sch").is_file());
    assert_sound(&project);
}

#[test]
fn subcircuit_and_module_names_follow_a_rename() {
    let (_dir, mut project) = two_layouts();
    let root = project.root.clone();
    let write = |path: &str, text: &str| {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    };
    write(
        "libs/a/inv/netlist/inv.spice",
        "* inverter\n.subckt inv A Y VDD VSS\nXM1 Y A VSS VSS nfet w=1u\n.ends inv\n",
    );
    write(
        "libs/a/inv/hdl/inv.v",
        "module inv (input a, output y);\n  assign y = ~a;\nendmodule\n",
    );
    write(
        "libs/a/buf/netlist/buf.spice",
        ".subckt buf A Y VDD VSS\nx1 A n VDD VSS inv\nx2 n Y VDD VSS\n+ INV m=2\n.ends\n",
    );
    write(
        "libs/a/buf/hdl/buf.v",
        "module buf (input a, output y);\n  wire n; // inv u0 (a, n);\n  inv u1 (.a(a), .y(n));\n  \
         inv #(.D(1)) u2 (.a(n), .y(y));\nendmodule\n",
    );
    project.reload().unwrap();
    let index = Index::build(&project);
    assert_eq!(
        index
            .tree(&project, "a", "buf", TreeKind::Netlist)
            .unwrap()
            .lines(true),
        ["buf", "  inv ×2 (x1, x2)", "    nfet (XM1)"],
    );
    assert_eq!(
        index
            .tree(&project, "a", "buf", TreeKind::Hdl)
            .unwrap()
            .lines(false),
        ["buf", "  inv ×2 (u1, u2)"]
    );
    assert_eq!(index.used_by("a", "inv", None).len(), 2);

    run(
        &mut project,
        Operation::RenameCell {
            library: "a".into(),
            cell: "inv".into(),
            new: "inv_x2".into(),
        },
    );
    assert_eq!(
        read(root.join("libs/a/inv_x2/netlist/inv_x2.spice")),
        "* inverter\n.subckt inv_x2 A Y VDD VSS\nXM1 Y A VSS VSS nfet w=1u\n.ends inv_x2\n"
    );
    assert!(read(root.join("libs/a/inv_x2/hdl/inv_x2.v")).starts_with("module inv_x2 (input a"));
    assert_eq!(
        read(root.join("libs/a/buf/netlist/buf.spice")),
        ".subckt buf A Y VDD VSS\nx1 A n VDD VSS inv_x2\nx2 n Y VDD VSS\n+ inv_x2 m=2\n.ends\n"
    );
    assert_eq!(
        read(root.join("libs/a/buf/hdl/buf.v")),
        "module buf (input a, output y);\n  wire n; // inv u0 (a, n);\n  inv_x2 u1 (.a(a), .y(n));\n  \
         inv_x2 #(.D(1)) u2 (.a(n), .y(y));\nendmodule\n"
    );
    let index = Index::build(&project);
    assert_eq!(index.used_by("a", "inv_x2", None).len(), 2);
}

#[test]
fn libraries_whose_folders_share_a_name_are_reported() {
    let (_dir, mut project) = two_layouts();
    let options = AddLibrary {
        create: true,
        ..Default::default()
    };
    project
        .add_library("other", "elsewhere/a", options)
        .unwrap();
    let warnings = libcdept::hierarchy::shadowed_libraries(&project);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("'a'") && warnings[0].contains("'other'"));
    assert_eq!(Index::build(&project).warnings, warnings);
}

#[test]
fn deleting_says_who_is_left_without_it() {
    let (_dir, mut project) = buffer();
    let root = project.root.clone();
    let operation = Operation::DeleteCell {
        library: "logic".into(),
        cell: "inv".into(),
    };
    let plan = refactor::plan(&project, &operation, &Options::default()).unwrap();
    let left: BTreeSet<&str> = plan
        .not_followed()
        .iter()
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(
        left,
        BTreeSet::from(["logic/buf/layout", "logic/buf/schematic"])
    );
    assert!(root.join("libs/logic/inv").is_dir());
    let outcome = refactor::apply(&project, &plan).unwrap();
    assert_eq!(outcome.problems.len(), 2);
    project.reload().unwrap();
    assert!(project.library("logic").unwrap().cell("inv").is_none());
    // The index now says so.
    let index = Index::build(&project);
    assert_eq!(index.unresolved().len(), 2);
    let tree = index
        .tree(&project, "logic", "buf", TreeKind::Schematic)
        .unwrap();
    assert_eq!(
        tree.children
            .iter()
            .filter(|c| c.kind == NodeKind::Missing)
            .count(),
        1
    );
}
