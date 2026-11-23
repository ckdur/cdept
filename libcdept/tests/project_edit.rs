//! Library definitions and the project file: editing a definition where it is
//! defined, the format key, own suffixes, name checks, safe writing, the copy rule.

use std::fs;
use std::path::Path;

use libcdept::formats;
use libcdept::project_edit::{
    CopyRule, FORMAT, LibraryChange, Origin, copy_cell_dir, portable, write_file,
};
use libcdept::{AddLibrary, Format, LibraryLayout, PROJECT_FILE, Project, Template};

fn no_env(_: &str) -> Option<String> {
    None
}

/// A project with one `cells` library `analog` (cell `ota` with a schematic) and a
/// library `prims` defined in an included file.
fn project(root: &Path) -> Project {
    let mut project = Project::create(root, Some("demo")).unwrap();
    let options = AddLibrary {
        create: true,
        ..Default::default()
    };
    project
        .add_library("analog", "libs/analog", options)
        .unwrap();
    project
        .library("analog")
        .unwrap()
        .create_view("ota", Template::XschemSchematic)
        .unwrap();
    let shared = root.parent().unwrap().join("shared");
    fs::create_dir_all(shared.join("prims")).unwrap();
    fs::write(shared.join("prims/nmos.sym"), "").unwrap();
    fs::write(
        shared.join("libraries.toml"),
        "# shared by the team\n[libraries]\nprims = \"prims\"  # the primitives\n",
    )
    .unwrap();
    let text = fs::read_to_string(&project.file).unwrap();
    let text = text.replacen(
        "[libraries]",
        "include = [\"../shared/libraries.toml\"]\n\n[libraries]",
        1,
    );
    fs::write(&project.file, text).unwrap();
    project.reload().unwrap();
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    project
}

#[test]
fn new_projects_and_libraries_say_their_format_and_layout() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("proj"));
    let text = fs::read_to_string(&project.file).unwrap();
    assert!(text.contains(&format!("format = {FORMAT}")), "{text}");
    // The layout is written, not left to detection.
    assert!(
        text.contains("analog = { path = \"libs/analog\", layout = \"cells\" }"),
        "{text}"
    );
    let lib = fs::read_to_string(project.root.join("libs/analog/lib.toml")).unwrap();
    assert!(lib.contains("format = 1"), "{lib}");

    // A flat folder added without saying the layout is written as flat, and stays
    // flat when a lib.toml appears later.
    let mut project = project;
    fs::create_dir_all(project.root.join("syms")).unwrap();
    fs::write(project.root.join("syms/res.sym"), "").unwrap();
    project
        .add_library("syms", "syms", AddLibrary::default())
        .unwrap();
    let text = fs::read_to_string(&project.file).unwrap();
    assert!(text.contains("layout = \"flat\""), "{text}");
    fs::write(project.root.join("syms/lib.toml"), "description = \"x\"\n").unwrap();
    project.reload().unwrap();
    assert_eq!(project.library("syms").unwrap().layout, LibraryLayout::Flat);
}

#[test]
fn a_newer_format_loads_with_a_warning() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(&tmp.path().join("proj"));
    let text = fs::read_to_string(&project.file).unwrap();
    fs::write(&project.file, text.replace("format = 1", "format = 7")).unwrap();
    fs::write(
        project.root.join("libs/analog/lib.toml"),
        "format = 3\ndescription = \"amps\"\n",
    )
    .unwrap();
    project.reload().unwrap();
    assert_eq!(project.libraries.len(), 2);
    let newer: Vec<_> = project
        .warnings
        .iter()
        .filter(|w| w.contains("this CDePT knows format 1"))
        .collect();
    assert_eq!(newer.len(), 2, "{:?}", project.warnings);
    assert!(newer[0].contains("format 7") && newer[0].contains("cdept.toml"));
    assert!(newer[1].contains("format 3") && newer[1].contains("lib.toml"));
    assert_eq!(
        project.library("analog").unwrap().description.as_deref(),
        Some("amps")
    );
}

