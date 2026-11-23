//! Organising a project: categories, search, importing an existing design folder,
//! LibMan project files. The tests that need a PDK or the xschem helper are skipped
//! when those are absent.

use std::fs;
use std::path::Path;

use libcdept::category::{self, Store};
use libcdept::import::{self, Options, Origin};
use libcdept::search::{self, ContentKind, Matched, Query};
use libcdept::{AddLibrary, LibraryLayout, Project, ViewKind};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

const SCH: &str = "v {xschem version=3.4.5 file_version=1.2}\nG {}\nK {}\nV {}\nS {}\nE {}\n\
C {devices/ipin.sym} 0 0 0 0 {name=p1 lab=VINP}\n\
C {analog/ota5t.sym} 100 0 0 0 {name=x1}\n";
const SYM: &str = "v {xschem version=3.4.5 file_version=1.2}\nG {}\n\
K {type=subcircuit\nformat=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"}\nV {}\nS {}\nE {}\n\
B 5 -2 -2 2 2 {name=VINP dir=in}\n";

fn no_env(_: &str) -> Option<String> {
    None
}

fn options() -> Options {
    Options {
        helpers: false,
        env: no_env,
        ..Options::default()
    }
}

/// A project with a `cells` library (ota5t, bandgap, inv), a writable flat library
/// (nand2, nor2) and a read-only flat library (nmos, pmos, res).
fn project(dir: &Path) -> Project {
    let mut project = Project::create(dir, Some("demo")).unwrap();
    project
        .add_library(
            "analog",
            "libs/analog",
            AddLibrary {
                create: true,
                ..AddLibrary::default()
            },
        )
        .unwrap();
    for cell in ["ota5t", "bandgap", "inv"] {
        let lib = dir.join("libs/analog");
        write(
            &lib.join(cell).join("schematic").join(format!("{cell}.sch")),
            SCH,
        );
        write(
            &lib.join(cell).join("symbol").join(format!("{cell}.sym")),
            SYM,
        );
    }
    write(&dir.join("libs/analog/ota5t/layout/ota5t.gds"), "");
    for cell in ["nand2", "nor2"] {
        write(&dir.join(format!("libs/gates/{cell}.sch")), SCH);
        write(&dir.join(format!("libs/gates/{cell}.sym")), SYM);
    }
    for (cell, kind) in [("nmos", "nmos"), ("pmos", "pmos"), ("res", "resistor")] {
        write(
            &dir.join(format!("pdk/prims/{cell}.sym")),
            &SYM.replace("type=subcircuit", &format!("type={kind}")),
        );
    }
    project
        .add_library("gates", "libs/gates", AddLibrary::default())
        .unwrap();
    project
        .add_library(
            "prims",
            "pdk/prims",
            AddLibrary {
                read_only: true,
                ..AddLibrary::default()
            },
        )
        .unwrap();
    project
}

