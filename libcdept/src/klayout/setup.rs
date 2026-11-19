//! Where KLayout and the helper script are, and how the helper is started
//! (docs/KLAYOUT.md §5).
//!
//! The helper is a Python program that KLayout's own interpreter runs:
//! `klayout -z -nc -e -r <helper>/cdept_klayout/main.py -rd connect=<socket>
//! -rd frame_dir=<folder> [-rd display=1]` (see `cdept-klayout/helper`). `-z`: no main
//! window, but layout views work; `-nc`: the user's `klayoutrc` is not read; `-e`:
//! editable mode. Autorun macros on `KLAYOUT_PATH` still run, so a PDK's
//! PCell libraries register themselves.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::Project;

/// Environment variable naming the `klayout` binary explicitly.
pub const KLAYOUT_ENV: &str = "CDEPT_KLAYOUT";
/// Environment variable naming the helper folder (the one holding `cdept_klayout/`).
pub const HELPER_ENV: &str = "CDEPT_KLAYOUT_HELPER";
/// Set for the helper when the project names a technology (`[klayout] tech`).
pub const TECH_ENV: &str = "CDEPT_KLAYOUT_TECH";
/// Set for the helper when the project names a layer properties file.
pub const LAYER_PROPERTIES_ENV: &str = "CDEPT_KLAYOUT_LAYER_PROPERTIES";
/// The helper's main script, relative to the helper folder.
pub const MAIN_SCRIPT: &str = "cdept_klayout/main.py";

/// The development tree (the workspace folder), when this was built from one.
fn dev_tree() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .to_path_buf();
    dir.join("cdept-klayout").is_dir().then_some(dir)
}

/// The `klayout` binary: `$CDEPT_KLAYOUT`, the build of the development tree
/// (`cdept-klayout/.bin/klayout`), `klayout` next to the running program, or on `PATH`.
pub fn find_klayout() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(KLAYOUT_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if let Some(tree) = dev_tree() {
        let built = tree.join("cdept-klayout/.bin/klayout");
        if built.is_file() {
            return Some(built);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("klayout");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("klayout"))
        .find(|candidate| candidate.is_file())
}

/// The helper folder: `$CDEPT_KLAYOUT_HELPER`, `<prefix>/share/cdept-klayout` next to
/// the running program (`<prefix>/bin`), or `cdept-klayout/helper` of the development
/// tree. It must hold [`MAIN_SCRIPT`].
pub fn find_helper_dir() -> Option<PathBuf> {
    let has_main = |dir: &Path| dir.join(MAIN_SCRIPT).is_file();
    if let Some(path) = std::env::var_os(HELPER_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(Path::parent)
    {
        let installed = prefix.join("share/cdept-klayout");
        if has_main(&installed) {
            return Some(installed);
        }
    }
    let tree = dev_tree()?.join("cdept-klayout/helper");
    has_main(&tree).then_some(tree)
}

/// The arguments of `klayout` that run the helper.
pub fn helper_args(
    helper_dir: &Path,
    socket: &Path,
    frame_dir: &Path,
    display: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-z".into(), "-nc".into(), "-e".into(), "-r".into()];
    args.push(helper_dir.join(MAIN_SCRIPT).into_os_string());
    let mut define = |name: &str, value: &OsStr| {
        args.push("-rd".into());
        let mut pair = OsString::from(format!("{name}="));
        pair.push(value);
        args.push(pair);
    };
    define("connect", socket.as_os_str());
    define("frame_dir", frame_dir.as_os_str());
    if display {
        define("display", OsStr::new("1"));
    }
    args
}

/// Environment for the helper besides the project's: the shared libraries of a
/// `klayout` built in place (they sit next to the binary), and no window system.
pub fn helper_env(klayout: &Path) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    if let Some(dir) = klayout.parent()
        && has_klayout_libraries(dir)
    {
        let mut value = dir.as_os_str().to_owned();
        if let Some(old) = std::env::var_os("LD_LIBRARY_PATH").filter(|v| !v.is_empty()) {
            value.push(":");
            value.push(old);
        }
        env.push(("LD_LIBRARY_PATH".into(), value));
    }
    // `-z` opens no window; this keeps Qt from looking for a display at all.
    env.push(("QT_QPA_PLATFORM".into(), "offscreen".into()));
    // A private KLayout home: the user's `~/.klayout` (salt packages, autorun macros)
    // would otherwise run in every helper, e.g. `klive` taking port 8082 away from the
    // user's own KLayout.
    if let Some(home) = private_home() {
        env.push(("KLAYOUT_HOME".into(), home.into_os_string()));
    }
    env
}

/// `$CDEPT_KLAYOUT_HOME`, else `$XDG_CACHE_HOME/cdept/klayout-home` (or
/// `~/.cache/cdept/klayout-home`), created if needed.
fn private_home() -> Option<std::path::PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let dir = match var("CDEPT_KLAYOUT_HOME") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => var("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| var("HOME").map(|home| std::path::Path::new(&home).join(".cache")))?
            .join("cdept")
            .join("klayout-home"),
    };
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn has_klayout_libraries(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("libklayout_") && name.contains(".so"))
        })
    })
}

