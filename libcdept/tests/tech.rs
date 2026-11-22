//! The technology snapshot end to end, on the PDKs of this machine: scan, write, load
//! the project with it. Each test skips when its PDK is not installed.
//!
//! PDKs are looked for in `$PDK_ROOT` and `/opt/ext/OpenPDKs`.

use std::fs;
use std::path::PathBuf;

use libcdept::tech::{self, PdkDir, ScanContext, file, report};
use libcdept::{PROJECT_FILE, Project};

fn pdk(name: &str) -> Option<PdkDir> {
    let roots = [
        std::env::var_os("PDK_ROOT").map(PathBuf::from),
        Some(PathBuf::from("/opt/ext/OpenPDKs")),
    ];
    let found = roots
        .into_iter()
        .flatten()
        .find(|root| root.join(name).join("libs.tech").is_dir())
        .map(|root| PdkDir::new(root, name));
    if found.is_none() {
        eprintln!("skipped: PDK {name} is not on this machine");
    }
    found
}

/// Files only: the helper programs are tested by the sections that use them.
fn scan(pdk: &PdkDir) -> (tech::Tech, tech::Catalog) {
    tech::scan(&ScanContext::new(pdk.clone()).files_only())
}

/// Everything the snapshot names below `$PDK_ROOT` as a setting of a tool is there.
fn check_paths(pdk: &PdkDir, tech: &tech::Tech) {
    let mut named = Vec::new();
    named.extend(tech.klayout.path.iter());
    named.extend(tech.klayout.lyt.iter());
    named.extend(tech.klayout.layer_properties.iter());
    named.extend(tech.klayout.drc.iter().map(|deck| &deck.file));
    named.extend(tech.klayout.lvs.iter().map(|deck| &deck.file));
    named.extend(tech.xschem.rc.iter());
    named.extend(tech.magic.rc.iter());
    named.extend(tech.magic.tech_file.iter());
    named.extend(tech.netgen.setup.iter());
    for library in tech.libraries() {
        let path = library.path.resolve_in(pdk).unwrap();
        assert!(
            path.is_dir(),
            "library {}: {}",
            library.name,
            path.display()
        );
        assert!(
            library.path.as_str().starts_with("$PDK_ROOT/"),
            "{}",
            library.path
        );
    }
    for path in named {
        assert!(
            path.as_str().starts_with("$PDK_ROOT/"),
            "{path} is not below $PDK_ROOT"
        );
        let on_disk = path.resolve_in(pdk).unwrap();
        assert!(on_disk.exists(), "{path} → {}", on_disk.display());
    }
}