#[test]
fn categories_are_kept_in_the_library_or_in_the_project() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let analog = project.library("analog").unwrap();
    assert_eq!(analog.layout, LibraryLayout::Cells);
    assert_eq!(category::store_of(analog), Store::Library);
    assert_eq!(
        category::store_of(project.library("gates").unwrap()),
        Store::Project
    );
    assert_eq!(
        category::store_of(project.library("prims").unwrap()),
        Store::Project
    );

    // A library with view folders keeps them in its lib.toml.
    category::assign(&project, "analog", "analog/amplifiers", &["ota5t"]).unwrap();
    category::assign(&project, "analog", "favourites", &["ota5t", "inv"]).unwrap();
    category::add(&project, "analog", "empty").unwrap();
    let text = fs::read_to_string(dir.path().join("libs/analog/lib.toml")).unwrap();
    assert!(text.contains("[categories]"), "{text}");
    assert!(
        text.contains("\"analog/amplifiers\" = [\"ota5t\"]"),
        "{text}"
    );
    assert!(
        text.contains("description"),
        "the rest of the file is kept: {text}"
    );
    assert!(!dir.path().join(category::PROJECT_CATEGORIES_FILE).exists());

    let found = category::load(&project, analog);
    assert_eq!(
        found.all.names(),
        ["analog", "analog/amplifiers", "empty", "favourites"]
    );
    assert_eq!(
        found.all.of_cell("ota5t"),
        ["analog/amplifiers", "favourites"]
    );
    assert!(found.all.contains("analog", "ota5t"));
    assert!(found.all.is_uncategorised("bandgap"));

    // Errors: unknown cell, a category that exists, one that does not.
    assert!(category::assign(&project, "analog", "x", &["nothing"]).is_err());
    assert!(category::add(&project, "analog", "analog").is_err());
    assert!(category::remove(&project, "analog", "nothing").is_err());

    category::rename(&project, "analog", "analog", "ana").unwrap();
    category::unassign(&project, "analog", "favourites", &["inv"]).unwrap();
    category::cell_renamed(&project, "analog", "ota5t", "ota").unwrap();
    category::remove(&project, "analog", "empty").unwrap();
    let text = fs::read_to_string(dir.path().join("libs/analog/lib.toml")).unwrap();
    assert!(text.contains("\"ana/amplifiers\" = [\"ota\"]"), "{text}");
    assert!(text.contains("favourites = [\"ota\"]"), "{text}");
    assert!(!text.contains("empty"), "{text}");

    // Read-only and flat libraries: in the project's categories.toml.
    category::assign(&project, "prims", "mos", &["nmos", "pmos"]).unwrap();
    category::assign(&project, "gates", "logic", &["nand2"]).unwrap();
    let text = fs::read_to_string(dir.path().join(category::PROJECT_CATEGORIES_FILE)).unwrap();
    assert!(
        text.contains("[prims]") && text.contains("[gates]"),
        "{text}"
    );
    assert!(text.contains("mos = [\"nmos\", \"pmos\"]"), "{text}");
    assert!(!dir.path().join("pdk/prims/lib.toml").exists());
    assert!(!dir.path().join("libs/gates/lib.toml").exists());
    // The flat library is still flat.
    let project = Project::load(dir.path()).unwrap();
    assert_eq!(
        project.library("gates").unwrap().layout,
        LibraryLayout::Flat
    );
    let prims = project.library("prims").unwrap();
    assert_eq!(
        category::load(&project, prims).all.cells("mos"),
        ["nmos", "pmos"]
    );
    category::remove(&project, "prims", "mos").unwrap();
    category::remove(&project, "gates", "logic").unwrap();
    let text = fs::read_to_string(dir.path().join(category::PROJECT_CATEGORIES_FILE)).unwrap();
    assert!(!text.contains("[prims]"), "{text}");
}

#[test]
fn categories_are_suggested_and_libman_groups_are_read() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let prims = project.library("prims").unwrap();
    let suggested = category::suggest(prims);
    assert_eq!(suggested.names(), ["nmos", "pmos", "resistor"]);
    assert_eq!(suggested.cells("resistor"), ["res"]);
    assert_eq!(category::add_all(&project, "prims", &suggested).unwrap(), 3);
    assert_eq!(category::add_all(&project, "prims", &suggested).unwrap(), 0);

    // LibMan keeps a category as <library>/<name>.group.
    write(
        &dir.path().join("libs/analog/Amplifiers.group"),
        "ota5t nothing\nbandgap\n",
    );
    let analog = project.library("analog").unwrap();
    let found = category::load(&project, analog);
    // Cells that do not exist are not shown.
    assert_eq!(found.all.cells("Amplifiers"), ["bandgap", "ota5t"]);
    assert_eq!(found.libman.cells("Amplifiers").len(), 3);
    // LibMan's own cannot be changed until adopted.
    assert!(category::remove(&project, "analog", "Amplifiers").is_err());
    assert!(category::unassign(&project, "analog", "Amplifiers", &["ota5t"]).is_err());
    assert_eq!(category::adopt_libman(&project, "analog").unwrap(), 3);
    category::unassign(&project, "analog", "Amplifiers", &["bandgap"]).unwrap();
    let text = fs::read_to_string(dir.path().join("libs/analog/lib.toml")).unwrap();
    assert!(
        text.contains("Amplifiers = [\"nothing\", \"ota5t\"]"),
        "{text}"
    );
}

