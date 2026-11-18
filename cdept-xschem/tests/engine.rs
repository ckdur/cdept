//! End-to-end tests: start the helper, drive xschem through `libcdept::xschem::Engine`.

use std::path::{Path, PathBuf};

use libcdept::xschem::{Engine, EngineOptions, NetlistFormat, setup, symbol};
use libcdept::{AddLibrary, LibraryLayout, Project, Template};

fn options() -> EngineOptions {
    EngineOptions {
        helper: Some(PathBuf::from(env!("CARGO_BIN_EXE_cdept-xschem"))),
        ..Default::default()
    }
}

/// xschem's example library, part of the submodule.
fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("xschem/xschem_library/examples")
        .join(name)
}

#[test]
fn starts_and_evaluates_tcl() {
    let mut engine = Engine::start(options()).unwrap();
    assert!(
        engine.xschem_version().starts_with('3'),
        "{}",
        engine.xschem_version()
    );
    assert_eq!(engine.tcl("expr {6 * 7}").unwrap(), "42");
    // xschem's own command and globals are there, and it runs without a display.
    assert_eq!(
        engine.tcl("xschem get version").unwrap(),
        engine.xschem_version()
    );
    assert_eq!(engine.tcl("info exists has_x").unwrap(), "0");

    let error = engine.tcl("error {no good}").unwrap_err();
    assert_eq!(error.to_string(), "no good");
    // An error does not break the connection.
    assert_eq!(engine.tcl("string length abc").unwrap(), "3");
    engine.shutdown().unwrap();
}

#[test]
fn a_script_calling_exit_is_reported_not_hung() {
    let mut engine = Engine::start(options()).unwrap();
    let error = engine.tcl("exit 3").unwrap_err().to_string();
    assert!(error.contains("stopped"), "{error}");
}

#[test]
fn missing_helper_is_explained() {
    let error = Engine::start(EngineOptions {
        helper: Some(PathBuf::from("/nonexistent/cdept-xschem")),
        ..Default::default()
    })
    .err()
    .unwrap();
    assert!(error.to_string().contains("cannot start"), "{error}");
}

#[test]
fn loads_netlists_and_draws_an_example() {
    let out = tempfile::tempdir().unwrap();
    let mut engine = Engine::start(options()).unwrap();

    let schematic = example("cmos_inv.sch");
    engine.load(&schematic).unwrap();

    let instances = engine.instances().unwrap();
    let symbols: Vec<&str> = instances.iter().map(|i| i.symbol.as_str()).collect();
    assert!(symbols.contains(&"nmos4.sym"), "{symbols:?}");
    assert!(symbols.contains(&"pmos4.sym"), "{symbols:?}");
    assert!(
        instances.iter().all(|i| i.kind != "missing"),
        "unresolved symbols: {instances:?}"
    );

    let netlist = out.path().join("sub/cmos_inv.spice");
    engine.netlist(NetlistFormat::Spice, &netlist).unwrap();
    let text = std::fs::read_to_string(&netlist).unwrap();
    assert!(text.contains(".subckt cmos_inv"), "{text}");
    assert!(text.to_lowercase().contains(".ends"), "{text}");

    let svg = out.path().join("cmos_inv.svg");
    engine.export_svg(&svg).unwrap();
    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(text.contains("<svg"), "{}", &text[..text.len().min(200)]);
    assert!(text.matches("<path").count() > 5, "the drawing is empty");

    assert!(engine.load(&out.path().join("missing.sch")).is_err());
}

/// A schematic instantiating `reference` once, plus a resistor from xschem's devices.
fn top_schematic(reference: &str) -> String {
    format!(
        "v {{xschem version=3.4.5 file_version=1.2\n}}\nG {{}}\nK {{}}\nV {{}}\nS {{}}\nE {{}}\n\
         C {{{reference}}} 0 0 0 0 {{name=x1}}\n\
         C {{devices/res.sym}} 200 0 0 0 {{name=R1 value=1k}}\n"
    )
}

