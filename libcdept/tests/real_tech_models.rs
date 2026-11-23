//! Real-PDK tests: the xschem, models and devices sections of the technology snapshot on
//! the PDKs of this machine (skipped when they are absent, see `libcdept::testutil`): IHP
//! SG13G2 and sky130A.
//!
//! `cargo test -p libcdept --test real_tech_models -- --nocapture` prints sizes and times.

use std::time::Instant;

use libcdept::link::devices::DeviceMap;
use libcdept::tech::{self, Catalog, PdkDir, ScanContext, Tech, devices, models, xschem};
use libcdept::testutil::{RealPdk, real_pdk};

/// The installed PDK, and the guard that keeps the environment for it.
fn pdk(name: &str) -> Option<(PdkDir, RealPdk)> {
    let real = real_pdk(name)?;
    Some((PdkDir::new(real.root.clone(), name), real))
}

/// Make the xschem helper findable from a test (it is built next to `deps/`).
fn helper() -> bool {
    if libcdept::xschem::Engine::find_helper().is_some() {
        return true;
    }
    let built = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("cdept-xschem")))
        .filter(|p| p.is_file());
    match built {
        // SAFETY: every test sets the same value, before an engine starts.
        Some(path) => {
            unsafe { std::env::set_var("CDEPT_XSCHEM", path) };
            true
        }
        None => {
            eprintln!("the xschem helper was not found (cargo build -p cdept-xschem): text only");
            false
        }
    }
}

/// The three sections, without the others (KLayout takes its own time).
fn scan(pdk: &PdkDir, helpers: bool) -> (Tech, Catalog) {
    let mut ctx = ScanContext::new(pdk.clone());
    ctx.helpers = helpers;
    let mut tech = Tech::default();
    let mut catalog = Catalog::default();
    tech.pdk.name = pdk.name.clone();
    let t = Instant::now();
    let x = xschem::scan(&ctx);
    let tx = t.elapsed();
    let t = Instant::now();
    let m = models::scan(&ctx);
    let tm = t.elapsed();
    tech.xschem = x.section;
    catalog.xschem = x.catalog;
    tech.models = m.section;
    catalog.models = m.catalog;
    tech.notes.extend(x.notes);
    tech.notes.extend(m.notes);
    let t = Instant::now();
    let d = devices::scan(&ctx, &tech, &catalog);
    let td = t.elapsed();
    tech.devices = d.section;
    catalog.devices = d.catalog;
    tech.notes.extend(d.notes);
    let toml_len = toml::to_string_pretty(&tech).unwrap().len();
    let json_len = serde_json::to_string(&catalog).unwrap().len();
    eprintln!(
        "{} (helpers: {helpers}): xschem {tx:.2?} ({} symbols), models {tm:.2?}, devices {td:.2?} \
         ({} devices); tech.toml part {toml_len} bytes, catalog part {json_len} bytes",
        pdk.name,
        catalog.xschem.symbols.len(),
        catalog.devices.devices.len(),
    );
    for note in &tech.notes {
        eprintln!("  note: {note}");
    }
    (tech, catalog)
}

fn var_for(pdk: &PdkDir) -> impl Fn(&str) -> Option<String> + '_ {
    move |name| match name {
        "PDK_ROOT" => Some(pdk.root.to_string_lossy().into_owned()),
        "PDK" => Some(pdk.name.clone()),
        _ => None,
    }
}

/// Every file a corner set names exists, and every section is in its file.
fn check_corners(pdk: &PdkDir, tech: &Tech) {
    for (name, set) in &tech.models.sets {
        assert!(!set.corners.is_empty(), "{name}: no corner sets");
        for corner in set.corners.keys() {
            for lib in set.corner(corner).unwrap() {
                let file = lib.file.resolve_in(pdk).unwrap();
                let text = std::fs::read_to_string(&file)
                    .unwrap_or_else(|e| panic!("{name}/{corner}: {}: {e}", file.display()));
                if !lib.section.is_empty() {
                    let found = text.lines().any(|l| {
                        let mut words = l.split_whitespace();
                        words.next().is_some_and(|w| w.eq_ignore_ascii_case(".lib"))
                            && words.next() == Some(lib.section.as_str())
                    });
                    assert!(
                        found,
                        "{name}/{corner}: no section {} in {}",
                        lib.section,
                        file.display()
                    );
                }
            }
        }
        for file in set.init.iter().chain(&set.plugins) {
            assert!(
                file.resolve_in(pdk).unwrap().exists(),
                "{name}: {file} is missing"
            );
        }
    }
}