#[test]
fn a_definition_is_changed_in_the_project_file() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(&tmp.path().join("proj"));
    let definition = project.library_definition("analog").unwrap();
    assert_eq!(definition.path, "libs/analog");
    assert_eq!(definition.layout, Some(LibraryLayout::Cells));
    assert!(!definition.read_only && !definition.included);
    assert_eq!(definition.file, project.file);
    assert_eq!(project.library_origin("analog"), Some(Origin::Project));
    assert!(project.library_origin("nope").is_none());

    // Read-only on, with a hand-written comment kept.
    let text = fs::read_to_string(&project.file).unwrap();
    fs::write(
        &project.file,
        text.replace("layout = \"cells\" }", "layout = \"cells\" } # ours"),
    )
    .unwrap();
    project
        .set_library(
            "analog",
            &LibraryChange {
                read_only: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(project.library("analog").unwrap().read_only);
    let text = fs::read_to_string(&project.file).unwrap();
    assert!(
        text.contains(
            "analog = { path = \"libs/analog\", layout = \"cells\", read_only = true } # ours"
        ),
        "{text}"
    );
    assert!(text.contains("# CDePT project file"));

    // A read-only library takes no description; a writable one does.
    let describe = LibraryChange {
        description: Some("amplifiers".into()),
        ..Default::default()
    };
    let error = project.set_library("analog", &describe).unwrap_err();
    assert!(format!("{error:#}").contains("read-only"), "{error:#}");
    project
        .set_library(
            "analog",
            &LibraryChange {
                read_only: Some(false),
                description: Some("amplifiers".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let lib = project.library("analog").unwrap();
    assert!(!lib.read_only);
    assert_eq!(lib.description.as_deref(), Some("amplifiers"));
    assert_eq!(
        fs::read_to_string(&project.file)
            .unwrap()
            .lines()
            .find(|l| l.starts_with("analog"))
            .unwrap(),
        "analog = { path = \"libs/analog\", layout = \"cells\" } # ours"
    );

    // Another folder: it must exist, and is written as given.
    let moved = LibraryChange {
        path: Some("libs/moved".into()),
        ..Default::default()
    };
    assert!(project.set_library("analog", &moved).is_err());
    fs::rename(
        project.root.join("libs/analog"),
        project.root.join("libs/moved"),
    )
    .unwrap();
    project.set_library("analog", &moved).unwrap();
    let lib = project.library("analog").unwrap();
    assert_eq!(lib.path, project.root.join("libs/moved"));
    assert!(lib.cell("ota").is_some());

    // The layout: said, or detected again.
    project
        .set_library(
            "analog",
            &LibraryChange {
                layout: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(project.library_definition("analog").unwrap().layout, None);
    assert_eq!(
        fs::read_to_string(&project.file)
            .unwrap()
            .lines()
            .find(|l| l.starts_with("analog"))
            .unwrap(),
        "analog = { path = \"libs/moved\" } # ours"
    );
    assert!(project.set_library("nope", &moved).is_err());
}

#[test]
fn a_library_of_an_included_file_is_changed_and_removed_there() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(&tmp.path().join("proj"));
    let shared = tmp
        .path()
        .join("shared/libraries.toml")
        .canonicalize()
        .unwrap();
    let definition = project.library_definition("prims").unwrap();
    assert!(definition.included);
    assert_eq!(definition.file, shared);
    assert_eq!(definition.path, "prims");
    assert_eq!(
        project.library_origin("prims"),
        Some(Origin::Included(shared.clone()))
    );
    let before = fs::read_to_string(&project.file).unwrap();

    project
        .set_library(
            "prims",
            &LibraryChange {
                read_only: Some(true),
                layout: Some(Some(LibraryLayout::Flat)),
                ..Default::default()
            },
        )
        .unwrap();
    let lib = project.library("prims").unwrap();
    assert!(lib.read_only);
    assert_eq!(lib.layout, LibraryLayout::Flat);
    assert_eq!(
        fs::read_to_string(&shared).unwrap(),
        "# shared by the team\n[libraries]\n\
         prims = { path = \"prims\", layout = \"flat\", read_only = true }  # the primitives\n"
    );
    assert_eq!(fs::read_to_string(&project.file).unwrap(), before);

    // A new folder is relative to the included file, not to the project.
    fs::create_dir_all(tmp.path().join("shared/more")).unwrap();
    project
        .set_library(
            "prims",
            &LibraryChange {
                path: Some("more".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        project.library("prims").unwrap().path,
        tmp.path().join("shared/more").canonicalize().unwrap()
    );

    project.remove_library("prims").unwrap();
    assert!(project.library("prims").is_none());
    assert_eq!(
        fs::read_to_string(&shared).unwrap(),
        "# shared by the team\n[libraries]\n"
    );
    assert_eq!(fs::read_to_string(&project.file).unwrap(), before);
}

#[test]
fn a_description_does_not_flip_a_detected_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    fs::create_dir_all(root.join("syms")).unwrap();
    fs::write(root.join("syms/res.sym"), "").unwrap();
    fs::write(
        root.join(PROJECT_FILE),
        "name = \"p\"\n[libraries]\nsyms = \"syms\"\n",
    )
    .unwrap();
    let mut project = Project::load_with(&root, no_env).unwrap();
    assert_eq!(project.library("syms").unwrap().layout, LibraryLayout::Flat);
    project
        .set_library(
            "syms",
            &LibraryChange {
                description: Some("symbols".into()),
                ..Default::default()
            },
        )
        .unwrap();
    // The lib.toml that now exists would make the folder look like `cells`.
    assert!(root.join("syms/lib.toml").is_file());
    let lib = project.library("syms").unwrap();
    assert_eq!(lib.layout, LibraryLayout::Flat);
    assert_eq!(lib.description.as_deref(), Some("symbols"));
    assert_eq!(lib.cells.len(), 1);
    assert!(
        fs::read_to_string(&project.file)
            .unwrap()
            .contains("syms = { path = \"syms\", layout = \"flat\" }")
    );
}

#[test]
fn the_same_folder_or_folder_name_twice_is_noticed() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let project = project(&tmp.path().join("proj"));
    fs::create_dir_all(project.root.join("other/analog")).unwrap();
    assert_eq!(project.folder_clash("digital", "libs/digital"), None);
    let same = project.folder_clash("again", "libs/analog").unwrap();
    assert!(same.contains("already library 'analog'"), "{same}");
    let name = project.folder_clash("analog2", "other/analog").unwrap();
    assert!(
        name.contains("also called 'analog'") && name.contains("xschem"),
        "{name}"
    );
    // The library itself does not clash with itself (a change of its own folder).
    assert_eq!(project.folder_clash("analog", "libs/analog"), None);
}

#[test]
fn own_suffixes_change_what_the_libraries_show() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(&tmp.path().join("proj"));
    let cell = project.root.join("libs/analog/ota");
    fs::write(cell.join("ota.net"), "* netlist\n").unwrap();
    fs::write(cell.join("models.lib"), "* models\n").unwrap();
    fs::write(cell.join("notes.md"), "").unwrap();
    fs::write(cell.join("plot.png"), "").unwrap();
    fs::write(project.root.join("libs/analog/README"), "").unwrap();
    project.reload().unwrap();
    let analog = project.library("analog").unwrap();
    let formats_of = |project: &Project| -> Vec<Format> {
        project
            .library("analog")
            .unwrap()
            .cell("ota")
            .unwrap()
            .entries
            .iter()
            .filter_map(|e| e.format)
            .collect()
    };
    assert!(formats_of(&project).contains(&Format::Liberty));
    // What says nothing to CDePT is counted, per library and per cell.
    let unknown = formats::unrecognised(analog);
    let names: Vec<_> = unknown
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["README", "notes.md", "ota.net", "plot.png"]);
    assert_eq!(formats::unrecognised_in(analog, "ota").len(), 3);
    assert!(formats::unrecognised_in(analog, "nope").is_empty());

    let mut text = fs::read_to_string(&project.file).unwrap();
    text.push_str(
        "\n[suffixes]\n\".net\" = \"spice\"\n\".lib\" = \"spice\"\n\".sch\" = \"none\"\n\
         \".bad\" = \"word\"\n",
    );
    fs::write(&project.file, text).unwrap();
    project.reload().unwrap();
    assert_eq!(formats_of(&project), [Format::Spice, Format::Spice]);
    assert_eq!(
        project
            .warnings
            .iter()
            .filter(|w| w.contains("[suffixes]"))
            .count(),
        1,
        "{:?}",
        project.warnings
    );
    let analog = project.library("analog").unwrap();
    // The schematic is no design file here any more, the netlist is one.
    assert_eq!(formats::unrecognised_in(analog, "ota").len(), 2);
    assert_eq!(
        formats::Suffixes::read(&project.file).entries(),
        [
            (".net".to_string(), "spice"),
            (".lib".to_string(), "spice"),
            (".sch".to_string(), "none")
        ]
    );

    // A scan after the load (another thread, as the GUI does) sees the same.
    let mut library = analog.clone();
    let scanned = std::thread::spawn(move || {
        library.scan();
        library
    })
    .join()
    .unwrap();
    assert_eq!(&scanned, analog);

    // Without the table the built-in suffixes are back.
    let text = fs::read_to_string(&project.file).unwrap();
    let text = &text[..text.find("[suffixes]").unwrap()];
    fs::write(&project.file, text).unwrap();
    project.reload().unwrap();
    assert!(formats_of(&project).contains(&Format::XschemSchematic));
    let mut library = project.library("analog").unwrap().clone();
    library.scan();
    assert_eq!(&library, project.library("analog").unwrap());
}

#[test]
fn names_that_will_cause_trouble_are_warned_about() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(&tmp.path().join("proj"));
    let lib = project.root.join("libs/analog");
    fs::create_dir_all(lib.join("OTA/schematic")).unwrap();
    if !lib.join("ota").join("schematic").is_dir() || lib.join("OTA") == lib.join("ota") {
        return;
    }
    // On a file system that ignores case the two are one folder: nothing to check.
    let two = fs::read_dir(&lib)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("ota"))
        .count();
    if two != 2 {
        return;
    }
    fs::create_dir_all(project.root.join("libs/Analog")).unwrap();
    let text = fs::read_to_string(&project.file).unwrap();
    fs::write(
        &project.file,
        format!("{text}\n[libraries.Analog]\npath = \"libs/Analog\"\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let _ = fs::write(
            lib.join("ota").join(OsStr::from_bytes(b"bad\xffname.sch")),
            "",
        );
    }
    project.reload().unwrap();
    let has = |text: &str| project.warnings.iter().any(|w| w.contains(text));
    assert!(
        has("libraries 'Analog' and 'analog' differ only by case"),
        "{:?}",
        project.warnings
    );
    assert!(
        has("cells 'OTA' and 'ota' differ only by case"),
        "{:?}",
        project.warnings
    );
    #[cfg(unix)]
    if fs::read_dir(lib.join("ota")).unwrap().count() > 1 {
        assert!(has("not UTF-8"), "{:?}", project.warnings);
    }
}

#[cfg(unix)]
#[test]
fn saving_keeps_a_link_and_the_permissions() {
    let _env = libcdept::testutil::hermetic_env();
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real.toml");
    let link = tmp.path().join("link.toml");
    fs::write(&real, "a = 1\n").unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink("real.toml", &link).unwrap();

    write_file(&link, "a = 2\n").unwrap();
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&real).unwrap(), "a = 2\n");
    assert_eq!(
        fs::metadata(&real).unwrap().permissions().mode() & 0o777,
        0o640
    );
    // No temporary file is left.
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);

    // A new file is simply written.
    let new = tmp.path().join("new.toml");
    write_file(&new, "b = 1\n").unwrap();
    assert_eq!(fs::read_to_string(&new).unwrap(), "b = 1\n");

    // The project file as a link: editing the project keeps it one. (The project is
    // where the file really is: paths are relative to the file that holds them.)
    let root = tmp.path().join("proj");
    let project = Project::create(&root, None).unwrap();
    let kept = tmp.path().join("kept.toml");
    fs::rename(&project.file, &kept).unwrap();
    std::os::unix::fs::symlink("../kept.toml", root.join(PROJECT_FILE)).unwrap();
    let mut project = Project::load_with(&root, no_env).unwrap();
    fs::create_dir_all(project.root.join("lib")).unwrap();
    project
        .add_library("lib", "lib", AddLibrary::default())
        .unwrap();
    assert!(
        fs::symlink_metadata(root.join(PROJECT_FILE))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::read_to_string(&kept).unwrap().contains("lib = "));

    // A lib.toml that is a link (a description shared between checkouts) stays one.
    let shared = tmp.path().join("shared-lib.toml");
    fs::write(&shared, "description = \"old\"\n").unwrap();
    std::os::unix::fs::symlink(&shared, project.root.join("lib/lib.toml")).unwrap();
    project
        .set_library(
            "lib",
            &LibraryChange {
                description: Some("new".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        fs::symlink_metadata(project.root.join("lib/lib.toml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&shared).unwrap(),
        "description = \"new\"\n"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_reached_through_a_link_is_written_relative() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp = tmp.path().canonicalize().unwrap();
    let root = tmp.join("data/proj");
    fs::create_dir_all(root.join("libs/a")).unwrap();
    fs::create_dir_all(tmp.join("outside/pdk")).unwrap();
    std::os::unix::fs::symlink(&root, tmp.join("proj")).unwrap();
    std::os::unix::fs::symlink(tmp.join("outside/pdk"), root.join("libs/pdk")).unwrap();
    std::os::unix::fs::symlink(root.join("libs"), tmp.join("shortcut")).unwrap();

    assert_eq!(portable(&root, &root.join("libs/a")), "libs/a");
    assert_eq!(portable(&root, &root), ".");
    // The project under another name.
    assert_eq!(portable(&root, &tmp.join("proj/libs/a")), "libs/a");
    assert_eq!(portable(&root, &tmp.join("proj")), ".");
    // A link inside the project that leads outside stays as written.
    assert_eq!(portable(&root, &root.join("libs/pdk")), "libs/pdk");
    assert_eq!(portable(&root, &tmp.join("proj/libs/pdk")), "libs/pdk");
    // A link outside the project that leads inside.
    assert_eq!(portable(&root, &tmp.join("shortcut/a")), "libs/a");
    // Outside stays absolute; relative and `$VAR` texts are left alone.
    let outside = tmp.join("outside/pdk");
    assert_eq!(portable(&root, &outside), outside.to_string_lossy());
    assert_eq!(portable(&root, Path::new("libs/x")), "libs/x");
    assert_eq!(portable(&root, Path::new("$PDK_ROOT/x")), "$PDK_ROOT/x");
}

#[test]
fn a_cell_copy_leaves_results_out() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let mut project = project(&tmp.path().join("proj"));
    let lib = project.root.join("libs/analog");
    let tb = lib.join("ota/tb_ac");
    fs::create_dir_all(tb.join("results/run1")).unwrap();
    fs::write(tb.join("setup.toml"), "").unwrap();
    fs::write(tb.join("results/run1/tran.raw"), "big").unwrap();
    fs::write(tb.join("tran.raw"), "big").unwrap();
    // A view may be called `results`: the rule is about what is inside views.
    fs::create_dir_all(lib.join("ota/results")).unwrap();
    fs::write(lib.join("ota/results/keep.txt"), "").unwrap();
    project.reload().unwrap();

    let analog = project.library("analog").unwrap();
    analog.copy_cell("ota", analog, "ota2").unwrap();
    let copy = lib.join("ota2");
    assert!(copy.join("schematic/ota2.sch").is_file());
    assert!(copy.join("tb_ac/setup.toml").is_file());
    assert!(copy.join("tb_ac/tran.raw").is_file());
    assert!(!copy.join("tb_ac/results").exists());
    assert!(copy.join("results/keep.txt").is_file());

    // The library says what else stays behind.
    assert_eq!(CopyRule::of_library(&lib), CopyRule::default());
    let mut text = fs::read_to_string(lib.join("lib.toml")).unwrap();
    text.push_str("\n[copy]\nexclude = [\"results\", \"*.raw\"]\n");
    fs::write(lib.join("lib.toml"), text).unwrap();
    let rule = CopyRule::of_library(&lib);
    assert!(rule.excludes("tran.raw") && rule.excludes("results") && !rule.excludes("a.sch"));
    let copied = copy_cell_dir(&lib.join("ota"), &lib.join("ota3"), &lib).unwrap();
    assert_eq!(copied.skipped.len(), 2, "{:?}", copied.skipped);
    assert!(!lib.join("ota3/tb_ac/tran.raw").exists());
    assert!(lib.join("ota3/tb_ac/setup.toml").is_file());

    // A copy that fails in the middle leaves nothing behind.
    let error = copy_cell_dir(&lib.join("nowhere"), &lib.join("ota4"), &lib).unwrap_err();
    assert!(
        format!("{error:#}").contains("nothing was copied"),
        "{error:#}"
    );
    assert!(!lib.join("ota4").exists());
}