/// Build a project with one library holding cells `sub` (symbol + schematic) and `top`
/// (schematic instantiating `sub`), then netlist `top` through the generated start-up
/// file. Returns the netlist text.
fn netlist_project(layout: LibraryLayout) -> String {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), Some("demo")).unwrap();
    project
        .add_library(
            "mylib",
            "libs/mylib",
            AddLibrary {
                create: true,
                layout: Some(layout),
                ..Default::default()
            },
        )
        .unwrap();
    let library = project.library("mylib").unwrap().clone();
    let symbol = library.create_view("sub", Template::XschemSymbol).unwrap();
    let sub = library
        .create_view("sub", Template::XschemSchematic)
        .unwrap();
    // Something recognisable inside `sub`, to prove xschem descended into it.
    let mut contents = std::fs::read_to_string(&sub).unwrap();
    contents.push_str("C {res.sym} 0 0 0 0 {name=R9 value=2k}\n");
    std::fs::write(&sub, contents).unwrap();
    let top = library
        .create_view("top", Template::XschemSchematic)
        .unwrap();
    project.rescan();

    let reference = setup::reference(&project, &symbol).unwrap();
    std::fs::write(&top, top_schematic(&reference)).unwrap();

    let mut engine = Engine::start(EngineOptions {
        startup: Some(setup::startup_script(&project)),
        ..options()
    })
    .unwrap();
    engine.load(&top).unwrap();

    let instances = engine.instances().unwrap();
    let x1 = instances.iter().find(|i| i.name == "x1").unwrap();
    assert_eq!(x1.symbol, reference);
    assert_eq!(x1.kind, "subcircuit", "the symbol was not found by xschem");
    // xschem's devices resolve with the `devices/` prefix too.
    let r1 = instances.iter().find(|i| i.name == "R1").unwrap();
    assert_eq!(r1.kind, "resistor");

    let netlist = dir.path().join("out/top.spice");
    engine.netlist(NetlistFormat::Spice, &netlist).unwrap();
    std::fs::read_to_string(&netlist).unwrap()
}

#[test]
fn project_libraries_resolve_in_the_flat_layout() {
    let text = netlist_project(LibraryLayout::Flat);
    assert!(text.contains("x1 sub"), "{text}");
    assert!(
        text.contains(".subckt sub"),
        "sub-schematic not netlisted:\n{text}"
    );
    assert!(
        text.contains("R9"),
        "xschem did not descend into sub:\n{text}"
    );
}

/// With the symbol in `sub/symbol/` and the schematic in `sub/schematic/`, xschem finds
/// the schematic through the `schematic=` attribute CDePT writes into the symbol.
#[test]
fn project_libraries_resolve_in_the_cells_layout() {
    let text = netlist_project(LibraryLayout::Cells);
    assert!(text.contains("x1 sub"), "{text}");
    assert!(
        text.contains(".subckt sub"),
        "sub-schematic not netlisted:\n{text}"
    );
    assert!(
        text.contains("R9"),
        "xschem did not descend into sub:\n{text}"
    );
}

/// On a machine where xschem never ran, its folder in the home does not exist. Engines
/// started at the same moment must not trip over each other creating it.
#[test]
fn engines_start_together_in_a_fresh_home() {
    let home = tempfile::tempdir().unwrap();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let home = home.path().to_path_buf();
            std::thread::spawn(move || {
                let mut engine = Engine::start(EngineOptions {
                    env: vec![("HOME".into(), home.into())],
                    ..options()
                })?;
                engine.tcl("expr {6 * 7}")
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap().unwrap(), "42");
    }
    assert!(home.path().join(".xschem/xschemrc").is_file());
}