#[test]
fn ihp_sg13g2() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        return;
    };
    let (tech, catalog) = scan(&pdk, helper());

    // xschem
    let x = &tech.xschem;
    assert_eq!(
        x.rc.as_ref().unwrap().as_str(),
        "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc"
    );
    let library = |name: &str| x.libraries.iter().find(|l| l.name == name);
    let pr = library("sg13g2_pr").expect("sg13g2_pr");
    assert_eq!(
        (pr.role.as_str(), pr.enabled, pr.layout.as_str()),
        ("primitives", true, "flat")
    );
    assert_eq!(
        pr.path.as_str(),
        "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr"
    );
    assert!(!library("sg13g2_tests").expect("tests").enabled);
    let std = library("sg13g2_stdcell").expect("stdcells");
    assert_eq!(std.role, "stdcells");
    assert_eq!(
        std.path.as_str(),
        "$PDK_ROOT/ihp-sg13g2/libs.ref/sg13g2_stdcell/sym/xschem"
    );
    assert_eq!(x.references["sg13g2_pr"], "sg13g2_pr/");
    assert_eq!(
        x.variables["SG13G2_MODELS"],
        "$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/models"
    );
    assert_eq!(x.netlist_dir.as_deref(), Some("$PWD/simulations"));
    if x.evaluated {
        let (text, _) = scan(&pdk, false);
        assert_eq!(text.xschem.libraries, x.libraries);
        assert_eq!(text.xschem.variables, x.variables);
        assert_eq!(text.xschem.netlist_dir, x.netlist_dir);
    }
    assert!(
        x.start_window
            .as_ref()
            .is_some_and(|w| w.as_str().ends_with("start_page.sch")),
        "{:?}",
        x.start_window
    );
    let nmos = catalog
        .xschem
        .symbols
        .iter()
        .find(|s| s.name == "sg13_lv_nmos")
        .expect("sg13_lv_nmos");
    assert_eq!(
        (
            nmos.kind.as_str(),
            nmos.model.as_str(),
            nmos.prefix.as_str()
        ),
        ("nmos", "sg13_lv_nmos", "X")
    );
    assert_eq!(nmos.pins, ["D", "G", "S", "B"]);
    assert_eq!(nmos.template["w"], "0.15u");

    // models
    let m = &tech.models;
    assert_eq!(m.default.as_deref(), Some("ngspice"));
    assert_eq!(m.corner.as_deref(), Some("tt"));
    assert_eq!(m.simulators(), ["ngspice", "xyce"]);
    let ng = &m.sets["ngspice"];
    assert_eq!(
        ng.dir.as_str(),
        "$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/models"
    );
    assert!(ng.libraries.iter().any(|l| l == "cornerMOSlv.lib"));
    for corner in ["tt", "ff", "ss", "sf", "fs", "mc", "tt_mm"] {
        assert!(
            ng.corners.contains_key(corner),
            "no corner {corner}: {:?}",
            ng.corner_names()
        );
    }
    assert!(ng.corners["tt"].contains(&"cornerMOSlv.lib mos_tt".to_string()));
    assert!(ng.corners["tt"].contains(&"cornerRES.lib res_typ".to_string()));
    assert!(ng.corners["ff"].contains(&"cornerRES.lib res_bcs".to_string()));
    assert!(ng.corners["ss"].contains(&"cornerHBT.lib hbt_wcs".to_string()));
    assert!(ng.corners["sf"].contains(&"cornerMOShv.lib mos_sf".to_string()));
    assert!(ng.corners["mc"].contains(&"cornerMOSlv.lib mos_tt_stat".to_string()));
    assert!(
        ng.init[0]
            .as_str()
            .ends_with("libs.tech/ngspice/.spiceinit")
    );
    assert!(ng.osdi.iter().any(|f| f.as_str().ends_with("psp103.osdi")));
    assert!(
        ng.search_path
            .iter()
            .any(|f| f.as_str().ends_with("libs.tech/ngspice/models"))
    );
    assert_eq!(ng.tnom, Some(27.0));
    assert!(m.verilog_a.is_some());
    check_corners(&pdk, &tech);
    let lines = m.lines("ngspice", "tt", var_for(&pdk)).unwrap();
    assert!(
        lines.iter().any(|l| l.ends_with("/cornerMOSlv.lib mos_tt")),
        "{lines:?}"
    );

    let defs = &catalog.models.sets["ngspice"];
    let def = defs.definition("sg13_lv_nmos").expect("sg13_lv_nmos model");
    assert_eq!(
        (def.library.as_str(), def.class.as_str()),
        ("cornerMOSlv.lib", "mos_lv")
    );
    assert_eq!(def.pins, ["d", "g", "s", "b"]);
    assert_eq!(defs.definition("rsil").unwrap().library, "cornerRES.lib");
    // Only what a netlist with an LV NMOS, a resistor and a bond pad needs.
    let (libs, unknown) = catalog
        .models
        .libs_for(m, "ngspice", "ss", &["sg13_lv_nmos", "rsil", "bondpad"])
        .unwrap();
    assert!(unknown.is_empty(), "{unknown:?}");
    let texts: Vec<String> = libs
        .iter()
        .map(|l| l.line(var_for(&pdk)).unwrap())
        .collect();
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert!(texts[0].ends_with("cornerMOSlv.lib mos_ss"));
    assert!(texts[1].ends_with("cornerRES.lib res_wcs"));
    assert!(texts[2].starts_with(".include ") && texts[2].ends_with("sg13g2_bondpad.lib"));

    // devices: the shipped map decides the PCells.
    let d = &tech.devices;
    assert_eq!(d.pcells, "map");
    assert!(d.count >= 30, "{d:?}");
    let device = |name: &str| {
        catalog
            .devices
            .devices
            .iter()
            .find(|x| x.name == name)
            .unwrap_or_else(|| panic!("no device {name}"))
    };
    let nmos = device("sg13_lv_nmos");
    assert_eq!(nmos.class, "mos_lv");
    assert_eq!(nmos.symbol, "sg13g2_pr/sg13_lv_nmos.sym");
    assert_eq!(nmos.models["ngspice"], "cornerMOSlv.lib");
    assert!(nmos.simulators.contains(&"xyce".to_string()));
    let pcell = nmos.pcell.as_ref().expect("pcell of sg13_lv_nmos");
    assert_eq!(
        (pcell.library.as_str(), pcell.cell.as_str(), pcell.guess),
        ("SG13_dev", "nmos", false)
    );
    assert_eq!(device("sg13_hv_pmos").class, "mos_hv");
    assert_eq!(device("rppd").class, "res");
    assert_eq!(device("npn13G2").class, "bjt");
    assert_eq!(device("cap_cmim").class, "cap");
    assert_eq!(device("diodevdd_2kv").class, "esd");
    // The draft is a device map all the same.
    let mut parsed = DeviceMap::default();
    parsed
        .add_text(&devices::draft_map(&tech, &catalog))
        .unwrap();
    assert_eq!(parsed.devices.len(), d.with_pcell);
}

