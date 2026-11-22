//! Finding the PDKs installed on this machine (`libcdept::tech::discover`, `ciel`).
//!
//! These look at the real PDK folder of the development machine and are skipped
//! where it is not there. The unit tests in the modules cover the rules with
//! made-up folders.

use std::path::Path;

use libcdept::tech::discover::{self, Origin, Search};
use libcdept::tech::{Family, ciel};

const PDKS: &str = "/opt/ext/OpenPDKs";

fn no_env(_: &str) -> Option<String> {
    None
}

/// The search of a machine with nothing set but one folder named.
fn search() -> Option<Search> {
    let root = Path::new(PDKS);
    if !root.join("ihp-sg13g2/libs.tech").is_dir() {
        eprintln!("skipped: no PDKs in {PDKS}");
        return None;
    }
    Some(Search::with_env(no_env).only().given([root.to_path_buf()]))
}

#[test]
fn the_pdks_of_this_machine_are_found_with_their_versions() {
    let Some(search) = search() else { return };
    let found = discover::discover(&search);
    let names: Vec<&str> = found.iter().map(|f| f.pdk.name.as_str()).collect();
    assert!(names.contains(&"ihp-sg13g2"), "{names:?}");

    let ihp = found.iter().find(|f| f.pdk.name == "ihp-sg13g2").unwrap();
    assert_eq!(
        ihp.pdk.root,
        Path::new(PDKS),
        "reached through the root first"
    );
    assert_eq!(ihp.pdk.family, Family::IhpSg13g2);
    assert_eq!(ihp.origin, Origin::Given);
    let info = ihp.info();
    assert!(info.commit.is_some() || info.version.is_some(), "{info:?}");
    for tool in ["klayout", "xschem", "ngspice"] {
        assert!(
            info.tools.iter().any(|t| t == tool),
            "{tool}: {:?}",
            info.tools
        );
    }

    // sky130 is a ciel install here: its version is the folder it is in.
    if let Some(sky) = found.iter().find(|f| f.pdk.name == "sky130A") {
        let info = sky.info();
        assert_eq!(info.source.as_deref(), Some("ciel"), "{info:?}");
        let commit = info.commit.expect("open_pdks commit");
        assert_eq!(commit.len(), 40, "{commit}");
        assert!(
            info.version.is_some(),
            "open_pdks version from nodeinfo.json"
        );
        assert!(info.description.is_some());
        assert!(names.contains(&"sky130B"), "{names:?}");

        let versions = ciel::installed(Path::new(PDKS), Some("sky130"));
        assert!(versions.iter().any(|v| v.version == commit && v.current));
        assert_eq!(ciel::current(Path::new(PDKS), "sky130"), Some(commit));
    }
}

#[test]
fn only_pdks_are_listed_and_each_once() {
    let Some(search) = search() else { return };
    let found = discover::discover(&search);
    // Every entry is a PDK folder; a folder that is not one (a vendor's own tree,
    // ciel's version store) gives nothing, not even through a well-known name.
    for f in &found {
        assert!(discover::is_pdk(&f.pdk.dir()), "{:?}", f.pdk);
        let real = f.real.to_string_lossy();
        assert!(!real.contains("/XFAB"), "{real}");
    }
    assert!(
        found
            .iter()
            .all(|f| f.pdk.root != Path::new(PDKS).join("ciel"))
    );
    let mut real: Vec<_> = found.iter().map(|f| f.real.clone()).collect();
    real.sort();
    real.dedup();
    assert_eq!(
        real.len(),
        found.len(),
        "a PDK reached twice is listed once"
    );

    // By name, and by the path of the PDK folder.
    let by_name = discover::find("ihp-sg13g2", &search).unwrap();
    assert_eq!(by_name.root, Path::new(PDKS));
    let by_path = discover::find(&format!("{PDKS}/ihp-sg13g2"), &search).unwrap();
    assert_eq!(by_path, by_name);
    assert!(discover::find("no-such-pdk", &search).is_none());
}

#[test]
fn a_checkout_one_level_down_is_found() {
    let Some(search) = search() else { return };
    let checkout = Path::new(PDKS).join("IHP-Open-PDK");
    if !checkout.join("ihp-sg13cmos5l/libs.tech").is_dir() {
        eprintln!("skipped: no IHP-Open-PDK checkout");
        return;
    }
    let found = discover::discover(&search);
    let cmos = found
        .iter()
        .find(|f| f.pdk.name == "ihp-sg13cmos5l")
        .expect("found inside the checkout");
    assert_eq!(cmos.pdk.root, checkout);
    assert_eq!(cmos.pdk.family, Family::IhpSg13cmos5l);
    let info = cmos.info();
    assert_eq!(info.source.as_deref(), Some("git"));
    assert!(info.branch.is_some() || info.commit.is_some(), "{info:?}");
    assert!(
        info.tested_with.contains_key("xschem"),
        "versions.txt is read"
    );
}
