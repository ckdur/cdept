//! Support for tests that depend on the process environment.
//!
//! CDePT reads `PDK`, `PDK_ROOT`, `PDKPATH`, `HOME`, `XDG_*` and the `CDEPT_*` variables, and
//! the environment is shared by all the threads of a test binary. Tests are therefore of two
//! kinds:
//!
//! - **hermetic** tests build a synthetic project or PDK in a scratch folder and take a
//!   [`hermetic_env`] guard, so the shell they run from (say with `PDK=ihp-sg13g2` exported)
//!   cannot change the result;
//! - **real-PDK** tests need an installed PDK. They ask [`real_pdk`], print
//!   `skipped: <pdk> not installed` and return when it is absent, and otherwise run with an
//!   explicit `PDK`, `PDK_ROOT` and `PDKPATH`.
//!
//! The guard keeps the environment while it lives and restores it when the last guard of the
//! same kind drops: tests that ask for the same environment (every [`hermetic_env`], every
//! [`real_pdk`] of one PDK) run in parallel, and one that asks for another waits for them.
//! A test must not take a second guard while it holds one.
//!
//! This module is compiled for the tests of this crate and, with the `testutil` feature, for
//! its integration tests and the tests of the other crates.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

/// Where the real PDKs of the machine are, unless `CDEPT_TEST_PDK_ROOT` says otherwise.
pub const DEFAULT_PDK_ROOT: &str = "/opt/ext/OpenPDKs";

/// The variables that choose a PDK or where CDePT keeps its state: unset by
/// [`hermetic_env`].
const PDK_VARS: &[&str] = &[
    "PDK",
    "PDK_ROOT",
    "PDKPATH",
    "CDEPT_PDK_PATH",
    "CDEPT_PDK_DATA",
    "CDEPT_INDEX_CACHE",
    "CDEPT_KLAYOUT_HOME",
    "CDEPT_TECH_SCAN_TIMEOUT",
    "KLAYOUT_HOME",
    "KLAYOUT_PATH",
    "XDG_RUNTIME_DIR",
];

/// Variables that find the tools; [`hermetic_env_without_tools`] unsets them.
const TOOL_VARS: &[&str] = &[
    "CDEPT_XSCHEM",
    "CDEPT_KLAYOUT",
    "CDEPT_KLAYOUT_HELPER",
    "CDEPT_NGSPICE",
    "CDEPT_LIBNGSPICE",
    "CDEPT_XYCE",
    "CDEPT_VACASK",
    "CDEPT_VACASK_PYTHON",
    "CDEPT_MAGIC",
    "CDEPT_NETGEN",
    "CDEPT_OPENROAD",
    "CDEPT_YOSYS",
    "CDEPT_QUCS",
    "CDEPT_UBERWAVE",
    "CDEPT_PYTHON",
    "CDEPT_OPENVAF",
];

type Vars = Vec<(String, Option<OsString>)>;

/// What the guards of one kind share: set up by the first, restored by the last.
struct Active {
    key: String,
    shared: bool,
    users: usize,
    saved: Vec<(String, Option<OsString>)>,
    scratch: PathBuf,
}

static STATE: Mutex<Option<Active>> = Mutex::new(None);
static FREE: Condvar = Condvar::new();
static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The environment of one test, restored when the last guard of its kind drops. See
/// [`hermetic_env`].
pub struct EnvGuard {
    scratch: PathBuf,
}

fn scratch_vars(scratch: &Path) -> Vars {
    let xdg = |sub: &str| Some(scratch.join("xdg").join(sub).into_os_string());
    vec![
        ("HOME".into(), Some(scratch.join("home").into_os_string())),
        ("XDG_CONFIG_HOME".into(), xdg("config")),
        ("XDG_CACHE_HOME".into(), xdg("cache")),
        ("XDG_DATA_HOME".into(), xdg("data")),
        ("XDG_STATE_HOME".into(), xdg("state")),
    ]
}

