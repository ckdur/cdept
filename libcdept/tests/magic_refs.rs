//! References between Magic views across libraries: relative `use` paths, the
//! `.mag.json` sidecar, rename/copy/move following, missing cells, broken files.
//! Hermetic: no Magic needed.

use std::fs;
use std::path::Path;

use libcdept::Project;
use libcdept::layout::meta::{Meta, sidecar_path};
use libcdept::magic::mag;
use libcdept::magic::refs::{self, How, Target};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn mag_with(uses: &[(&str, &str, &str)]) -> String {
    let mut t = String::from("magic\ntech sky130A\ntimestamp 5\n<< metal1 >>\nrect 0 0 10 10\n");
    for (cell, inst, path) in uses {
        t += &format!(
            "use {cell}  {inst} {path}\ntimestamp 1\ntransform 1 0 0 0 1 0\nbox 0 0 100 100\n"
        );
    }
    t += "<< end >>\n";
    t
}

/// Two `cells` libraries: `base/inv`, `top/chip` placing it with an absolute path (as
/// Magic writes it), and `top/other` placing it by name only.
fn project(dir: &Path) -> Project {
    write(
        &dir.join("cdept.toml"),
        "name = \"t\"\n[libraries]\nbase = { path = \"libs/base\", layout = \"cells\" }\n\
         top = { path = \"deep/er/libs/top\", layout = \"cells\" }\n",
    );
    let inv_dir = dir.join("libs/base/inv/layout");
    write(&inv_dir.join("inv.mag"), &mag_with(&[]));
    write(
        &dir.join("deep/er/libs/top/chip/layout/chip.mag"),
        &mag_with(&[(
            "inv",
            "inv_0",
            &inv_dir.canonicalize().unwrap().to_string_lossy(),
        )]),
    );
    write(
        &dir.join("deep/er/libs/top/other/layout/other.mag"),
        &mag_with(&[("inv", "u1", "")]),
    );
    Project::load(dir).unwrap()
}

#[test]
fn references_resolve_normalize_and_follow() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let p = project(&dir);
    let chip = dir.join("deep/er/libs/top/chip/layout/chip.mag");
    let other = dir.join("deep/er/libs/top/other/layout/other.mag");

    let r = refs::resolve_file(&p, &chip).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].how, How::Path);
    let Target::Project(v) = &r[0].target else {
        panic!("{:?}", r[0].target)
    };
    assert_eq!(v.label(), "base/inv/layout");
    let r = refs::resolve_file(&p, &other).unwrap();
    assert_eq!(r[0].how, How::Name);

    // check: an absolute path and a name-only use are reported
    let problems = refs::check(&p, false);
    assert!(
        problems.iter().any(|x| x.text.contains("absolute path")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|x| x.text.contains("by name only")),
        "{problems:?}"
    );

    // normalize: relative paths, sidecar entries
    let n = refs::normalize(&p, &chip).unwrap();
    assert_eq!(n.paths, 1);
    assert!(n.sidecar);
    let text = fs::read_to_string(&chip).unwrap();
    assert!(
        text.contains("use inv  inv_0 ../../../../../../libs/base/inv/layout\n"),
        "{text}"
    );
    let meta = Meta::read(&sidecar_path(&chip)).unwrap();
    assert_eq!(meta.label(), "top/chip/layout");
    assert_eq!(meta.references["inv"].library, "base");
    // a second run changes nothing
    let n = refs::normalize(&p, &chip).unwrap();
    assert_eq!((n.paths, n.sidecar), (0, false));
    refs::normalize(&p, &other).unwrap();
    assert!(
        refs::check(&p, false).is_empty(),
        "{:?}",
        refs::check(&p, false)
    );

    // rename base/inv -> nand: both placing views follow (use line and sidecar)
    let base = p.library("base").unwrap();
    let followed = base.rename_cell_following(&p, "inv", "nand").unwrap();
    assert_eq!(
        followed.iter().filter(|f| !f.is_problem()).count(),
        2,
        "{followed:?}"
    );
    let p = Project::load(&dir).unwrap();
    let text = fs::read_to_string(&chip).unwrap();
    assert!(
        text.contains("use nand  inv_0 ../../../../../../libs/base/nand/layout\n"),
        "{text}"
    );
    let meta = Meta::read(&sidecar_path(&chip)).unwrap();
    assert_eq!(meta.references["nand"].cell, "nand");
    assert!(!meta.references.contains_key("inv"));
    // the renamed view's own sidecar says what it is now
    let own = dir.join("libs/base/nand/layout/nand.mag");
    let own_meta = Meta::read(&sidecar_path(&own)).unwrap();
    assert_eq!(
        (own_meta.cell.as_str(), own_meta.top.as_str()),
        ("nand", "nand")
    );
    assert!(
        refs::check(&p, false).is_empty(),
        "{:?}",
        refs::check(&p, false)
    );

    // copy top/chip into base (another depth): its relative path is written again
    let top = p.library("top").unwrap();
    top.copy_cell("chip", p.library("base").unwrap(), "chip2")
        .unwrap();
    let copy = dir.join("libs/base/chip2/layout/chip2.mag");
    let text = fs::read_to_string(&copy).unwrap();
    assert!(
        text.contains("use nand  inv_0 ../../nand/layout\n"),
        "{text}"
    );
    let m = Meta::read(&sidecar_path(&copy)).unwrap();
    assert_eq!(m.label(), "base/chip2/layout");
    let p = Project::load(&dir).unwrap();
    assert!(
        refs::check(&p, false).is_empty(),
        "{:?}",
        refs::check(&p, false)
    );

    // delete base/nand: the views that placed it are reported, and show it missing
    let orphans = refs::placed_by(&p, "base", "nand", None);
    assert_eq!(orphans.len(), 3, "{orphans:?}");
    p.library("base").unwrap().delete_cell("nand").unwrap();
    let p = Project::load(&dir).unwrap();
    let r = refs::resolve_file(&p, &chip).unwrap();
    assert_eq!(r[0].target, Target::Missing);
    let n = refs::normalize(&p, &chip).unwrap();
    assert_eq!(n.missing, vec![("inv_0".to_string(), "nand".to_string())]);
    // the sidecar keeps what it knew of the missing cell
    let meta = Meta::read(&sidecar_path(&chip)).unwrap();
    assert!(meta.references.contains_key("nand"));
}