#[test]
fn sky130a() {
    let Some((pdk, _env)) = pdk("sky130A") else {
        return;
    };
    let (tech, catalog) = scan(&pdk, helper());

    let x = &tech.xschem;
    let library = |name: &str| x.libraries.iter().find(|l| l.name == name);
    let pr = library("sky130_fd_pr").expect("sky130_fd_pr");
    // Through ciel's link all the same.
    assert_eq!(
        pr.path.as_str(),
        "$PDK_ROOT/sky130A/libs.tech/xschem/sky130_fd_pr"
    );
    assert_eq!(pr.role, "primitives");
    assert!(library("sky130_stdcells").is_some_and(|l| l.role == "stdcells" && l.enabled));
    assert!(library("sky130_tests").is_some_and(|l| !l.enabled));
    assert_eq!(
        x.variables["SKYWATER_MODELS"],
        "$PDK_ROOT/sky130A/libs.tech/combined"
    );
    assert_eq!(x.model_symbols, ["sky130_fd_pr/corner.sym"]);

    assert!(
        x.tcl_files.iter().all(|f| f.as_str().starts_with('$')),
        "{:?}",
        x.tcl_files
    );
    // Read as text, the start-up file gives the same libraries and variables.
    if x.evaluated {
        let (text, _) = scan(&pdk, false);
        assert!(!text.xschem.evaluated);
        assert_eq!(text.xschem.libraries, x.libraries);
        assert_eq!(text.xschem.variables, x.variables);
        assert_eq!(text.xschem.library_path, x.library_path);
        assert!(text.notes.iter().any(|n| n.contains("read as text")));
    }

    let m = &tech.models;
    // The xschemrc points the symbols at the combined models.
    assert_eq!(m.default.as_deref(), Some("combined"));
    assert_eq!(m.set("ngspice").unwrap().0, "combined");
    let set = &m.sets["combined"];
    assert_eq!(set.libraries, ["sky130.lib.spice"]);
    for corner in ["tt", "ff", "ss", "sf", "fs", "mc", "tt_mm", "ll", "hh"] {
        assert_eq!(
            set.corners.get(corner),
            Some(&vec![format!("sky130.lib.spice {corner}")]),
            "{:?}",
            set.corner_names()
        );
    }
    assert!(
        set.options.iter().any(|o| o.contains("ngbehavior=hsa")),
        "{:?}",
        set.options
    );
    check_corners(&pdk, &tech);
    let lines = m.lines("ngspice", "ff", var_for(&pdk)).unwrap();
    assert_eq!(
        lines,
        [format!(
            ".lib {}/sky130A/libs.tech/combined/sky130.lib.spice ff",
            pdk.root.display()
        )]
    );
    // A section that is no corner set.
    let libs = catalog.models.corner(m, "ngspice", "sf_ll_mm").unwrap();
    assert_eq!(libs[0].section, "sf_ll_mm");
    assert!(catalog.models.corner(m, "ngspice", "nonsense").is_none());

    let defs = &catalog.models.sets["combined"];
    let def = defs
        .definition("sky130_fd_pr__nfet_01v8")
        .expect("nfet_01v8 model");
    assert_eq!(def.library, "sky130.lib.spice");

    let nfet = catalog
        .devices
        .devices
        .iter()
        .find(|d| d.name == "nfet_01v8")
        .expect("nfet_01v8");
    assert_eq!(nfet.model, "sky130_fd_pr__nfet_01v8");
    assert_eq!(nfet.class, "mos");
    assert_eq!(nfet.symbol, "sky130_fd_pr/nfet_01v8.sym");
    assert_eq!(nfet.models["combined"], "sky130.lib.spice");
    assert!(
        nfet.parameters
            .iter()
            .any(|p| p.name == "W" && p.unit == "um")
    );
    assert!(!catalog.devices.devices.iter().any(|d| d.name == "corner"));
    // A three-terminal symbol names its bulk before the model.
    let nfet3 = catalog
        .devices
        .devices
        .iter()
        .find(|d| d.name == "nfet3_01v8")
        .unwrap();
    assert_eq!(nfet3.model, "sky130_fd_pr__nfet_01v8");
    assert_eq!(nfet3.model_pins.len(), 4);
    assert!(tech.devices.count >= 40, "{:?}", tech.devices);
    let mut parsed = DeviceMap::default();
    parsed
        .add_text(&devices::draft_map(&tech, &catalog))
        .unwrap();
}

/// The whole scan, every section, round-trips through the two files.
#[test]
fn whole_scan_round_trips() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        return;
    };
    // Files only: the helpers are exercised above and by the other sections' tests.
    let (tech, catalog) = tech::scan(&ScanContext::new(pdk).files_only());
    let text = toml::to_string_pretty(&tech).unwrap();
    let back: Tech = toml::from_str(&text).unwrap();
    assert_eq!(back.xschem, tech.xschem);
    assert_eq!(back.models, tech.models);
    assert_eq!(back.devices, tech.devices);
    let json = serde_json::to_string(&catalog).unwrap();
    let back: Catalog = serde_json::from_str(&json).unwrap();
    assert_eq!(back.xschem, catalog.xschem);
    assert_eq!(back.models, catalog.models);
    assert_eq!(back.devices, catalog.devices);
}