#[test]
fn search_finds_cells_views_and_content() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    category::assign(&project, "analog", "analog/amplifiers", &["ota5t"]).unwrap();
    let find = |text: &str| search::search(&project, &Query::parse(text));
    let names = |text: &str| -> Vec<String> {
        find(text)
            .hits
            .iter()
            .map(|h| match &h.view {
                Some(view) => format!("{}/{}/{}", h.library, h.cell, view),
                None => format!("{}/{}", h.library, h.cell),
            })
            .collect()
    };

    assert_eq!(names("ota"), ["analog/ota5t"]);
    assert_eq!(names("o5"), ["analog/ota5t"]);
    assert_eq!(names("n*2"), ["gates/nand2", "gates/nor2"]);
    assert_eq!(names("nand nor"), Vec::<String>::new());
    // The best match first: the name itself, then a longer one.
    assert_eq!(names("n")[..2], ["prims/nmos", "gates/nor2"]);
    assert_eq!(names("lib:gates"), ["gates/nand2", "gates/nor2"]);
    assert_eq!(names("pri/mos"), ["prims/nmos", "prims/pmos"]);
    assert_eq!(names("kind:layout"), ["analog/ota5t/layout"]);
    assert_eq!(names("format:sch lib:an* inv"), ["analog/inv/schematic"]);
    assert_eq!(names("cat:amplifiers"), ["analog/ota5t"]);
    assert_eq!(names("cat:analog"), ["analog/ota5t"]);
    assert_eq!(names("cat:nothing"), Vec::<String>::new());
    assert_eq!(find("").total, 8);
    let mut limited = Query::parse("");
    limited.limit = 3;
    let results = search::search(&project, &limited);
    assert_eq!((results.hits.len(), results.total), (3, 8));

    // Inside the files: instances and pins.
    let results = find("in:content ota5t.sym lib:gates");
    assert_eq!(results.hits.len(), 2);
    assert!(results.hits.iter().all(|h| matches!(
        &h.matched,
        Matched::Content {
            kind: ContentKind::Instance,
            line: 8,
            ..
        }
    )));
    assert!(results.files_scanned >= 4);
    let results = find("in:content vinp kind:symbol lib:prims");
    assert_eq!(results.hits.len(), 3);
    assert!(results.hits.iter().all(|h| {
        h.kind == Some(ViewKind::Symbol)
            && matches!(
                &h.matched,
                Matched::Content {
                    kind: ContentKind::Pin,
                    ..
                }
            )
    }));
}

/// A design folder without a project: flat symbol folders, a library with cell
/// folders, a GDS folder, simulation output.
fn design_folder(dir: &Path) {
    for cell in ["ota", "bias"] {
        write(&dir.join(format!("xschem/analog/{cell}.sch")), SCH);
        write(&dir.join(format!("xschem/analog/{cell}.sym")), SYM);
    }
    write(&dir.join("xschem/tb/tb_ota.sch"), SCH);
    write(&dir.join("gds/ota.gds"), "");
    write(&dir.join("gds/top.gds"), "");
    write(&dir.join("ip/adc/schematic/adc.sch"), SCH);
    write(&dir.join("ip/adc/symbol/adc.sym"), SYM);
    write(&dir.join("ip/dac/schematic/dac.sch"), SCH);
    write(&dir.join("simulations/tb_ota.spice"), "* netlist\n");
    write(&dir.join("simulations/tb_ota.raw"), "");
    write(&dir.join("xschem/tb/run1/out.raw"), "");
    write(&dir.join("xschem/tb/run1/out.spice"), "");
    write(&dir.join("doc/readme.md"), "");
    write(&dir.join(".git/config"), "");
    write(
        &dir.join("tech/my.lyt"),
        "<technology>\n <name>mytech</name>\n</technology>\n",
    );
}

