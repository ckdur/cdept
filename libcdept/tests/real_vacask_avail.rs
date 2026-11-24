//! Real-PDK test: which installed technologies VACASK can run (docs/VACASK.md section 2).
//! sky130A models its transistors as binned BSIM4, which VACASK's converter cannot
//! convert, so it is unavailable whatever tools exist; ihp-sg13g2 is available when
//! VACASK, OpenVAF and python3 are. Skipped when the PDK is not installed
//! (`$CDEPT_TEST_PDK_ROOT`, else /opt/ext/OpenPDKs).

use libcdept::tech::{self, PdkDir, ScanContext};
use libcdept::testutil::real_pdk;
use libcdept::vacask::{Availability, availability};
use libcdept::{Project, batch};

/// A new project whose snapshot is a scan of the installed PDK `name`.
fn project_for(
    name: &str,
    dir: &std::path::Path,
) -> Option<(libcdept::testutil::RealPdk, Project)> {
    let real = real_pdk(name)?;
    let pdk = PdkDir::new(real.root.clone(), name);
    let (tech, catalog) = tech::scan(&ScanContext::new(pdk.clone()).files_only());
    let project = Project::create_for_pdk(dir, None, &pdk, &tech, &catalog).unwrap();
    Some((real, project))
}

#[test]
fn sky130_is_unavailable_and_the_reason_names_the_technology() {
    let tmp = tempfile::tempdir().unwrap();
    let Some((_pdk, project)) = project_for("sky130A", &tmp.path().join("p")) else {
        return;
    };
    let t = std::time::Instant::now();
    let a = availability(&project);
    let first = t.elapsed();
    eprintln!("sky130A: {a:?} ({first:.2?})");
    // Unavailable because of the models, or earlier because there is no VACASK here: the
    // models are the reason to look for when VACASK is installed.
    let why = a.reason().unwrap_or_else(|| panic!("sky130A: {a:?}"));
    if batch::vacask::ADAPTER.locate().is_some() {
        assert!(
            why.contains("sky130A") && why.contains("binned BSIM4"),
            "{why}"
        );
        assert!(a.line().starts_with("VACASK: not available: "), "{a:?}");
    }
    // Asking again is answered from memory, and the same.
    let t = std::time::Instant::now();
    assert_eq!(availability(&project), a);
    eprintln!("again: {:.2?}", t.elapsed());
    assert!(t.elapsed() < first.max(std::time::Duration::from_millis(50)));
}

#[test]
fn ihp_sg13g2_is_available_when_the_tools_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let Some((_pdk, project)) = project_for("ihp-sg13g2", &tmp.path().join("p")) else {
        return;
    };
    let tools = batch::vacask::ADAPTER.locate().is_some()
        && batch::openvaf::ADAPTER.locate().is_some()
        && libcdept::tools::find_in_path("python3").is_some();
    let t = std::time::Instant::now();
    let a = availability(&project);
    eprintln!("ihp-sg13g2: {a:?} ({:.2?})", t.elapsed());
    if tools {
        assert_eq!(a, Availability::Ready);
    } else {
        eprintln!("skipped: VACASK, OpenVAF or python3 not found: {a:?}");
        assert!(a.reason().is_some());
    }
}