/// The example project of the repository: three levels across two libraries.
#[test]
fn the_example_project_netlists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/buffer");
    let project = Project::load(&root).unwrap();
    assert_eq!(project.warnings, Vec::<String>::new());
    assert_eq!(symbol::check(&project, None, None), []);

    let out = tempfile::tempdir().unwrap();
    let netlist = out.path().join("tb_buf.spice");
    let mut engine = Engine::start(EngineOptions {
        startup: Some(setup::startup_script(&project)),
        ..options()
    })
    .unwrap();
    engine
        .load(&root.join("libs/bench/tb_buf/schematic/tb_buf.sch"))
        .unwrap();
    engine.netlist(NetlistFormat::Spice, &netlist).unwrap();
    let text = std::fs::read_to_string(&netlist).unwrap();
    for line in [
        "x1 in out VDD 0 buf",
        "x2 net1 Y VDD VSS inv",
        "M1 Y A VSS VSS nmos w=1u l=0.35u",
    ] {
        assert!(text.contains(line), "missing `{line}` in:\n{text}");
    }
}

/// A copy of the example project that tests may write into.
fn example_copy() -> (tempfile::TempDir, Project) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/buffer");
    copy(&source, dir.path());
    let project = Project::load(dir.path()).unwrap();
    (dir, project)
}

fn engine_for(project: &Project) -> Engine {
    Engine::start(EngineOptions {
        helper: options().helper,
        ..EngineOptions::for_project(project)
    })
    .unwrap()
}

#[test]
fn the_hierarchy_lists_every_schematic_once() {
    let (dir, project) = example_copy();
    let mut engine = engine_for(&project);
    let top = dir.path().join("libs/bench/tb_buf/schematic/tb_buf.sch");
    let hierarchy = engine.hierarchy(&top).unwrap();

    let names: Vec<&str> = hierarchy.cells.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["tb_buf", "buf", "inv"]);
    let child = |cell: usize, instance: &str| {
        hierarchy.cells[cell]
            .instances
            .iter()
            .find(|i| i.instance.name == instance)
            .unwrap()
            .child
    };
    assert_eq!(child(0, "x1"), Some(1));
    assert_eq!(child(0, "C1"), None, "a capacitor is a primitive");
    // Both inverters of the buffer are the same schematic.
    assert_eq!(child(1, "x1"), Some(2));
    assert_eq!(child(1, "x2"), Some(2));
    assert!(
        hierarchy.cells[2]
            .instances
            .iter()
            .all(|i| i.child.is_none())
    );
}

#[test]
fn a_netlist_becomes_a_view_of_the_cell() {
    let (dir, mut project) = example_copy();
    let mut engine = engine_for(&project);
    let library = project.library("logic").unwrap().clone();
    let file = engine
        .netlist_view(&library, "buf", NetlistFormat::Spice)
        .unwrap();
    assert_eq!(file, dir.path().join("libs/logic/buf/netlist/buf.spice"));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("M1 Y A VSS VSS nmos"), "{text}");

    project.rescan();
    let cell = project.library("logic").unwrap().cell("buf").unwrap();
    assert!(cell.entry("netlist").is_some(), "{:?}", cell.entries);

    assert!(
        engine
            .netlist_view(&library, "nothing", NetlistFormat::Spice)
            .is_err()
    );
}

