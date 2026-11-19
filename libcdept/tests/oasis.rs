//! The OASIS reader and renamer against files KLayout writes, and KLayout reading the
//! renamed files back. Skipped (with a note) when KLayout is not found, as in CI.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use libcdept::klayout::setup;
use libcdept::layout::oasis;

/// Run a Python script in `klayout -b`; its standard output.
fn klayout(dir: &Path, script: &str) -> Option<String> {
    let klayout = setup::find_klayout()?;
    let file = dir.join("script.py");
    std::fs::write(&file, script).unwrap();
    let out = Command::new(&klayout)
        .args(["-b", "-nc", "-r"])
        .arg(&file)
        .envs(setup::helper_env(&klayout))
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

const WRITE: &str = r#"
import pya
ly = pya.Layout()
ly.dbu = 0.001
top = ly.create_cell("buf")
inv = ly.create_cell("inv")
inv.ghost_cell = True
nand = ly.create_cell("nand2")
nand.ghost_cell = True
leaf = ly.create_cell("leaf")
l = ly.layer(1, 0)
leaf.shapes(l).insert(pya.Box(0, 0, 100, 200))
top.shapes(l).insert(pya.Box(0, 0, 1000, 2000))
top.insert(pya.CellInstArray(inv.cell_index(), pya.Trans(0, 0)))
top.insert(pya.CellInstArray(inv.cell_index(), pya.Trans(pya.Trans.R90, 5000, 0)))
top.insert(pya.CellInstArray(nand.cell_index(), pya.CplxTrans(1.5, 30, True, 7, 9)))
top.insert(pya.CellInstArray(leaf.cell_index(), pya.Trans(100, 100), pya.Vector(300, 0), pya.Vector(0, 300), 3, 2))
top.shapes(ly.layer(2, 0)).insert(pya.Text("A", pya.Trans(1, 1)))
top.shapes(l).insert(pya.Path([pya.Point(0, 0), pya.Point(1000, 0), pya.Point(1000, 500)], 50))
top.shapes(l).insert(pya.Polygon([pya.Point(0, 0), pya.Point(100, 300), pya.Point(400, 30)]))
top.shapes(l).insert(pya.Polygon(pya.DPolygon.ellipse(pya.DBox(0, 0, 3, 3), 16).to_itype(1)))
for x in (0, 1300, 1700, 4100):
    top.insert(pya.CellInstArray(leaf.cell_index(), pya.Trans(x, 3000)))
for i in range(40):
    top.shapes(l).insert(pya.Box(i * 10, 0, i * 10 + 5, 5))
top.set_property("key", 17)
for name, comp, strict, cblocks in [("plain", 0, False, False), ("comp", 2, False, False),
        ("strict", 0, True, False), ("cblock", 0, False, True), ("all", 10, True, True)]:
    o = pya.SaveLayoutOptions()
    o.format = "OASIS"
    o.oasis_compression_level = comp
    o.oasis_strict_mode = strict
    o.oasis_write_cblocks = cblocks
    o.oasis_write_cell_bounding_boxes = True
    o.oasis_write_std_properties = 2
    ly.write(name + ".oas", o)
ly.write("default.oas")
"#;

const READ: &str = r#"
import pya
for name in ["plain", "comp", "strict", "cblock", "all", "default"]:
    ly = pya.Layout()
    ly.read(name + ".renamed.oas")
    cells = sorted((c.name, c.is_ghost_cell()) for c in ly.each_cell())
    top = ly.cell("b")
    print(name, cells, top.bbox(), len(list(top.each_inst())), top.shapes(ly.layer(1, 0)).size())
"#;

#[test]
fn klayout_files_read_rename_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    if klayout(dir.path(), WRITE).is_none() {
        eprintln!("skipped: KLayout was not found");
        return;
    }
    let map = BTreeMap::from([
        ("inv".to_string(), "inverter_with_a_longer_name".to_string()),
        ("buf".to_string(), "b".to_string()),
    ]);
    for name in ["plain", "comp", "strict", "cblock", "all", "default"] {
        let file = dir.path().join(format!("{name}.oas"));
        let s = oasis::read_structure(&file).unwrap();
        assert_eq!(s.ghosts(), ["inv", "nand2"], "{name}");
        assert_eq!(s.top_cells(), ["buf"], "{name}");
        assert!(s.defines("leaf") && !s.defines("inv"), "{name}");
        assert!((s.user_per_dbu - 0.001).abs() < 1e-12, "{name}");

        if name == "strict" {
            // The checker agrees with KLayout's own offsets.
            assert!(oasis::verify_cell_offsets(&std::fs::read(&file).unwrap()).unwrap() > 0);
        }
        let renamed = dir.path().join(format!("{name}.renamed.oas"));
        std::fs::copy(&file, &renamed).unwrap();
        oasis::rename_structures(&renamed, &map).unwrap();
        let s = oasis::read_structure(&renamed).unwrap();
        assert_eq!(
            s.ghosts(),
            ["inverter_with_a_longer_name", "nand2"],
            "{name}"
        );
        assert_eq!(s.top_cells(), ["b"], "{name}");
        let bytes = std::fs::read(&renamed).unwrap();
        // No "buf" is left, S_TOP_CELL included (a PROPSTRING or inline).
        assert!(!bytes.windows(4).any(|w| w == b"\x03buf"), "{name}");
        let checked = oasis::verify_cell_offsets(&std::fs::read(&renamed).unwrap()).unwrap();
        // Strict mode (KLayout's default) writes S_CELL_OFFSET for the defined cells.
        assert_eq!(
            checked > 0,
            matches!(name, "strict" | "all" | "default"),
            "{name}"
        );
    }
    let out = klayout(dir.path(), READ).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 6, "{out}");
    for line in &lines {
        assert!(
            line.ends_with(
                "[('b', False), ('inverter_with_a_longer_name', True), ('leaf', False), \
                 ('nand2', True)] (0,-25;4200,3200) 8 44"
            ),
            "{out}"
        );
    }
}
