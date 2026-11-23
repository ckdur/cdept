//! Importing design files from outside the project into a library: planning, applying,
//! name clashes, rollback. The PDK test is skipped when the PDK is absent.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use libcdept::importer::{self, ImportRequest, OnClash};
use libcdept::layout::gds;
use libcdept::refactor::{self, Operation};
use libcdept::{AddLibrary, Format, Project};

fn write(path: &Path, bytes: impl AsRef<[u8]>) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    path.to_path_buf()
}

fn rec(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
    out.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(data);
}

/// A GDS stream with structures `(name, [placed names])`.
fn build(structures: &[(&str, &[&str])]) -> Vec<u8> {
    const UNITS: [u8; 16] = [
        0x3E, 0x41, 0x89, 0x37, 0x4B, 0xC6, 0xA7, 0xF0, 0x39, 0x44, 0xB8, 0x2F, 0xA0, 0x9B, 0x5A,
        0x53,
    ];
    let name = |t: &str| {
        let mut b = t.as_bytes().to_vec();
        if b.len() % 2 == 1 {
            b.push(0);
        }
        b
    };
    let dates = [0u8; 24];
    let mut out = Vec::new();
    rec(&mut out, 0x0002, &600i16.to_be_bytes());
    rec(&mut out, 0x0102, &dates);
    rec(&mut out, 0x0206, &name("LIB"));
    rec(&mut out, 0x0305, &UNITS);
    for (cell, refs) in structures {
        rec(&mut out, 0x0502, &dates);
        rec(&mut out, 0x0606, &name(cell));
        for r in *refs {
            rec(&mut out, 0x0A00, &[]);
            rec(&mut out, 0x1206, &name(r));
            rec(&mut out, 0x1003, &[0u8; 8]);
            rec(&mut out, 0x1100, &[]);
        }
        rec(&mut out, 0x0700, &[]);
    }
    rec(&mut out, 0x0400, &[]);
    out
}

const SPICE: &str = ".include \"models.spice\"\n.subckt inv A Y\nMN0 Y A 0 0 nmos\n.ends\n\
.subckt buf A Y\nX1 A n inv\nX2 n Y inv\n.ends\n";

fn project(dir: &Path) -> Project {
    let mut project = Project::create(dir, Some("demo")).unwrap();
    project
        .add_library(
            "lib",
            "libs/lib",
            AddLibrary {
                create: true,
                ..AddLibrary::default()
            },
        )
        .unwrap();
    project
}

/// Files outside the project: a GDS with two top cells and a ghost, a SPICE file.
fn sources(dir: &Path) -> (PathBuf, PathBuf) {
    let g = write(
        &dir.join("in/cells.gds"),
        build(&[
            ("inv", &["leaf"]),
            ("leaf", &[]),
            ("buf", &["leaf", "ghost"]),
        ]),
    );
    write(&dir.join("in/models.spice"), "* models\n");
    let s = write(&dir.join("in/cells.spice"), SPICE);
    (g, s)
}

fn views(project: &Project, cell: &str) -> BTreeSet<String> {
    let mut project = project.clone();
    for lib in &mut project.libraries {
        lib.scan();
    }
    project
        .library("lib")
        .unwrap()
        .cell(cell)
        .map(|c| c.entries.iter().map(|e| e.name.clone()).collect())
        .unwrap_or_default()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn files_become_cells_with_several_views() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("p"));
    let (gds_file, spice_file) = sources(tmp.path());
    let request = ImportRequest::new("lib", vec![gds_file.clone(), spice_file]);
    let planned = importer::plan_import_report(&project, &request).unwrap();

    // A dry run writes nothing.
    assert!(!tmp.path().join("p/libs/lib/inv").exists());
    let lines = planned.plan.lines(tmp.path()).join("\n");
    assert!(lines.contains("libs/lib/inv/layout/inv.gds"), "{lines}");
    assert!(lines.contains("libs/lib/buf/netlist/buf.spice"), "{lines}");
    // The ghost cell is reported.
    assert!(
        planned
            .report
            .problems
            .iter()
            .any(|p| p.contains("'ghost'") && p.contains("'buf'")),
        "{:?}",
        planned.report.problems
    );
    assert_eq!(planned.report.cells(), ["buf", "inv"]);
    assert_eq!(planned.report.imported.len(), 4);

    refactor::apply(&project, &planned.plan).unwrap();
    assert_eq!(views(&project, "inv"), set(&["layout", "netlist"]));
    let buf = gds::read_structure(&tmp.path().join("p/libs/lib/buf/layout/buf.gds")).unwrap();
    assert_eq!(buf.top_cells(), ["buf"]);
    assert!(buf.defines("leaf") && !buf.defines("inv"));
    // The netlist is usable on its own: the include is absolute, the used cell travels.
    let netlist = fs::read_to_string(tmp.path().join("p/libs/lib/buf/netlist/buf.spice")).unwrap();
    assert!(
        netlist.contains(".subckt inv") && netlist.contains(".subckt buf"),
        "{netlist}"
    );
    assert!(
        netlist.contains(&format!("{}/in/models.spice", tmp.path().display())),
        "{netlist}"
    );
    // The source is untouched.
    assert!(gds_file.is_file());
}

