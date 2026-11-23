//! The KLayout, Magic and netgen sections of the technology snapshot on real PDKs.
//!
//! Real-PDK tests: each skips when its PDK is not installed (`$CDEPT_TEST_PDK_ROOT`, else
//! `/opt/ext/OpenPDKs`; see `libcdept::testutil`) and runs with `PDK` and `PDK_ROOT` set
//! to it. The tests that ask KLayout skip when it is not built (`cdept-klayout/.bin`) or
//! found.

use libcdept::testutil::{RealPdk, real_pdk};
use libcdept::klayout::setup;
use libcdept::tech::klayout::{self, KlayoutCatalog};
use libcdept::tech::{PdkDir, ScanContext, magic, netgen};

/// The installed PDK, and the guard that keeps the environment for it.
fn pdk(name: &str) -> Option<(PdkDir, RealPdk)> {
    let real = real_pdk(name)?;
    Some((PdkDir::new(real.root.clone(), name), real))
}

fn has_klayout() -> bool {
    let found = setup::find_klayout().is_some_and(|k| k.is_file())
        && setup::find_helper_dir().is_some_and(|d| d.join(klayout::PCELL_SCRIPT).is_file());
    if !found {
        eprintln!("skipped: KLayout or its helper script is not here");
    }
    found
}

fn layer<'a>(catalog: &'a KlayoutCatalog, name: &str) -> &'a klayout::Layer {
    catalog
        .layers
        .iter()
        .find(|l| l.name == name)
        .unwrap_or_else(|| panic!("no layer {name}"))
}

#[test]
fn ihp_klayout_from_files() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        return;
    };
    let scanned = klayout::scan(&ScanContext::new(pdk).files_only());
    let s = &scanned.section;
    assert_eq!(s.tech.as_deref(), Some("sg13g2"));
    assert_eq!(s.dbu, Some(0.001));
    assert_eq!(s.path[0].as_str(), "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout");
    assert!(
        s.lyt
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("tech/sg13g2.lyt")
    );
    assert!(
        s.layer_properties
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("tech/sg13g2.lyp")
    );
    assert_eq!(s.default_grid, Some(0.005));
    assert!(s.connectivity.contains(&"Metal1,Via1,Metal2".to_string()));
    // The decks klayout::verify::find_deck picks are the first ones.
    assert!(s.drc[0].file.as_str().ends_with("tech/drc/ihp-sg13g2.drc"));
    assert!(s.lvs[0].file.as_str().ends_with("tech/lvs/sg13g2.lvs"));
    assert!(!s.layer_maps.is_empty());

    let c = &scanned.catalog;
    assert!(c.layers.len() > 200, "{} layers", c.layers.len());
    let metal1 = layer(c, "Metal1.drawing");
    assert_eq!((metal1.layer, metal1.datatype), (8, 0));
    assert_eq!(
        (metal1.base.as_str(), metal1.purpose.as_str()),
        ("Metal1", "drawing")
    );
    assert!(metal1.fill_color.starts_with('#'));
    let pin = layer(c, "Metal1.pin");
    assert_eq!((pin.layer, pin.datatype), (8, 2));
    assert!(!c.dither_patterns.is_empty());
    assert!(
        c.symbols
            .iter()
            .any(|s| s.name == "Metal1" && s.expression == "8/0-8/29")
    );
    assert!(
        c.layer_map
            .iter()
            .any(|m| m.name == "Metal1" && m.layer == 8)
    );

    let drc = c
        .decks
        .iter()
        .find(|d| d.check == "drc" && d.default)
        .unwrap();
    let role = |deck: &klayout::DeckDetail, role: &str| {
        deck.switches
            .iter()
            .find(|s| s.role == role)
            .map(|s| s.name.clone())
    };
    assert_eq!(role(drc, "input").as_deref(), Some("input"));
    assert_eq!(role(drc, "topcell").as_deref(), Some("topcell"));
    assert_eq!(role(drc, "report").as_deref(), Some("report"));
    let feol = drc.switches.iter().find(|s| s.name == "no_feol").unwrap();
    assert_eq!(
        (feol.kind.as_str(), feol.default.as_str()),
        ("flag", "false")
    );
    assert!(drc.rule_files > 10);
    assert!(drc.wrapper.is_some());
    let lvs = c
        .decks
        .iter()
        .find(|d| d.check == "lvs" && d.default)
        .unwrap();
    assert_eq!(role(lvs, "schematic").as_deref(), Some("schematic"));
    assert_eq!(
        role(lvs, "target_netlist").as_deref(),
        Some("target_netlist")
    );
    assert!(!c.macros.is_empty());

    // Without KLayout only the library of plain classes is read; the device library
    // is built at run time, and a note says so.
    assert_eq!(c.pcell_source, "files");
    assert!(c.pcell_libraries.iter().all(|l| l.name != "SG13_dev"));
    assert!(scanned.notes.iter().any(|n| n.contains("made at run time")));
}