fn snapshot_round_trip(name: &str) {
    let Some(pdk) = pdk(name) else {
        return;
    };
    let (tech, catalog) = scan(&pdk);
    assert_eq!(tech.schema, tech::TECH_SCHEMA);
    assert_eq!(tech.pdk.name, name);
    assert_eq!(catalog.pdk, name);
    check_paths(&pdk, &tech);

    // A second scan finds the same: nothing depends on the order files are met in.
    let (again, catalog_again) = scan(&pdk);
    let changes = report::changes((&tech, &catalog), (&again, &catalog_again));
    assert!(changes.is_empty(), "{changes:#?}");

    // Written and read back, both files say what the scan found.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("design");
    let project = Project::create_for_pdk(&root, Some("design"), &pdk, &tech, &catalog).unwrap();
    let text = fs::read_to_string(file::tech_file(&project.root)).unwrap();
    assert!(
        text.starts_with("# CDePT technology snapshot"),
        "{}",
        &text[..80]
    );
    assert!(
        !text.contains(pdk.root.to_str().unwrap()),
        "tech.toml names this machine's PDK folder"
    );
    assert_eq!(
        file::load_tech(&project.root).unwrap().as_ref(),
        Some(&tech)
    );
    assert_eq!(project.catalog().unwrap().as_ref(), Some(&catalog));
    assert_eq!(project.tech.as_ref(), Some(&tech));

    // The project: the PDK is the project's, here through the user's local file (the
    // test's environment may not have PDK_ROOT), and nothing is wrong with it.
    assert!(project.tech_in_use);
    assert_eq!(project.technology.as_deref(), Some(name));
    assert_eq!(project.pdk_dir().as_ref(), Some(&pdk));
    let about_snapshot: Vec<_> = project
        .warnings
        .iter()
        .filter(|w| w.contains("tech.toml") || w.contains("PDK_ROOT"))
        .collect();
    assert!(about_snapshot.is_empty(), "{about_snapshot:#?}");
    let project_text = fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    assert!(project_text.contains(&format!("technology = \"{name}\"")));
    assert!(!project_text.contains("\npdk_root"), "{project_text}");

    // What the snapshot names is what the tools get.
    if let Some(rc) = &tech.xschem.rc {
        assert_eq!(project.xschem_rc, [rc.resolve_in(&pdk).unwrap()]);
    }
    if let Some(name) = &tech.klayout.tech {
        assert_eq!(project.klayout.tech.as_ref(), Some(name));
    }
    if let Some(deck) = tech.klayout.drc.first() {
        assert_eq!(
            project.klayout.drc,
            Some(deck.file.resolve_in(&pdk).unwrap())
        );
    }
    if let Some(deck) = tech.klayout.lvs.first() {
        assert_eq!(
            project.klayout.lvs,
            Some(deck.file.resolve_in(&pdk).unwrap())
        );
    }
    assert_eq!(project.magic_tech(), tech.magic.tech);
    if let Some(dbu) = tech.klayout.dbu {
        assert_eq!(project.dbu(), dbu);
    }
    // The PDK's enabled libraries are in the project, read-only, each name once.
    let mut expected: Vec<String> = Vec::new();
    for library in tech.libraries() {
        if library.enabled && !expected.contains(&library.name) {
            expected.push(library.name);
        }
    }
    let listed: Vec<&str> = project.libraries.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(listed, expected);
    for library in &project.libraries {
        assert!(library.read_only, "{}", library.name);
        assert_eq!(library.error, None, "{}", library.name);
        assert_eq!(library.defined_in, file::tech_file(&project.root));
    }
    // The summary has a line for every section.
    let summary = report::summary(&tech, &catalog);
    for section in report::SECTIONS {
        assert!(summary.iter().any(|l| l.section == section), "{section}");
    }
}

#[test]
fn ihp_sg13g2_snapshot() {
    snapshot_round_trip("ihp-sg13g2");
}

#[test]
fn sky130a_snapshot() {
    snapshot_round_trip("sky130A");
}

#[test]
fn the_snapshot_follows_the_pdk_folder_of_the_machine() {
    let Some(pdk) = pdk("ihp-sg13g2") else {
        return;
    };
    let (tech, catalog) = scan(&pdk);
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("design");
    let project = Project::create_for_pdk(&root, None, &pdk, &tech, &catalog).unwrap();
    let before = project.xschem_rc.clone();

    // Another machine: the same PDK elsewhere, reached through a link.
    let elsewhere = tmp.path().join("pdks");
    fs::create_dir_all(&elsewhere).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(pdk.dir(), elsewhere.join(&pdk.name)).unwrap();
    #[cfg(not(unix))]
    return;
    fs::write(
        project.local_file(),
        format!("pdk_root = \"{}\"\n", elsewhere.display()),
    )
    .unwrap();
    let moved = Project::load(&root).unwrap();
    assert_eq!(moved.pdk.var("PDK_ROOT"), Some(elsewhere.to_str().unwrap()));
    if let Some(rc) = before.first() {
        let relative = rc.strip_prefix(&pdk.root).unwrap();
        assert_eq!(moved.xschem_rc, [elsewhere.join(relative)]);
    }
    for library in &moved.libraries {
        assert!(library.path.starts_with(&elsewhere), "{}", library.name);
        assert_eq!(library.error, None, "{}", library.name);
    }
    assert_eq!(moved.snapshot_stale(), None);
}