#[test]
fn clashes_skip_rename_or_overwrite() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("p"));
    let (gds_file, spice_file) = sources(tmp.path());
    let first = ImportRequest::new("lib", vec![gds_file.clone()]);
    importer::run(&project, &first).unwrap();
    let inv = tmp.path().join("p/libs/lib/inv/layout/inv.gds");
    let before = fs::read(&inv).unwrap();

    // Default: the existing view stays, the new views come in.
    let mut both = ImportRequest::new("lib", vec![gds_file.clone(), spice_file.clone()]);
    let planned = importer::plan_import_report(&project, &both).unwrap();
    assert_eq!(planned.report.imported.len(), 2, "{:?}", planned.report);
    assert!(planned.report.imported.iter().all(|i| i.view == "netlist"));
    assert!(planned
        .report
        .skipped
        .iter()
        .any(|s| s.contains("inv/layout")));
    refactor::apply(&project, &planned.plan).unwrap();
    assert_eq!(fs::read(&inv).unwrap(), before);
    assert!(tmp
        .path()
        .join("p/libs/lib/inv/netlist/inv.spice")
        .is_file());

    // Rename: the whole cell goes under another name.
    both.on_clash = OnClash::Rename;
    both.files = vec![gds_file.clone()];
    let (report, _) = importer::run(&project, &both).unwrap();
    assert_eq!(report.cells(), ["buf_1", "inv_1"]);
    let renamed =
        gds::read_structure(&tmp.path().join("p/libs/lib/inv_1/layout/inv_1.gds")).unwrap();
    assert_eq!(renamed.top_cells(), ["inv_1"]);

    // Overwrite only when asked.
    write(&inv, b"not a stream");
    both.on_clash = OnClash::Overwrite;
    let plan = importer::plan_import(&project, &both).unwrap();
    assert!(plan
        .lines(tmp.path())
        .join("\n")
        .contains("replaces the existing file"));
    let (report, _) = importer::run(&project, &both).unwrap();
    assert!(report
        .imported
        .iter()
        .any(|i| i.cell == "inv" && i.view == "layout"));
    assert_eq!(gds::read_structure(&inv).unwrap().top_cells(), ["inv"]);
}

#[test]
fn a_failure_puts_everything_back() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("p"));
    let (gds_file, _) = sources(tmp.path());
    let plan = importer::plan_import(&project, &ImportRequest::new("lib", vec![gds_file])).unwrap();
    // Something that is not a folder stands where the second cell must go.
    write(&tmp.path().join("p/libs/lib/inv"), b"in the way");
    let error = refactor::apply(&project, &plan).unwrap_err();
    assert!(format!("{error:#}").contains("put back"), "{error:#}");
    assert!(!tmp.path().join("p/libs/lib/buf").exists());
    assert_eq!(
        fs::read(tmp.path().join("p/libs/lib/inv")).unwrap(),
        b"in the way"
    );
}