#[test]
fn cyclic_names_spaces_and_broken_files_do_not_break_anything() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    write(
        &dir.join("cdept.toml"),
        "name = \"t\"\n[libraries]\n\"lib one\" = { path = \"lib one\", layout = \"cells\" }\n",
    );
    let a = dir.join("lib one/a.b/layout/a.b.mag");
    let b = dir.join("lib one/bé/layout/bé.mag");
    // a places bé, bé places a.b: a cycle (Magic refuses it; CDePT must not loop)
    write(&a, &mag_with(&[("bé", "x", "../../bé/layout")]));
    write(&b, &mag_with(&[("a.b", "y", "../../a.b/layout")]));
    // a truncated file, an empty file, a binary file and a broken sidecar
    write(
        &dir.join("lib one/cut/layout/cut.mag"),
        "magic\ntech x\nuse a.b  z ../../a.b/layo",
    );
    write(&dir.join("lib one/empty/layout/empty.mag"), "");
    fs::create_dir_all(dir.join("lib one/bin/layout")).unwrap();
    fs::write(
        dir.join("lib one/bin/layout/bin.mag"),
        [0u8, 159, 146, 150, 255, 10, 117],
    )
    .unwrap();
    write(&sidecar_path(&b), "{ not json");
    let p = Project::load(&dir).unwrap();
    for v in refs::mag_views(&p) {
        let _ = refs::resolve_file(&p, &v.file);
        let _ = refs::normalize(&p, &v.file);
    }
    let problems = refs::check(&p, false);
    assert!(
        problems
            .iter()
            .any(|x| x.view.contains("cut") && x.text.contains("cut short"))
    );
    assert!(
        problems.iter().any(|x| x.view.contains("bin")),
        "{problems:?}"
    );
    // the broken sidecar was replaced by a good one by normalize
    assert!(Meta::read(&sidecar_path(&b)).is_ok());
    let r = refs::resolve_file(&p, &a).unwrap();
    assert!(matches!(r[0].target, Target::Project(_)));
}

#[test]
fn read_only_libraries_are_reported_not_changed() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    write(
        &dir.join("cdept.toml"),
        "name = \"t\"\n[libraries]\nbase = { path = \"base\", layout = \"cells\" }\n\
         ro = { path = \"ro\", layout = \"cells\", read_only = true }\n",
    );
    write(&dir.join("base/inv/layout/inv.mag"), &mag_with(&[]));
    let user = dir.join("ro/user/layout/user.mag");
    write(
        &user,
        &mag_with(&[("inv", "i", "../../../base/inv/layout")]),
    );
    let before = fs::read(&user).unwrap();
    let p = Project::load(&dir).unwrap();
    let n = refs::normalize(&p, &user).unwrap();
    assert!(!n.sidecar);
    let followed = p
        .library("base")
        .unwrap()
        .rename_cell_following(&p, "inv", "inv2")
        .unwrap();
    assert!(followed.iter().any(|f| f.is_problem()), "{followed:?}");
    assert_eq!(fs::read(&user).unwrap(), before);
    assert!(!sidecar_path(&user).exists());
}