#[test]
fn ihp_pcells_through_klayout() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        return;
    };
    if !has_klayout() {
        return;
    }
    let started = std::time::Instant::now();
    let scanned = klayout::scan(&ScanContext::new(pdk));
    eprintln!(
        "scan with KLayout: {:.2} s",
        started.elapsed().as_secs_f64()
    );
    let c = &scanned.catalog;
    assert_eq!(c.pcell_source, "klayout", "{:?}", scanned.notes);
    assert!(c.program.contains("KLayout"));
    assert!(
        scanned
            .section
            .pcell_libraries
            .contains(&"SG13_dev".to_string())
    );
    let dev = c
        .pcell_libraries
        .iter()
        .find(|l| l.name == "SG13_dev")
        .unwrap();
    assert_eq!(dev.technologies, ["sg13g2"]);
    assert!(c.pcell_libraries.iter().all(|l| l.name != "Basic"));
    let nmos = dev.pcells.iter().find(|p| p.name == "nmos").unwrap();
    let parameter = |name: &str| {
        nmos.parameters
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("nmos has no {name}"))
    };
    assert_eq!(parameter("model").default, "sg13_lv_nmos");
    assert_eq!(parameter("w").kind, "string");
    assert!(parameter("w").default.ends_with('u'));
    assert_eq!(parameter("ng").default, "1");
    assert!(dev.pcells.iter().any(|p| p.name == "cmim"));
    assert!(dev.pcells.iter().any(|p| p.name == "rsil"));
}

#[test]
fn sky130_klayout() {
    let Some((pdk, _env)) = pdk("sky130A") else {
        return;
    };
    let scanned = klayout::scan(&ScanContext::new(pdk).files_only());
    let s = &scanned.section;
    assert_eq!(s.tech.as_deref(), Some("sky130"));
    assert!(
        s.lyt
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("tech/sky130A.lyt")
    );
    assert!(s.drc[0].file.as_str().ends_with("drc/sky130A_mr.drc"));
    assert!(s.lvs[0].file.as_str().ends_with("lvs/sky130.lvs"));
    assert!(s.drc.iter().any(|d| d.partial));
    let c = &scanned.catalog;
    let met1 = c
        .layers
        .iter()
        .find(|l| l.base == "met1" && l.purpose == "drawing")
        .expect("met1.drawing");
    assert_eq!((met1.layer, met1.datatype), (68, 20));
    // sky130's deck calls the top cell `top_cell`, IHP's `topcell`: the role tells.
    let drc = c
        .decks
        .iter()
        .find(|d| d.check == "drc" && d.default)
        .unwrap();
    let topcell = drc.switches.iter().find(|s| s.role == "topcell").unwrap();
    assert_eq!(topcell.name, "top_cell");
    assert!(
        drc.switches
            .iter()
            .any(|s| s.name == "feol" && s.kind == "flag")
    );
    // The PCells are plain classes: read out of the Python sources.
    assert_eq!(c.pcell_source, "files");
    let library = &c.pcell_libraries[0];
    assert_eq!(library.name, "skywater130");
    let nfet = library.pcells.iter().find(|p| p.name == "nfet").unwrap();
    let kind = nfet.parameters.iter().find(|p| p.name == "type").unwrap();
    assert!(
        kind.choices
            .contains(&"sky130_fd_pr__nfet_01v8".to_string())
    );
    assert_eq!(kind.default, "sky130_fd_pr__nfet_01v8");
    let w = nfet.parameters.iter().find(|p| p.name == "w").unwrap();
    assert_eq!((w.kind.as_str(), w.unit.as_str()), ("double", "um"));
    assert!(w.default.parse::<f64>().is_ok(), "{}", w.default);
}