#[test]
fn a_design_folder_becomes_a_project() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("chip");
    design_folder(&root);
    let mut plan = import::scan(&root, &options()).unwrap();
    assert_eq!(plan.name, "chip");
    let summary: Vec<(String, &str, usize)> = plan
        .libraries
        .iter()
        .map(|l| (plan.path_text(&l.folder), l.layout.name(), l.cells))
        .collect();
    assert_eq!(
        summary,
        [
            ("gds".to_string(), "flat", 2),
            ("ip".to_string(), "cells", 2),
            ("xschem/analog".to_string(), "flat", 2),
            ("xschem/tb".to_string(), "flat", 1),
        ]
    );
    assert!(plan.libraries.iter().all(|l| l.enabled && !l.read_only));
    assert!(plan.libraries.iter().all(|l| l.origin == Origin::Folder));
    assert_eq!(plan.libraries[1].views[&ViewKind::Schematic], 2);
    let skipped: Vec<String> = plan
        .skipped
        .iter()
        .map(|s| plan.path_text(&s.folder))
        .collect();
    assert_eq!(skipped, ["simulations", "xschem/tb/run1"]);
    assert_eq!(plan.technology, None);
    assert_eq!(plan.klayout_tech, Some(root.join("tech/my.lyt")));
    let text = plan.describe();
    assert!(
        text.contains("[x] analog") && text.contains("Left out:"),
        "{text}"
    );

    // The user leaves one out and renames another.
    plan.libraries[3].enabled = false;
    plan.libraries[0].name = "layouts".into();
    let toml = plan.toml();
    assert!(
        toml.contains("layouts = { path = \"gds\", layout = \"flat\" }"),
        "{toml}"
    );
    assert!(!toml.contains("xschem/tb"), "{toml}");
    let project = plan.write().unwrap();
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    let libraries: Vec<(&str, usize)> = project
        .libraries
        .iter()
        .map(|l| (l.name.as_str(), l.cells.len()))
        .collect();
    assert_eq!(libraries, [("layouts", 2), ("ip", 2), ("analog", 2)]);
    assert_eq!(project.library("ip").unwrap().layout, LibraryLayout::Cells);
    assert_eq!(
        project.klayout.tech.as_deref(),
        root.join("tech/my.lyt").to_str()
    );
    // A second import does not overwrite the project; it can add what is missing.
    let mut plan = import::scan(&root, &options()).unwrap();
    assert!(plan.write().is_err());
    let mut project = Project::load(&root).unwrap();
    plan.libraries.iter_mut().for_each(|l| l.enabled = true);
    assert_eq!(plan.add_to(&mut project).unwrap(), ["tb"]);
    assert_eq!(project.libraries.len(), 4);
}