#[test]
fn single_files_unknown_files_and_compressed_files() {
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("p"));
    let d = tmp.path().join("in");
    let head = "v {xschem version=3.4.5 file_version=1.2}\nG {}\nK {}\nV {}\nS {}\nE {}\n";
    let sch = write(&d.join("amp.sch"), head);
    let sym = write(&d.join("amp.sym"), head);
    let mag = write(&d.join("pad.mag"), "magic\ntech sky130A\n<< end >>\n");
    let vhd = write(&d.join("ctl.vhd"), "entity ctl is end;\n");
    let va = write(&d.join("res.va"), "module res(a,b); endmodule\n");
    let oas = write(&d.join("blk.oas"), b"%SEMI-OASIS\r\n");
    let unknown = write(&d.join("notes.txt"), "hello");
    let gz = write(&d.join("old.gds.gz"), [0x1f, 0x8b, 0, 0]);
    let request = ImportRequest::new("lib", vec![sch, sym, mag, vhd, va, oas, unknown, gz]);
    let (report, _) = importer::run(&project, &request).unwrap();
    assert!(report
        .problems
        .iter()
        .any(|p| p.contains("notes.txt") && p.contains("unknown format")));
    assert!(report
        .problems
        .iter()
        .any(|p| p.contains("old.gds.gz") && p.contains("compressed")));
    assert_eq!(views(&project, "amp"), set(&["schematic", "symbol"]));
    for (cell, view) in [
        ("pad", "magic"),
        ("ctl", "vhdl"),
        ("res", "veriloga"),
        ("blk", "oasis"),
    ] {
        assert_eq!(views(&project, cell), set(&[view]), "{cell}");
    }
    // The project reads them back with the right formats.
    let mut p = project.clone();
    for lib in &mut p.libraries {
        lib.scan();
    }
    let amp = p.library("lib").unwrap().cell("amp").unwrap();
    assert_eq!(
        amp.entry("symbol").unwrap().format,
        Some(Format::XschemSymbol)
    );

    // Only the formats asked for.
    let mut only = ImportRequest::new("lib", vec![d.join("amp.sch"), d.join("pad.mag")]);
    only.prefix = "n_".into();
    only.views = vec![Format::Magic];
    let (report, _) = importer::run(&project, &only).unwrap();
    assert_eq!(report.cells(), ["n_pad"]);
}

#[test]
fn collections_split_or_stay_whole() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("p"));
    let d = tmp.path().join("in");
    let lef = write(
        &d.join("std.lef"),
        "VERSION 5.8 ;\nLAYER m1\n TYPE ROUTING ;\nEND m1\nMACRO a\n CLASS CORE ;\nEND a\nMACRO b\nEND b\nEND LIBRARY\n",
    );
    let lib = write(
        &d.join("std_tt.lib"),
        "library (tt) {\n time_unit : \"1ns\";\n cell (a) { area : 1; }\n cell (b) { area : 2; }\n}\n",
    );
    let v = write(
        &d.join("std.v"),
        "module a (input x); endmodule\nmodule b (input x); a u (.x(x)); endmodule\n",
    );
    let (report, _) = importer::run(
        &project,
        &ImportRequest::new("lib", vec![lef.clone(), lib.clone(), v.clone()]),
    )
    .unwrap();
    assert_eq!(report.cells(), ["a", "b", "std_tech"]);
    assert_eq!(
        views(&project, "a"),
        set(&["abstract", "liberty", "verilog"])
    );
    assert_eq!(views(&project, "std_tech"), set(&["techlef"]));
    let b = fs::read_to_string(tmp.path().join("p/libs/lib/b/verilog/b.v")).unwrap();
    assert!(b.contains("module a ") && b.contains("module b "), "{b}");

    // Without splitting: one cell per file, named after it; with a prefix.
    let tmp2 = tempfile::tempdir().unwrap();
    let project = project_in(tmp2.path());
    let mut whole = ImportRequest::new("lib", vec![lef, v]);
    whole.split = false;
    whole.prefix = "x_".into();
    let (report, _) = importer::run(&project, &whole).unwrap();
    assert_eq!(report.cells(), ["x_std"]);
    assert_eq!(report.imported.len(), 2);
    assert!(report.imported.iter().any(|i| i.view == "abstract"));

    // A name map.
    let tmp3 = tempfile::tempdir().unwrap();
    let project = project_in(tmp3.path());
    let mut mapped = ImportRequest::new("lib", vec![lib]);
    mapped.names.insert("a".into(), "alpha".into());
    let (report, _) = importer::run(&project, &mapped).unwrap();
    assert_eq!(report.cells(), ["alpha", "b"]);
    let text = fs::read_to_string(tmp3.path().join("p/libs/lib/alpha/liberty/alpha.lib")).unwrap();
    assert!(text.contains("cell (alpha)"), "{text}");
}

fn project_in(dir: &Path) -> Project {
    project(&dir.join("p"))
}