#[test]
fn ihp_magic_and_netgen() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        return;
    };
    let ctx = ScanContext::new(pdk);
    let scanned = magic::scan(&ctx);
    let s = &scanned.section;
    assert_eq!(s.tech.as_deref(), Some("ihp-sg13g2"));
    assert!(
        s.rc.as_ref()
            .unwrap()
            .as_str()
            .ends_with("ihp-sg13g2.magicrc")
    );
    assert!(
        s.tech_file
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("magic/ihp-sg13g2.tech")
    );
    assert!(s.tcl.as_ref().unwrap().as_str().ends_with("ihp-sg13g2.tcl"));
    assert_eq!(s.includes.len(), 4);
    assert_eq!(s.cifoutput[0], "gdsii");
    assert_eq!(s.extract[0], "ngspice");
    assert!(s.extract.contains(&"ngspice(lvs)".to_string()));
    assert!(s.drc[0].starts_with("drc("));
    assert_eq!(s.globals.get("GND").map(String::as_str), Some("VSS"));
    let c = &scanned.catalog;
    assert!(
        c.layers
            .iter()
            .any(|l| l.name == "nmos" && l.plane == "active")
    );
    assert!(
        c.gds
            .iter()
            .any(|g| g.direction == "output" && g.name == "NWELL")
    );
    assert!(
        c.gds
            .iter()
            .any(|g| g.direction == "input" && g.name == "NWELL")
    );
    assert!(
        c.extracted
            .iter()
            .any(|d| d.model == "sg13_lv_nmos" && d.layer == "nfet")
    );
    let nmos = c
        .devices
        .iter()
        .find(|d| d.name == "sg13_lv_nmos")
        .expect("a generator for sg13_lv_nmos");
    assert_eq!(nmos.namespace, "sg13g2");
    assert_eq!(nmos.class, "mosfet");
    assert!(nmos.draw);
    assert_eq!(nmos.description, "nmos (MOSFET)");
    assert!(
        nmos.parameters
            .iter()
            .any(|p| p.name == "w" && p.default == "0.6")
    );

    let scanned = netgen::scan(&ctx);
    assert!(
        scanned
            .section
            .setup
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("netgen/ihp-sg13g2_setup.tcl")
    );
    let mos = scanned
        .catalog
        .groups
        .iter()
        .find(|g| g.devices.contains(&"sg13_lv_nmos".to_string()))
        .expect("a group with the MOS transistors");
    assert_eq!(mos.permute, ["1 3"]);
    assert!(mos.parallel);
    assert!(mos.delete.contains(&"ng".to_string()));
}

#[test]
fn sky130_magic_and_netgen() {
    let Some((pdk, _env)) = pdk("sky130A") else {
        return;
    };
    let ctx = ScanContext::new(pdk);
    let scanned = magic::scan(&ctx);
    let s = &scanned.section;
    assert_eq!(s.tech.as_deref(), Some("sky130A"));
    assert!(
        s.tech_file
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("magic/sky130A.tech")
    );
    assert_eq!(s.globals.get("VDD").map(String::as_str), Some("VPWR"));
    assert!(s.cifinput.contains(&"sky130(vendor)".to_string()));
    assert!(s.scripts.iter().any(|x| x.role == "antenna"));
    assert!(
        s.cell_paths
            .iter()
            .any(|p| p.as_str().contains("libs.ref/sky130_fd_pr"))
    );
    let c = &scanned.catalog;
    let nfet = c
        .devices
        .iter()
        .find(|d| d.name == "sky130_fd_pr__nfet_01v8")
        .expect("a generator for nfet_01v8");
    assert_eq!(nfet.class, "mosfet");
    assert!(
        nfet.compatible
            .contains(&"sky130_fd_pr__nfet_01v8_lvt".to_string())
    );
    assert!(
        c.extracted
            .iter()
            .any(|d| d.model == "sky130_fd_pr__nfet_01v8")
    );
    assert!(scanned.notes.is_empty(), "{:?}", scanned.notes);

    let scanned = netgen::scan(&ctx);
    assert!(
        scanned
            .section
            .setup
            .as_ref()
            .unwrap()
            .as_str()
            .ends_with("netgen/sky130A_setup.tcl")
    );
    assert!(
        scanned.section.other_setups.is_empty(),
        "setup.tcl is a copy"
    );
    assert!(
        scanned
            .section
            .environment
            .contains(&"MAGIC_EXT_USE_GDS".to_string())
    );
    assert!(scanned.catalog.groups.len() >= 5);
    assert!(!scanned.catalog.patterns.is_empty());
}
