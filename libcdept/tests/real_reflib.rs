//! Real-PDK tests: reference libraries on the PDKs of this machine (skipped when they are
//! absent, see `libcdept::testutil`): what the container readers find, how long it takes,
//! the `ref` library layout.

use std::path::{Path, PathBuf};
use std::time::Instant;

use libcdept::container::{self, Cache, Index};
use libcdept::testutil::{RealPdk, real_pdk};

/// The folder of the installed PDK, and the guard that keeps the environment for it.
fn pdk(name: &str) -> Option<(PathBuf, RealPdk)> {
    let real = real_pdk(name)?;
    real.dir.join("libs.ref").is_dir().then(|| (real.dir.clone(), real))
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

#[test]
fn readers_on_sky130_fd_sc_hd() {
    let Some((pdk, _env)) = pdk("sky130A") else {
        eprintln!("skipped: no sky130A");
        return;
    };
    let lib = pdk.join("libs.ref/sky130_fd_sc_hd");
    let tmp = tempfile::tempdir().unwrap();
    let cache = Cache::at(tmp.path());

    let read = |file: PathBuf| {
        let start = Instant::now();
        let index = cache.get(&file).unwrap();
        let cold = start.elapsed();
        let start = Instant::now();
        let again = cache.get(&file).unwrap();
        let warm = start.elapsed();
        assert_eq!(index, again);
        eprintln!(
            "{:>9.1} ms cold {:>7.2} ms warm  {} ({} cells)",
            cold.as_secs_f64() * 1e3,
            warm.as_secs_f64() * 1e3,
            file.file_name().unwrap().to_string_lossy(),
            index.names().len()
        );
        index
    };

    let gds = read(lib.join("gds/sky130_fd_sc_hd.gds"));
    assert!(gds.defines("sky130_fd_sc_hd__inv_1"));
    assert!(gds.public().contains(&"sky130_fd_sc_hd__inv_1"));

    let Index::Lef(lef) = read(lib.join("lef/sky130_fd_sc_hd.lef")) else {
        panic!("not LEF");
    };
    let inv = lef
        .macros
        .iter()
        .find(|m| m.name == "sky130_fd_sc_hd__inv_1")
        .unwrap();
    assert_eq!(inv.class, "CORE");
    assert_eq!(inv.size, Some((1.38, 2.72)));
    assert_eq!(inv.site, "unithd");
    let pins: Vec<_> = inv.pins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(pins, ["A", "VGND", "VNB", "VPB", "VPWR", "Y"]);

    let Index::Lef(tlef) = read(lib.join("techlef/sky130_fd_sc_hd__nom.tlef")) else {
        panic!("not LEF");
    };
    assert!(tlef.is_technology());
    assert!(
        tlef.layers
            .iter()
            .any(|l| l.name == "met1" && l.kind == "ROUTING")
    );
    assert!(tlef.sites.iter().any(|s| s.name == "unithd"));

    let Index::Liberty(lib_tt) = read(lib.join("lib/sky130_fd_sc_hd__tt_025C_1v80.lib")) else {
        panic!("not Liberty");
    };
    assert_eq!(lib_tt.library, "sky130_fd_sc_hd__tt_025C_1v80");
    let conditions = lib_tt.conditions();
    assert_eq!(conditions.voltage, Some(1.8));
    assert_eq!(conditions.temperature, Some(25.0));
    let inv = lib_tt
        .cells
        .iter()
        .find(|c| c.name == "sky130_fd_sc_hd__inv_1")
        .unwrap();
    assert_eq!(inv.area, Some(3.7536));
    assert_eq!(inv.footprint, "sky130_fd_sc_hd__inv");
    let pin = |name: &str| inv.pins.iter().find(|p| p.name == name).unwrap();
    assert_eq!(pin("A").direction, "input");
    assert_eq!(pin("Y").direction, "output");
    assert_eq!(pin("VPWR").direction, "primary_power");
    let dff = lib_tt
        .cells
        .iter()
        .find(|c| c.name == "sky130_fd_sc_hd__dfxtp_1")
        .unwrap();
    assert_eq!(dff.sequential, "ff");
    let latch = lib_tt
        .cells
        .iter()
        .find(|c| c.name == "sky130_fd_sc_hd__dlxtp_1")
        .unwrap();
    assert_eq!(latch.sequential, "latch");

    let Index::Verilog(verilog) = read(lib.join("verilog/sky130_fd_sc_hd.v")) else {
        panic!("not Verilog");
    };
    let inv = verilog
        .modules
        .iter()
        .find(|m| m.name == "sky130_fd_sc_hd__inv_1")
        .unwrap();
    let ports: Vec<_> = inv
        .ports
        .iter()
        .map(|p| (p.name.as_str(), p.direction.as_str()))
        .collect();
    assert!(ports.contains(&("Y", "output")), "{ports:?}");
    assert!(ports.contains(&("A", "input")), "{ports:?}");

    let Index::Spice(spice) = read(lib.join("spice/sky130_fd_sc_hd.spice")) else {
        panic!("not a netlist");
    };
    let inv = spice
        .subckts
        .iter()
        .find(|s| s.name == "sky130_fd_sc_hd__inv_1")
        .unwrap();
    assert_eq!(inv.pins, ["A", "VGND", "VNB", "VPB", "VPWR", "Y"]);
    assert_eq!(inv.devices, 2);

    // The same cells in every format (the Liberty file has no fill and tap cells).
    let cdl = read(lib.join("cdl/sky130_fd_sc_hd.cdl"));
    assert_eq!(cdl.names().len(), spice.subckts.len());
    assert!(lef.macros.len() >= lib_tt.cells.len());

    // All Liberty corners side by side, cold, then warm.
    let libs = files(&lib.join("lib"));
    let paths: Vec<(&Path, container::Reader)> = libs
        .iter()
        .map(|f| (f.as_path(), container::Reader::Liberty))
        .collect();
    let fresh = Cache::at(tmp.path().join("all"));
    let start = Instant::now();
    let cold = container::read_many(&paths, &fresh);
    let cold_time = start.elapsed();
    let start = Instant::now();
    let warm = container::read_many(&paths, &fresh);
    let warm_time = start.elapsed();
    let bytes: u64 = libs.iter().map(|f| f.metadata().unwrap().len()).sum();
    eprintln!(
        "{} Liberty files, {} MB: {:.0} ms cold, {:.1} ms warm",
        libs.len(),
        bytes >> 20,
        cold_time.as_secs_f64() * 1e3,
        warm_time.as_secs_f64() * 1e3
    );
    for (a, b) in cold.iter().zip(&warm) {
        let (a, b) = (a.as_ref().unwrap(), b.as_ref().unwrap());
        assert_eq!(a, b);
        assert!(a.names().len() > 300, "{}", a.names().len());
    }
}

#[test]
fn readers_on_ihp_sg13g2() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        eprintln!("skipped: no ihp-sg13g2");
        return;
    };
    let lib = pdk.join("libs.ref/sg13g2_stdcell");
    let cache = Cache::off();
    let Index::Liberty(typ) = cache
        .get(&lib.join("lib/sg13g2_stdcell_typ_1p20V_25C.lib"))
        .unwrap()
    else {
        panic!("not Liberty");
    };
    assert_eq!(typ.library, "sg13g2_stdcell_typ_1p20V_25C");
    assert_eq!(typ.nom_voltage, Some(1.2));
    assert_eq!(typ.nom_temperature, Some(25.0));
    assert_eq!(typ.time_unit, "1ns");
    assert_eq!(typ.capacitive_load_unit, "1pf");
    let inv = typ.cells.iter().find(|c| c.name == "sg13g2_inv_1").unwrap();
    assert!(inv.area.is_some());
    assert!(
        inv.pins
            .iter()
            .any(|p| p.name == "A" && p.direction == "input")
    );
    assert!(
        inv.pins
            .iter()
            .any(|p| p.name == "Y" && p.direction == "output")
    );

    let Index::Lef(tech) = cache.get(&lib.join("lef/sg13g2_tech.lef")).unwrap() else {
        panic!("not LEF");
    };
    assert!(tech.is_technology());
    assert_eq!(tech.database_microns, Some(1000.0));
    assert_eq!(tech.manufacturing_grid, Some(0.005));
    assert!(
        tech.layers
            .iter()
            .any(|l| l.name == "Metal1" && l.kind == "ROUTING")
    );

    let Index::Lef(cells) = cache.get(&lib.join("lef/sg13g2_stdcell.lef")).unwrap() else {
        panic!("not LEF");
    };
    // The site is defined with the cells here, not in the technology LEF.
    assert!(!cells.is_technology());
    assert_eq!(cells.sites[0].name, "CoreSite");
    assert_eq!(cells.sites[0].size, Some((0.48, 3.78)));
    assert!(cells.macros.iter().any(|m| m.name == "sg13g2_inv_1"));
    let gds = cache.get(&lib.join("gds/sg13g2_stdcell.gds")).unwrap();
    assert!(gds.public().contains(&"sg13g2_inv_1"));
    let verilog = cache.get(&lib.join("verilog/sg13g2_stdcell.v")).unwrap();
    assert!(verilog.defines("sg13g2_inv_1"));
    let cdl = cache.get(&lib.join("cdl/sg13g2_stdcell.cdl")).unwrap();
    assert!(cdl.defines("sg13g2_inv_1"));
    eprintln!(
        "sg13g2_stdcell: {} lib cells, {} macros, {} gds tops, {} modules, {} subckts",
        typ.cells.len(),
        cells.macros.len(),
        gds.public().len(),
        verilog.names().len(),
        cdl.names().len()
    );
}

