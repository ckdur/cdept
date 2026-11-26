//! `[python]` of cdept.toml and `python` of cdept.local.toml: the interpreter Python
//! tools run with, and the tool environment that carries it.

use std::fs;
use std::path::Path;

use libcdept::Project;

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn env_of(p: &Project, key: &str) -> Option<String> {
    p.tool_env()
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.to_string_lossy().into_owned())
}

#[test]
fn a_venv_or_an_interpreter_is_used_and_the_local_file_wins() {
    let _env = libcdept::testutil::hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    write(&dir.join(".venv/bin/python3"), "#!/bin/sh\n");
    write(&dir.join("other/python"), "#!/bin/sh\n");

    // No setting: nothing added.
    write(&dir.join("cdept.toml"), "name = \"t\"\n");
    let p = Project::load(&dir).unwrap();
    assert_eq!(p.python.interpreter, None);
    assert_eq!(env_of(&p, "VIRTUAL_ENV"), None);

    // A venv of the project.
    write(&dir.join("cdept.toml"), "name = \"t\"\n[python]\nvenv = \".venv\"\n");
    let p = Project::load(&dir).unwrap();
    assert_eq!(p.python.interpreter, Some(dir.join(".venv/bin/python3")));
    assert_eq!(p.python(), Some(dir.join(".venv/bin/python3")));
    assert_eq!(env_of(&p, "VIRTUAL_ENV").unwrap(), dir.join(".venv").display().to_string());
    assert!(env_of(&p, "PATH").unwrap().starts_with(&dir.join(".venv/bin").display().to_string()));
    assert_eq!(
        env_of(&p, "CDEPT_PYTHON").unwrap(),
        dir.join(".venv/bin/python3").display().to_string()
    );

    // This user's own interpreter.
    write(&dir.join("cdept.local.toml"), "python = \"other/python\"\n");
    let p = Project::load(&dir).unwrap();
    assert_eq!(p.python.interpreter, Some(dir.join("other/python")));
    assert_eq!(p.python.venv, None);
    assert_eq!(env_of(&p, "VIRTUAL_ENV"), None);

    // Wrong ones are warnings, not errors.
    write(&dir.join("cdept.local.toml"), "python = \"missing/python\"\n");
    let p = Project::load(&dir).unwrap();
    assert_eq!(p.python.interpreter, None);
    assert!(p.warnings.iter().any(|w| w.contains("does not exist")), "{:?}", p.warnings);
    fs::create_dir_all(dir.join("notvenv")).unwrap();
    write(&dir.join("cdept.local.toml"), "python = \"notvenv\"\n");
    let p = Project::load(&dir).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("not a virtual environment")));
    write(&dir.join("cdept.local.toml"), "python = 3\n");
    let p = Project::load(&dir).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("`python` must be a text")));
}