#[test]
fn a_symbol_made_from_a_schematic_goes_into_the_symbol_view() {
    let (dir, mut project) = example_copy();
    let symbol = dir.path().join("libs/logic/buf/symbol/buf.sym");
    let beside = dir.path().join("libs/logic/buf/schematic/buf.sym");
    let mut engine = engine_for(&project);
    let library = project.library("logic").unwrap().clone();

    // The cell has a symbol: it is kept unless asked otherwise.
    let before = std::fs::read_to_string(&symbol).unwrap();
    let error = engine.make_symbol(&library, "buf", false).unwrap_err();
    assert!(error.to_string().contains("exists already"), "{error}");
    assert_eq!(std::fs::read_to_string(&symbol).unwrap(), before);

    // Without a symbol view: xschem's generator makes one, in the view folder, and it
    // names its schematic the way CDePT writes it.
    std::fs::remove_dir_all(symbol.parent().unwrap()).unwrap();
    project.rescan();
    let library = project.library("logic").unwrap().clone();
    assert_eq!(engine.make_symbol(&library, "buf", false).unwrap(), symbol);
    assert!(
        !beside.exists(),
        "a symbol was written next to the schematic"
    );
    let text = std::fs::read_to_string(&symbol).unwrap();
    assert_eq!(
        symbol::schematic(&text).as_deref(),
        Some("logic/@symname/schematic/@symname.sch")
    );
    assert_ne!(
        text, before,
        "this is the generated symbol, not the drawn one"
    );
    project.rescan();
    assert_eq!(symbol::check(&project, None, None), []);
    assert_eq!(engine.make_symbol(&library, "buf", true).unwrap(), symbol);

    // xschem finds the schematic behind the new symbol. (The testbench of the example
    // cannot be used: its wires end where the pins of the drawn symbol were.)
    let top = dir.path().join("libs/bench/top/schematic/top.sch");
    std::fs::create_dir_all(top.parent().unwrap()).unwrap();
    std::fs::write(&top, top_schematic("logic/buf/symbol/buf.sym")).unwrap();
    let hierarchy = engine.hierarchy(&top).unwrap();
    let x1 = hierarchy.cells[0]
        .instances
        .iter()
        .find(|i| i.instance.name == "x1")
        .unwrap();
    let child = &hierarchy.cells[x1.child.expect("x1 has no schematic")];
    assert_eq!(
        child.schematic,
        dir.path().join("libs/logic/buf/schematic/buf.sch")
    );

    // In a flat library the symbol stays next to the schematic, as xschem has it.
    project
        .add_library(
            "flat",
            "libs/flat",
            AddLibrary {
                create: true,
                layout: Some(LibraryLayout::Flat),
                ..Default::default()
            },
        )
        .unwrap();
    std::fs::copy(
        dir.path().join("libs/logic/inv/schematic/inv.sch"),
        dir.path().join("libs/flat/inv.sch"),
    )
    .unwrap();
    project.rescan();
    let flat = project.library("flat").unwrap().clone();
    let mut engine = engine_for(&project);
    let symbol = engine.make_symbol(&flat, "inv", false).unwrap();
    assert_eq!(symbol, dir.path().join("libs/flat/inv.sym"));
    assert_eq!(
        symbol::schematic(&std::fs::read_to_string(&symbol).unwrap()),
        None
    );
}

#[test]
fn the_service_runs_jobs_in_the_background_and_recovers() {
    use libcdept::xschem::Service;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let woken = Arc::new(AtomicUsize::new(0));
    let service = {
        let woken = woken.clone();
        Service::new(
            options(),
            Arc::new(move || {
                woken.fetch_add(1, Ordering::SeqCst);
            }),
        )
    };
    fn wait<T>(pending: &libcdept::xschem::Pending<T>) -> Result<T, String> {
        for _ in 0..4000 {
            if let Some(result) = pending.poll() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the job did not finish");
    }

    let first = service.run(|engine| engine.tcl("expr {6 * 7}"));
    let second = service.run(|engine| engine.tcl("set cdept_marker 1"));
    assert_eq!(wait(&first).unwrap(), "42");
    assert_eq!(wait(&second).unwrap(), "1");
    // The same xschem served both.
    let marker = service.run(|engine| engine.tcl("set cdept_marker"));
    assert_eq!(wait(&marker).unwrap(), "1");

    // A job that kills the helper fails; the next one gets a new xschem.
    let killed = service.run(|engine| engine.tcl("exit 3"));
    assert!(wait(&killed).unwrap_err().contains("stopped"));
    let after = service.run(|engine| engine.tcl("info exists cdept_marker"));
    assert_eq!(wait(&after).unwrap(), "0");
    assert_eq!(woken.load(Ordering::SeqCst), 5);
}