#[test]
fn large_hierarchies_resolve_quickly() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    write(
        &dir.join("cdept.toml"),
        "name = \"t\"\n[libraries]\nl = { path = \"l\", layout = \"cells\" }\n",
    );
    // 200 cells, each placing the previous one 20 times
    for i in 0..200 {
        let uses: Vec<(String, String, String)> = if i == 0 {
            Vec::new()
        } else {
            (0..20)
                .map(|k| {
                    (
                        format!("c{}", i - 1),
                        format!("u{k}"),
                        format!("../../c{}/layout", i - 1),
                    )
                })
                .collect()
        };
        let refs: Vec<(&str, &str, &str)> = uses
            .iter()
            .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
            .collect();
        write(
            &dir.join(format!("l/c{i}/layout/c{i}.mag")),
            &mag_with(&refs),
        );
    }
    let p = Project::load(&dir).unwrap();
    let start = std::time::Instant::now();
    let problems = refs::check(&p, false);
    assert!(
        problems.is_empty(),
        "{:?}",
        &problems[..problems.len().min(3)]
    );
    assert!(start.elapsed().as_secs() < 30, "{:?}", start.elapsed());
    let m = mag::read(&dir.join("l/c199/layout/c199.mag")).unwrap();
    assert_eq!(m.uses.len(), 20);
}

#[test]
fn refactor_moves_copies_and_renames_magic_cells() {
    use libcdept::refactor::{self, Operation, Options};
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let p = project(&dir);
    let chip = dir.join("deep/er/libs/top/chip/layout/chip.mag");
    refs::normalize(&p, &chip).unwrap();
    refs::normalize(&p, &dir.join("libs/base/inv/layout/inv.mag")).unwrap();
    let p = Project::load(&dir).unwrap();

    // move base/inv into top as buf: chip follows (name and path)
    let op = Operation::MoveCell {
        library: "base".into(),
        cell: "inv".into(),
        to_library: "top".into(),
        new: "buf".into(),
    };
    let plan = refactor::plan(&p, &op, &Options::default()).unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    refactor::apply(&p, &plan).unwrap();
    let text = fs::read_to_string(&chip).unwrap();
    assert!(text.contains("use buf  inv_0 ../../buf/layout\n"), "{text}");
    let p = Project::load(&dir).unwrap();
    let meta = Meta::read(&sidecar_path(&chip)).unwrap();
    assert_eq!(meta.references["buf"].library, "top");
    let own = dir.join("deep/er/libs/top/buf/layout/buf.mag");
    assert_eq!(
        Meta::read(&sidecar_path(&own)).unwrap().label(),
        "top/buf/layout"
    );
    let r = refs::resolve_file(&p, &chip).unwrap();
    assert!(matches!(&r[0].target, Target::Project(v) if v.label() == "top/buf/layout"));

    // copy top/chip into base: the copy's path is written for its folder; chip is left
    let before = fs::read(&chip).unwrap();
    let op = Operation::CopyCell {
        library: "top".into(),
        cell: "chip".into(),
        to_library: "base".into(),
        new: "chip3".into(),
        hierarchy: false,
    };
    let plan = refactor::plan(&p, &op, &Options::default()).unwrap();
    refactor::apply(&p, &plan).unwrap();
    assert_eq!(fs::read(&chip).unwrap(), before);
    let copy = dir.join("libs/base/chip3/layout/chip3.mag");
    let text = fs::read_to_string(&copy).unwrap();
    assert!(
        text.contains("use buf  inv_0 ../../../../deep/er/libs/top/buf/layout\n"),
        "{text}"
    );
    let p = Project::load(&dir).unwrap();
    assert!(
        refs::check(&p, false).is_empty(),
        "{:?}",
        refs::check(&p, false)
    );

    // a file open with unsaved changes blocks the rename
    let mut options = Options::default();
    options.busy.insert(chip.clone());
    let op = Operation::RenameCell {
        library: "top".into(),
        cell: "buf".into(),
        new: "buf2".into(),
    };
    let plan = refactor::plan(&p, &op, &options).unwrap();
    assert!(!plan.blockers.is_empty());
}