/// The project's environment for KLayout: `PDK_ROOT`/`PDK` ([`Project::tool_env`]),
/// `KLAYOUT_PATH` ([`Project::klayout_path`], ahead of an inherited one), and the
/// technology and layer properties of `[klayout]` ([`TECH_ENV`],
/// [`LAYER_PROPERTIES_ENV`]).
pub fn project_env(project: &Project) -> Vec<(OsString, OsString)> {
    let mut env = project.tool_env();
    let mut path = project.klayout_path();
    if !path.is_empty() {
        if let Some(old) = std::env::var_os("KLAYOUT_PATH").filter(|v| !v.is_empty()) {
            path.extend(std::env::split_paths(&old));
        }
        if let Ok(joined) = std::env::join_paths(path) {
            env.push(("KLAYOUT_PATH".into(), joined));
        }
    }
    if let Some(tech) = &project.klayout.tech {
        env.push((TECH_ENV.into(), tech.into()));
    }
    if let Some(lyp) = &project.klayout.layer_properties {
        env.push((LAYER_PROPERTIES_ENV.into(), lyp.into()));
    }
    env
}

/// Technology and layer properties from an environment made by [`project_env`], as
/// extra `open` parameters (`technology`, `layer_properties`).
pub(crate) fn open_extras(
    env: &[(OsString, OsString)],
) -> serde_json::Map<String, serde_json::Value> {
    let mut extras = serde_json::Map::new();
    for (name, value) in env {
        let key = if name == TECH_ENV {
            "technology"
        } else if name == LAYER_PROPERTIES_ENV {
            "layer_properties"
        } else {
            continue;
        };
        extras.insert(key.into(), value.to_string_lossy().into_owned().into());
    }
    extras
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_is_run_by_klayout() {
        let args = helper_args(
            Path::new("/h"),
            Path::new("/r/socket"),
            Path::new("/r"),
            true,
        );
        let args: Vec<_> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "-z",
                "-nc",
                "-e",
                "-r",
                "/h/cdept_klayout/main.py",
                "-rd",
                "connect=/r/socket",
                "-rd",
                "frame_dir=/r",
                "-rd",
                "display=1"
            ]
        );
        let headless = helper_args(Path::new("/h"), Path::new("/s"), Path::new("/r"), false);
        assert!(!headless.iter().any(|a| a == "display=1"));
    }

    #[test]
    fn technology_goes_into_open() {
        let env = vec![
            (OsString::from("PDK"), OsString::from("x")),
            (OsString::from(TECH_ENV), OsString::from("sg13g2")),
            (
                OsString::from(LAYER_PROPERTIES_ENV),
                OsString::from("/t/a.lyp"),
            ),
        ];
        let extras = open_extras(&env);
        assert_eq!(extras["technology"], "sg13g2");
        assert_eq!(extras["layer_properties"], "/t/a.lyp");
        assert_eq!(extras.len(), 2);
    }

    #[test]
    fn libraries_next_to_a_built_klayout_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let klayout = dir.path().join("klayout");
        std::fs::write(&klayout, "").unwrap();
        assert!(
            !helper_env(&klayout)
                .iter()
                .any(|(n, _)| n == "LD_LIBRARY_PATH")
        );
        std::fs::write(dir.path().join("libklayout_db.so.0"), "").unwrap();
        let env = helper_env(&klayout);
        let (_, value) = env.iter().find(|(n, _)| n == "LD_LIBRARY_PATH").unwrap();
        assert!(
            value
                .to_str()
                .unwrap()
                .starts_with(dir.path().to_str().unwrap())
        );
    }
}