#[test]
fn the_search_path_of_an_xschemrc_gives_libraries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("chip");
    let shared = dir.path().join("shared");
    write(&root.join("top.sch"), SCH);
    write(&root.join("blocks/amp.sch"), SCH);
    write(&root.join("blocks/amp.sym"), SYM);
    write(&shared.join("stdlib/buf.sym"), SYM);
    write(&shared.join("stdlib/buf.sch"), SCH);
    write(&root.join("simulations/top.spice"), "");
    write(
        &root.join("xschemrc"),
        &format!(
            "set XSCHEM_LIBRARY_PATH {{}}\n\
             append XSCHEM_LIBRARY_PATH :${{XSCHEM_SHAREDIR}}/xschem_library/devices\n\
             append XSCHEM_LIBRARY_PATH :[file dirname [info script]]\n\
             append XSCHEM_LIBRARY_PATH :{}\n\
             append XSCHEM_LIBRARY_PATH :$env(NOWHERE)/x\n\
             set netlist_dir [file dirname [info script]]/simulations\n",
            shared.display()
        ),
    );
    let plan = import::scan(&root, &options()).unwrap();
    assert_eq!(plan.xschem_rc, Some(root.join("xschemrc")));
    let found: Vec<(&str, String, Origin)> = plan
        .libraries
        .iter()
        .map(|l| (l.name.as_str(), plan.path_text(&l.folder), l.origin))
        .collect();
    assert_eq!(
        found,
        [
            ("blocks", "blocks".to_string(), Origin::XschemPath),
            ("chip", ".".to_string(), Origin::XschemPath),
            (
                "stdlib",
                shared.join("stdlib").to_string_lossy().into_owned(),
                Origin::XschemPath
            ),
        ]
    );
    assert_eq!(plan.skipped.len(), 1, "{:?}", plan.skipped);
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("could not be worked out"))
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("xschem's own libraries"))
    );
    let project = plan.write().unwrap();
    assert_eq!(
        project.xschem_rc,
        [root.canonicalize().unwrap().join("xschemrc")]
    );
    assert_eq!(project.libraries.len(), 3);
    assert!(project.libraries.iter().all(|l| l.error.is_none()));
}

#[test]
fn a_libman_project_file_gives_libraries_and_categories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("door");
    for cell in ["inv", "nand2"] {
        write(&root.join(format!("mylib/{cell}/{cell}.gds")), "");
        write(&root.join(format!("mylib/{cell}/{cell}.sch")), SCH);
        write(
            &root.join(format!("mylib/{cell}/{cell}.schematic.room")),
            "",
        );
    }
    write(&root.join("mylib/Logic.group"), "inv nand2\n");
    write(&root.join("prims/R/R.sym"), SYM);
    write(&root.join("extra/top/top.gds"), "");
    write(
        &root.join("chip.projects"),
        "# KLayout library definition file\n\
         define(\"design\", \"mylib\\\\inv\\\\inv.gds\");\n\
         define(\"design\", \"mylib\\\\nand2\\\\nand2.gds\");\n\
         define(\"design\", \"extra\\\\top\\\\top.gds\");\n\
         define(\"primitives\", \"prims\\\\*\");\n\
         define(\"gone\", \"nowhere/*\");\n\
         pdk(\"ihp-sg13g2\", \"C:\\\\PDK\\\\ihp-sg13g2\");\n\
         attach(\"design\", \"primitives\");\n",
    );
    let plan = import::scan(&root, &options()).unwrap();
    let found: Vec<(&str, String, &str, usize, Origin)> = plan
        .libraries
        .iter()
        .map(|l| {
            (
                l.name.as_str(),
                plan.path_text(&l.folder),
                l.layout.name(),
                l.cells,
                l.origin,
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("design", "mylib".to_string(), "cells", 2, Origin::LibMan),
            ("extra", "extra".to_string(), "cells", 1, Origin::LibMan),
            (
                "primitives",
                "prims".to_string(),
                "cells",
                1,
                Origin::LibMan
            ),
        ]
    );
    assert!(plan.libraries[0].note.as_deref().unwrap().contains("ROOM"));
    assert!(
        plan.libraries[1]
            .note
            .as_deref()
            .unwrap()
            .contains("'design'")
    );
    assert_eq!(plan.technology.as_deref(), Some("ihp-sg13g2"));
    for word in ["'gone'", "attach()", "categor", "not at"] {
        assert!(
            plan.notes.iter().any(|n| n.contains(word)),
            "{word}: {:?}",
            plan.notes
        );
    }
    let project = plan.write().unwrap();
    let design = project.library("design").unwrap();
    assert_eq!(design.cells.len(), 2);
    assert_eq!(design.cell("inv").unwrap().entries.len(), 2);
    assert_eq!(
        category::load(&project, design).all.cells("Logic"),
        ["inv", "nand2"]
    );

    // And back: the libraries as a LibMan project file.
    let text = import::libman::export(&project);
    let back = import::libman::parse(&text);
    assert!(back.problems.is_empty(), "{:?}", back.problems);
    let names: Vec<&str> = back.libraries.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["design", "extra", "primitives"]);
    assert!(back.libraries[0].paths[0].ends_with("/mylib/*"));
}