fn library(path: &Path) -> libcdept::Library {
    libcdept::Library {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        layout: libcdept::LibraryLayout::detect(path),
        read_only: true,
        description: None,
        defined_in: PathBuf::new(),
        error: None,
        cells: Vec::new(),
        preferred: Default::default(),
    }
}

/// Scan every library of `libs.ref` with a cold, then a warm cache; print the counts
/// and the times.
fn scan_all(pdk: &Path, cache: &Cache) -> Vec<libcdept::Library> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(pdk.join("libs.ref"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut libraries = Vec::new();
    for dir in dirs {
        let mut lib = library(&dir);
        assert_eq!(
            lib.layout,
            libcdept::LibraryLayout::Ref,
            "{}",
            dir.display()
        );
        let start = Instant::now();
        lib.scan_with(cache);
        let cold = start.elapsed();
        let cells = lib.cells.clone();
        let start = Instant::now();
        lib.scan_with(cache);
        let warm = start.elapsed();
        assert_eq!(lib.error, None);
        assert_eq!(lib.cells, cells, "a warm scan gives the same cells");
        let views: usize = lib.cells.iter().map(|c| c.entries.len()).sum();
        eprintln!(
            "{:>8.0} ms cold {:>7.1} ms warm  {:<20} {:>5} cells {:>6} views",
            cold.as_secs_f64() * 1e3,
            warm.as_secs_f64() * 1e3,
            lib.name,
            lib.cells.len(),
            views
        );
        libraries.push(lib);
    }
    libraries
}

fn views(lib: &libcdept::Library, cell: &str) -> Vec<String> {
    lib.cell(cell)
        .unwrap_or_else(|| panic!("no cell {cell} in {}", lib.name))
        .entries
        .iter()
        .map(|e| e.name.clone())
        .collect()
}

#[test]
fn ref_layout_on_sky130() {
    let Some((pdk, _env)) = pdk("sky130A") else {
        eprintln!("skipped: no sky130A");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let libraries = scan_all(&pdk, &Cache::at(tmp.path()));
    let lib = |name: &str| libraries.iter().find(|l| l.name == name).unwrap();

    let hd = lib("sky130_fd_sc_hd");
    assert_eq!(
        views(hd, "sky130_fd_sc_hd__inv_1"),
        [
            "abstract",
            "abstract_mag",
            "hdl",
            "layout",
            "layout_mag",
            "liberty",
            "netlist",
            "netlist_cdl"
        ]
    );
    let inv = hd.cell("sky130_fd_sc_hd__inv_1").unwrap();
    let layout = inv.entry("layout").unwrap();
    assert_eq!(layout.member.as_deref(), Some("sky130_fd_sc_hd__inv_1"));
    assert!(
        layout
            .main_file
            .as_ref()
            .unwrap()
            .ends_with("gds/sky130_fd_sc_hd.gds")
    );
    let liberty = inv.entry("liberty").unwrap();
    assert_eq!(liberty.files.len(), 18);
    assert!(
        liberty
            .main_file
            .as_ref()
            .unwrap()
            .ends_with("lib/sky130_fd_sc_hd__tt_025C_1v80.lib")
    );
    let hdl = inv.entry("hdl").unwrap();
    assert!(
        hdl.main_file
            .as_ref()
            .unwrap()
            .ends_with("verilog/sky130_fd_sc_hd.v")
    );
    // A `.mag` file is the view itself.
    assert_eq!(inv.entry("layout_mag").unwrap().member, None);
    // No cell for the technology LEFs, the primitives file or the collection files.
    assert!(hd.cell("sky130_fd_sc_hd").is_none());
    assert!(hd.cell("primitives").is_none());
    assert!(hd.cell("sky130_fd_sc_hd__nom").is_none());
    assert!(
        hd.cells.len() > 400 && hd.cells.len() < 520,
        "{}",
        hd.cells.len()
    );

    // A macro library: one cell per file, the sub-cells of the GDS are not cells
    // (unless the library has them as `.mag` files, which it does for OpenRAM's).
    let sram = lib("sky130_sram_macros");
    let ram = sram.cell("sky130_sram_1kbyte_1rw1r_32x256_8").unwrap();
    assert_eq!(ram.entry("layout").unwrap().member, None);
    assert!(ram.entry("liberty").is_some());
    assert!(ram.entry("abstract").is_some());

    // IO: a Liberty file per cell and corner; the cell is the one inside.
    let io = lib("sky130_fd_io");
    let pad = io.cell("sky130_ef_io__gpiov2_pad_wrapped").unwrap();
    let liberty = pad.entry("liberty").unwrap();
    assert!(liberty.files.len() > 1, "{:?}", liberty.files);
    assert_eq!(
        liberty.member.as_deref(),
        Some("sky130_ef_io__gpiov2_pad_wrapped")
    );

    // Primitives: devices with fixed layouts.
    let pr = lib("sky130_fd_pr");
    assert!(
        pr.cell("sky130_fd_pr__rf_nfet_01v8_aM02W1p65L0p15")
            .is_some()
    );
}

#[test]
fn ref_layout_on_ihp_sg13g2() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        eprintln!("skipped: no ihp-sg13g2");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let libraries = scan_all(&pdk, &Cache::at(tmp.path()));
    let lib = |name: &str| libraries.iter().find(|l| l.name == name).unwrap();

    let sc = lib("sg13g2_stdcell");
    assert_eq!(
        views(sc, "sg13g2_inv_1"),
        [
            "abstract",
            "hdl",
            "layout",
            "liberty",
            "netlist",
            "netlist_cdl",
            "schematic",
            "schematic_qucs-s",
            "symbol"
        ]
    );
    let inv = sc.cell("sg13g2_inv_1").unwrap();
    assert_eq!(inv.entry("symbol").unwrap().member, None);
    assert!(
        inv.entry("symbol")
            .unwrap()
            .main_file
            .as_ref()
            .unwrap()
            .ends_with("sym/xschem/sg13g2_inv_1.sym")
    );
    assert!(
        inv.entry("liberty")
            .unwrap()
            .main_file
            .as_ref()
            .unwrap()
            .ends_with("lib/sg13g2_stdcell_typ_1p20V_25C.lib")
    );
    // Qucs-S symbols have other names and are not xschem symbols: not cells.
    assert!(sc.cell("A21O").is_none());
    assert!(sc.cell("sg13g2_tech").is_none());
    assert!(
        sc.cells.len() >= 84 && sc.cells.len() < 100,
        "{}",
        sc.cells.len()
    );

    let sram = lib("sg13g2_sram");
    let ram = sram.cell("RM_IHPSG13_1P_1024x16_c2_bm_bist").unwrap();
    assert_eq!(ram.entry("layout").unwrap().member, None);
    assert_eq!(ram.entry("liberty").unwrap().files.len(), 3);
    // 30 macros and the 4 behavioural cores of their Verilog models.
    assert_eq!(sram.cells.len(), 34);
    assert!(ram.entry("hdl").is_some());

    let io = lib("sg13g2_io");
    assert!(io.cell("sg13g2_IOPadIn").is_some());
    let pr = lib("sg13g2_pr");
    assert!(!pr.cells.is_empty());
}

/// The `[refs]` section of the technology snapshot for a PDK, with timings.
fn refs_of(
    pdk: &Path,
    cache: &Cache,
) -> (
    libcdept::tech::refs::RefsTech,
    libcdept::tech::refs::RefsCatalog,
) {
    use libcdept::tech::{PdkDir, ScanContext, refs};
    let dir = PdkDir::new(
        pdk.parent().unwrap(),
        pdk.file_name().unwrap().to_string_lossy(),
    );
    let ctx = ScanContext::new(dir).files_only();
    let start = Instant::now();
    let cold = refs::scan_with(&ctx, cache);
    let cold_time = start.elapsed();
    let start = Instant::now();
    let warm = refs::scan_with(&ctx, cache);
    let warm_time = start.elapsed();
    assert_eq!(cold, warm);
    let toml = toml::to_string(&cold.section).unwrap();
    let json = serde_json::to_string(&cold.catalog).unwrap();
    eprintln!(
        "refs of {}: {:.0} ms cold, {:.0} ms warm; tech.toml part {} lines, catalog part {} KiB; notes {:?}",
        pdk.display(),
        cold_time.as_secs_f64() * 1e3,
        warm_time.as_secs_f64() * 1e3,
        toml.lines().count(),
        json.len() / 1024,
        cold.notes
    );
    if std::env::var_os("CDEPT_TEST_PRINT").is_some() {
        eprintln!("{toml}");
    }
    assert_eq!(
        toml::from_str::<refs::RefsTech>(&toml).unwrap(),
        cold.section
    );
    (cold.section, cold.catalog)
}

#[test]
fn refs_section_on_sky130() {
    let Some((pdk, _env)) = pdk("sky130A") else {
        eprintln!("skipped: no sky130A");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let (refs, catalog) = refs_of(&pdk, &Cache::at(tmp.path()));
    let hd = refs.library("sky130_fd_sc_hd").unwrap();
    assert_eq!(
        hd.path.as_str(),
        "$PDK_ROOT/sky130A/libs.ref/sky130_fd_sc_hd"
    );
    assert_eq!(
        (hd.layout.as_str(), hd.role.as_str(), hd.enabled),
        ("ref", "stdcells", true)
    );
    assert_eq!(hd.sites["unithd"], (0.46, 2.72));
    assert_eq!(hd.techlef.len(), 6);
    assert_eq!(hd.liberty.len(), 18);
    assert_eq!(hd.default_corner.as_deref(), Some("tt_025C_1v80"));
    assert!(
        hd.liberty["tt_025C_1v80"]
            .as_str()
            .ends_with("lib/sky130_fd_sc_hd__tt_025C_1v80.lib")
    );

    let roles: Vec<_> = refs
        .libraries
        .iter()
        .map(|l| (l.name.as_str(), l.role.as_str(), l.enabled))
        .collect();
    assert_eq!(
        roles,
        [
            ("sky130_fd_io", "io", true),
            ("sky130_fd_pr", "primitives", true),
            ("sky130_fd_sc_hd", "stdcells", true),
            ("sky130_fd_sc_hvl", "stdcells", true),
            ("sky130_ml_xx_hd", "other", false),
            ("sky130_sram_macros", "memory", true),
        ]
    );
    // IO: many files per corner (one per cell): a pattern.
    let io = refs.library("sky130_fd_io").unwrap();
    assert!(
        io.liberty
            .values()
            .any(|path| path.as_str().contains("/lib/*"))
    );
    eprintln!(
        "sky130_fd_io: {} corners, e.g. {:?}",
        io.liberty.len(),
        io.liberty.iter().take(3).collect::<Vec<_>>()
    );
    assert!(
        io.liberty.keys().all(|name| !name.starts_with("sky130")),
        "{:?}",
        io.liberty.keys()
    );

    let digital = refs.digital.as_ref().unwrap();
    assert_eq!(digital.flow, "openlane");
    assert_eq!(digital.default_library.as_deref(), Some("sky130_fd_sc_hd"));
    assert!(digital.libraries.contains_key("sky130_fd_sc_hd"));
    assert!(!digital.rcx_rules.is_empty());
    assert_eq!(catalog.digital["sky130_fd_sc_hd"]["PLACE_SITE"], "unithd");

    let hd = catalog
        .libraries
        .iter()
        .find(|l| l.name == "sky130_fd_sc_hd")
        .unwrap();
    let tt = hd.corner("tt_025C_1v80").unwrap();
    assert_eq!(
        (tt.process.as_deref(), tt.voltage, tt.temperature, tt.files),
        (Some("tt"), Some(1.8), Some(25.0), 1)
    );
    let ss = hd.corner("ss_n40C_1v28").unwrap();
    assert_eq!((ss.voltage, ss.temperature), (Some(1.28), Some(-40.0)));
    let cell = |name: &str| hd.cells.iter().find(|c| c.name == name).unwrap();
    let inv = cell("sky130_fd_sc_hd__inv_1");
    assert_eq!(inv.class, "CORE");
    assert_eq!(inv.size, Some((1.38, 2.72)));
    assert_eq!(inv.area, Some(3.7536));
    assert_eq!(cell("sky130_fd_sc_hd__dfxtp_1").sequential, "ff");
    let tlef = hd
        .techlef
        .iter()
        .find(|t| t.file.as_str().ends_with("__nom.tlef"))
        .unwrap();
    assert!(tlef.layers.iter().any(|l| l.name == "met1"));
}

#[test]
fn refs_section_on_ihp_sg13g2() {
    let Some((pdk, _env)) = pdk("ihp-sg13g2") else {
        eprintln!("skipped: no ihp-sg13g2");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let (refs, catalog) = refs_of(&pdk, &Cache::at(tmp.path()));
    let roles: Vec<_> = refs
        .libraries
        .iter()
        .map(|l| (l.name.as_str(), l.role.as_str(), l.enabled))
        .collect();
    assert_eq!(
        roles,
        [
            ("sg13g2_io", "io", true),
            ("sg13g2_pr", "primitives", true),
            ("sg13g2_sram", "memory", true),
            ("sg13g2_stdcell", "stdcells", true),
        ]
    );
    let sc = refs.library("sg13g2_stdcell").unwrap();
    assert_eq!(sc.cells, 84);
    assert_eq!(sc.default_corner.as_deref(), Some("typ_1p20V_25C"));
    assert_eq!(sc.liberty.len(), 6);
    assert!(
        sc.liberty["typ_1p20V_25C"]
            .as_str()
            .ends_with("_typ_1p20V_25C.lib")
    );
    assert_eq!(sc.sites["CoreSite"], (0.48, 3.78));
    assert!(sc.techlef[0].as_str().ends_with("lef/sg13g2_tech.lef"));
    let library = |name: &str| catalog.libraries.iter().find(|l| l.name == name).unwrap();
    let typ = library("sg13g2_stdcell").corner("typ_1p20V_25C").unwrap();
    assert_eq!(
        (typ.process.as_deref(), typ.voltage, typ.temperature),
        (Some("typ"), Some(1.2), Some(25.0))
    );
    // Memories: a file per macro and corner, three corners.
    let sram = refs.library("sg13g2_sram").unwrap();
    assert_eq!(sram.liberty.len(), 3, "{:?}", sram.liberty);
    assert_eq!(
        sram.liberty["typ_1p20V_25C"].as_str(),
        "$PDK_ROOT/ihp-sg13g2/libs.ref/sg13g2_sram/lib/*_typ_1p20V_25C.lib"
    );
    assert!(library("sg13g2_sram").corners.iter().all(|c| c.files == 30));
    // IO: two supplies.
    let io = library("sg13g2_io").corner("typ_1p2V_3p3V_25C").unwrap();
    assert_eq!(
        (io.voltage, io.voltages.as_slice()),
        (Some(1.2), &[1.2, 3.3][..])
    );

    let digital = refs.digital.as_ref().unwrap();
    assert_eq!(digital.flow, "librelane");
    assert_eq!(digital.default_library.as_deref(), Some("sg13g2_stdcell"));
    assert_eq!(digital.pad_library.as_deref(), Some("sg13g2_io"));
    assert_eq!(digital.default_corner.as_deref(), Some("nom_typ_1p20V_25C"));
    assert_eq!(digital.corners.len(), 3);
    assert!(catalog.digital[""].contains_key("VDD_PIN"));
}