/// Wait for the environment of kind `key`, then set it with `vars` (`None` unsets).
fn acquire(key: &str, shared: bool, vars: Vars) -> EnvGuard {
    let mut state = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        match state.as_mut() {
            None => break,
            Some(active) if shared && active.shared && active.key == key => {
                active.users += 1;
                return EnvGuard {
                    scratch: active.scratch.clone(),
                };
            }
            Some(_) => {
                state = FREE
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
    }
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let scratch = std::env::temp_dir().join(format!("cdept-testenv-{}-{n}", std::process::id()));
    std::fs::create_dir_all(scratch.join("home"))
        .expect("a scratch folder for the test environment");
    let mut saved: Vec<(String, Option<OsString>)> = Vec::new();
    let pdk_vars: Vars = PDK_VARS.iter().map(|n| (n.to_string(), None)).collect();
    for (name, value) in pdk_vars
        .into_iter()
        .chain(scratch_vars(&scratch))
        // The tests of the fake viewers are about the process backend; the embedded
        // one (when built in) is tested apart, with this variable set to `embedded`.
        .chain([(
            "CDEPT_UBERWAVE_BACKEND".to_string(),
            Some(OsString::from("process")),
        )])
        .chain(vars)
    {
        if !saved.iter().any(|(n, _)| *n == name) {
            saved.push((name.clone(), std::env::var_os(&name)));
        }
        // SAFETY: the state lock is held and the other guards are of the same kind (same
        // values); the tests that read these variables hold a guard.
        unsafe {
            match value {
                Some(v) => std::env::set_var(&name, v),
                None => std::env::remove_var(&name),
            }
        }
    }
    *state = Some(Active {
        key: key.into(),
        shared,
        users: 1,
        saved,
        scratch: scratch.clone(),
    });
    EnvGuard { scratch }
}

impl EnvGuard {
    /// The scratch folder that is `HOME` and the base of the `XDG_*` folders; removed when
    /// the last guard of the kind drops.
    pub fn scratch(&self) -> &Path {
        &self.scratch
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        let mut state = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(active) = state.as_mut() else { return };
        active.users -= 1;
        if active.users > 0 {
            return;
        }
        let active = state.take().expect("checked above");
        for (name, old) in active.saved.into_iter().rev() {
            // SAFETY: under the state lock, with no other guard alive.
            unsafe {
                match old {
                    Some(v) => std::env::set_var(&name, v),
                    None => std::env::remove_var(&name),
                }
            }
        }
        let _ = std::fs::remove_dir_all(&active.scratch);
        FREE.notify_all();
    }
}

/// The guard of a hermetic test: `PDK`, `PDK_ROOT`, `PDKPATH` and the other variables that
/// pick a PDK are unset, `HOME` and `XDG_{CONFIG,CACHE,DATA,STATE}_HOME` point to a scratch
/// folder. The tool variables (`CDEPT_XSCHEM`, ...) stay as they are, as tests that need a
/// tool skip when it is absent; see [`hermetic_env_without_tools`].
pub fn hermetic_env() -> EnvGuard {
    acquire("hermetic", true, Vec::new())
}

/// [`hermetic_env`] and the `CDEPT_*` variables that name the tools (xschem, KLayout,
/// ngspice, ...) unset too, for tests of what happens when a tool is absent.
pub fn hermetic_env_without_tools() -> EnvGuard {
    acquire(
        "hermetic-without-tools",
        true,
        TOOL_VARS.iter().map(|n| (n.to_string(), None)).collect(),
    )
}

/// [`hermetic_env`] with more variables set (`Some`) or unset (`None`); the guard is alone
/// while it lives.
pub fn hermetic_env_with(vars: &[(&str, Option<&OsStr>)]) -> EnvGuard {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let vars = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.map(OsString::from)))
        .collect();
    acquire(&format!("custom-{n}"), false, vars)
}

/// An installed PDK for a real-PDK test, with the environment set explicitly while it lives.
pub struct RealPdk {
    /// The PDK's name, as in `PDK`.
    pub name: String,
    /// The folder with the PDKs: `PDK_ROOT`.
    pub root: PathBuf,
    /// `root/name`: `PDKPATH`.
    pub dir: PathBuf,
    _env: EnvGuard,
}

/// The folder the real PDKs are looked for in: `$CDEPT_TEST_PDK_ROOT`, else
/// [`DEFAULT_PDK_ROOT`].
pub fn test_pdk_root() -> PathBuf {
    std::env::var_os("CDEPT_TEST_PDK_ROOT")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_PDK_ROOT), PathBuf::from)
}

/// The installed PDK `name`, or `None` after printing `skipped: <name> not installed`. While
/// the result lives, `PDK`, `PDK_ROOT` and `PDKPATH` are set to it (whatever the shell has),
/// `HOME` and `XDG_*` are scratch folders, and the tool variables are the shell's.
pub fn real_pdk(name: &str) -> Option<RealPdk> {
    let root = test_pdk_root();
    let dir = root.join(name);
    if !dir.is_dir() {
        eprintln!(
            "skipped: {name} not installed (looked in {})",
            root.display()
        );
        return None;
    }
    let vars = vec![
        ("PDK".to_string(), Some(OsString::from(name))),
        ("PDK_ROOT".to_string(), Some(root.clone().into_os_string())),
        ("PDKPATH".to_string(), Some(dir.clone().into_os_string())),
    ];
    let env = acquire(&format!("real-{name}"), true, vars);
    Some(RealPdk {
        name: name.to_string(),
        root,
        dir,
        _env: env,
    })
}