/// Tests that need an installed PDK.
mod real_pdk {
    use super::*;

    #[test]
    fn the_xschem_folder_of_a_pdk_gives_its_libraries_read_only() {
        for pdk in ["sky130A", "ihp-sg13g2"] {
            let Some(real) = libcdept::testutil::real_pdk(pdk) else {
                continue;
            };
            let pdk_root = real.root.clone();
            let dir = tempfile::tempdir().unwrap();
            let source = pdk_root.join(pdk).join("libs.tech/xschem");
            let plan = import::scan(
                &source,
                &Options {
                    into: Some(dir.path().join("p")),
                    name: Some("p".into()),
                    ..options()
                },
            )
            .unwrap();
            assert_eq!(plan.technology.as_deref(), Some(pdk), "{}", plan.describe());
            assert_eq!(plan.pdk_root.as_deref(), Some(pdk_root.as_path()));
            assert!(plan.write_pdk_root);
            assert!(!plan.libraries.is_empty());
            assert!(
                plan.libraries.iter().all(|l| l.read_only),
                "{}",
                plan.describe()
            );
            let primitives = if pdk == "sky130A" {
                "sky130_fd_pr"
            } else {
                "sg13g2_pr"
            };
            let lib = plan
                .libraries
                .iter()
                .find(|l| l.name == primitives)
                .unwrap();
            assert!(lib.enabled && lib.cells > 20, "{}", plan.describe());
            assert_eq!(
                plan.path_text(&lib.folder),
                format!("$PDK_ROOT/{pdk}/libs.tech/xschem/{primitives}")
            );
            assert!(
                plan.libraries.iter().any(|l| !l.enabled),
                "{}",
                plan.describe()
            );
            let project = plan.write().unwrap();
            let lib = project.library(primitives).unwrap();
            assert!(lib.read_only && lib.error.is_none() && lib.cells.len() > 20);
            assert_eq!(project.technology.as_deref(), Some(pdk));
        }
    }
}

#[test]
fn an_xschemrc_is_run_by_xschem_when_the_helper_is_built() {
    let _env = libcdept::testutil::hermetic_env();
    if libcdept::xschem::Engine::find_helper().is_none() {
        // The test runs from target/<profile>/deps; the helper is built next to it.
        let built = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.parent()?.join("cdept-xschem")))
            .filter(|p| p.is_file());
        match built {
            // SAFETY: set before any engine starts; no other test of this file reads it.
            Some(path) => unsafe { std::env::set_var("CDEPT_XSCHEM", path) },
            None => {
                eprintln!("skipped: the xschem helper was not found (cargo build -p cdept-xschem)");
                return;
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("chip");
    write(&root.join("libs/amp/ota.sym"), SYM);
    write(&root.join("libs/amp/ota.sch"), SCH);
    write(&root.join("libs/dig/inv.sym"), SYM);
    // A condition and a loop: reading the text would not get this right.
    write(
        &root.join("xschemrc"),
        "if {1} { set here [file dirname [info script]] } else { set here /nowhere }\n\
         foreach lib {amp dig} { append XSCHEM_LIBRARY_PATH :$here/libs/$lib }\n",
    );
    let plan = import::scan(
        &root,
        &Options {
            helpers: true,
            ..options()
        },
    )
    .unwrap();
    let found: Vec<(&str, Origin)> = plan
        .libraries
        .iter()
        .map(|l| (l.name.as_str(), l.origin))
        .collect();
    assert_eq!(
        found,
        [("amp", Origin::XschemPath), ("dig", Origin::XschemPath)],
        "{}",
        plan.describe()
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("xschem's own libraries"))
    );
}