#[test]
fn refuses_what_cannot_take_files() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("p"));
    let (gds_file, _) = sources(tmp.path());
    let e = importer::plan_import(
        &project,
        &ImportRequest::new("nope", vec![gds_file.clone()]),
    );
    assert!(e.unwrap_err().to_string().contains("no library"));
    let e = importer::plan_import(&project, &ImportRequest::new("lib", vec![]));
    assert!(e.is_err());
    let mut bad = ImportRequest::new("lib", vec![gds_file]);
    bad.prefix = "a b".into();
    assert!(importer::plan_import(&project, &bad).is_err());
    let sch = write(&tmp.path().join("in/x.sch"), "v {}\n");
    let plan = importer::plan_import(&project, &ImportRequest::new("lib", vec![sch])).unwrap();
    assert!(matches!(plan.operation, Operation::Import { .. }));
}

/// Tests that need an installed PDK.
mod real_pdk {
    use super::*;

    // ---------------------------------------------------------------------------------
    // The IHP standard cells
    // ---------------------------------------------------------------------------------

    /// The first file of a folder with one of the suffixes.
    fn first_in(dir: &Path, suffixes: &[&str]) -> Option<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| suffixes.iter().any(|s| p.to_string_lossy().ends_with(s)))
            .collect();
        found.sort();
        found.into_iter().next()
    }

    #[test]
    fn the_ihp_standard_cells_come_in_with_three_views() {
        let Some(pdk) = libcdept::testutil::real_pdk("ihp-sg13g2") else {
            return;
        };
        let std = pdk.dir.join("libs.ref/sg13g2_stdcell");
        if !std.join("gds").is_dir() {
            eprintln!("skipped: ihp-sg13g2 has no standard cell library");
            return;
        }
        let files: Vec<PathBuf> = [
            first_in(&std.join("gds"), &[".gds"]),
            first_in(&std.join("spice"), &[".spice", ".spi"]),
            first_in(&std.join("cdl"), &[".cdl"]),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(files.len(), 3, "{files:?}");
        let tmp = tempfile::tempdir().unwrap();
        // A project of the PDK (for the DRC deck), with a library of its own.
        let root = tmp.path().join("p");
        fs::create_dir_all(root.join("libs/lib")).unwrap();
        let pdk_root = &pdk.root;
        fs::write(
            root.join("cdept.toml"),
            format!(
                "name = \"t\"\ntechnology = \"ihp-sg13g2\"\npdk_root = \"{}\"\n[libraries]\nlib = \"libs/lib\"\n",
                pdk_root.display()
            ),
        )
        .unwrap();
        let project = Project::load(&root).unwrap();
        let planned =
            importer::plan_import_report(&project, &ImportRequest::new("lib", files)).unwrap();
        assert!(
            planned.report.imported.len() > 200,
            "{}",
            planned.report.imported.len()
        );
        refactor::apply(&project, &planned.plan).unwrap();
        assert_eq!(
            views(&project, "sg13g2_inv_1"),
            set(&["cdl", "layout", "netlist"])
        );
        let cell = tmp.path().join("p/libs/lib/sg13g2_inv_1");
        let g = gds::read_structure(&cell.join("layout/sg13g2_inv_1.gds")).unwrap();
        assert_eq!(g.top_cells(), ["sg13g2_inv_1"]);
        let netlist = first_in(&cell.join("netlist"), &[".spice", ".spi"]).unwrap();
        let n = fs::read_to_string(netlist).unwrap();
        assert!(n.contains(".subckt sg13g2_inv_1"), "{n}");

        // The imported layout passes the PDK's DRC deck.
        use libcdept::klayout::verify::{self, Check};
        if libcdept::klayout::setup::find_klayout().is_none()
            || libcdept::klayout::setup::find_helper_dir().is_none()
        {
            eprintln!("DRC skipped: KLayout or the cdept-klayout helper was not found");
            return;
        }
        let mut project = Project::load(&root).unwrap();
        for lib in &mut project.libraries {
            lib.scan();
        }
        let Some(deck) = verify::find_deck(&project, Check::Drc) else {
            eprintln!("DRC skipped: the PDK has no DRC deck");
            return;
        };
        let dir = tmp.path().join("drc");
        fs::create_dir_all(&dir).unwrap();
        let layout =
            libcdept::klayout::cell_layout(project.library("lib").unwrap(), "sg13g2_inv_1").unwrap();
        let stream =
            verify::export_stream(&project, &layout, "sg13g2_inv_1", &dir.join("inv.gds")).unwrap();
        assert!(stream.missing.is_empty(), "{:?}", stream.missing);
        let run = verify::run_drc(&project, &deck, &stream, &dir, &[]).unwrap();
        assert!(run.rdb.counts().is_empty(), "{:?}", run.rdb.counts());
    }
}
