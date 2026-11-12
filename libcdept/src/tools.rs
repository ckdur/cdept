//! Detection of the external tools CDePT orchestrates.
//!
//! Phase 0 only finds tools (binaries on `PATH`, Python packages, KLayout salt
//! packages) and asks them for a version string. The `ToolAdapter` trait and process
//! supervisor arrive in Phase 3.

use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How to find a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locate {
    /// Executable on `PATH`; the first name found wins.
    Bin(&'static [&'static str]),
    /// Python package importable by the detected interpreter. `dist` is the
    /// distribution name used for the version (`importlib.metadata`).
    Python {
        module: &'static str,
        dist: &'static str,
    },
    /// KLayout package ("salt grain") directory, e.g. `<prefix>/share/klayout/salt/<name>`.
    KlayoutSalt(&'static str),
}

/// Static description of an external tool.
#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    /// What the tool is used for in CDePT.
    pub role: &'static str,
    /// Roadmap phase that integrates it (`docs/ROADMAP.md`); `None`: detected only.
    pub phase: Option<u8>,
    pub locate: Locate,
    /// Arguments that print a version and exit. Empty: don't probe (GUI-only tools,
    /// or tools without a usable version flag).
    pub version_args: &'static [&'static str],
    pub note: Option<&'static str>,
}

/// Tools CDePT knows how to find. The list follows the tools of the
/// `factory.symbioticeda.com/asic-all:dev` image that matter to an analog/mixed-signal
/// design manager (see "External tools" in `docs/ROADMAP.md` for what is left out);
/// version flags and binary names were checked in that image.
pub const KNOWN_TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "xschem",
        role: "Schematic",
        phase: Some(3),
        locate: Locate::Bin(&["xschem"]),
        version_args: &["-v"],
        note: None,
    },
    ToolSpec {
        name: "Qucs-S",
        role: "Schematic",
        phase: Some(3),
        locate: Locate::Bin(&["qucs-s"]),
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "XCircuit",
        role: "Schematic",
        phase: None,
        locate: Locate::Bin(&["xcircuit"]),
        version_args: &[],
        note: Some("import/view only"),
    },
    ToolSpec {
        name: "KLayout",
        role: "Layout",
        phase: Some(3),
        locate: Locate::Bin(&["klayout"]),
        version_args: &["-v"],
        note: None,
    },
    ToolSpec {
        name: "KLayout (Python)",
        role: "Layout batch",
        phase: Some(3),
        locate: Locate::Python {
            module: "klayout",
            dist: "klayout",
        },
        version_args: &[],
        note: Some("pip `klayout`: batch DRC/LVS/scripts without the GUI"),
    },
    ToolSpec {
        name: "Magic",
        role: "Layout",
        phase: Some(3),
        locate: Locate::Bin(&["magic"]),
        version_args: &["--version"],
        note: None,
    },
    ToolSpec {
        name: "klive",
        role: "KLayout plugin",
        phase: Some(3),
        locate: Locate::KlayoutSalt("klive"),
        version_args: &[],
        note: Some("reload a GDS in a running KLayout via JSON on port 8082"),
    },
    ToolSpec {
        name: "KLayout Prod. Suite",
        role: "KLayout plugin",
        phase: Some(6),
        locate: Locate::KlayoutSalt("klayout-productivity-suite"),
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "xsection",
        role: "KLayout plugin",
        phase: Some(7),
        locate: Locate::KlayoutSalt("xsection"),
        version_args: &[],
        note: Some("process cross-sections"),
    },
    ToolSpec {
        name: "gdsfactory",
        role: "Layout (Py)",
        phase: Some(6),
        locate: Locate::Python {
            module: "gdsfactory",
            dist: "gdsfactory",
        },
        version_args: &[],
        note: Some("PCells / layout generators"),
    },
    ToolSpec {
        name: "GDS3D",
        role: "3D viewer",
        phase: Some(7),
        locate: Locate::Bin(&["GDS3D"]),
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "ngspice",
        role: "Simulation",
        phase: Some(3),
        locate: Locate::Bin(&["ngspice"]),
        version_args: &["-v"],
        note: None,
    },
    ToolSpec {
        name: "Xyce",
        role: "Simulation",
        phase: Some(4),
        locate: Locate::Bin(&["Xyce", "xyce"]),
        version_args: &["-v"],
        note: None,
    },
    ToolSpec {
        name: "VACASK",
        role: "Simulation",
        phase: Some(4),
        locate: Locate::Bin(&["vacask"]),
        version_args: &["-v"],
        note: None,
    },
    ToolSpec {
        name: "OpenVAF",
        role: "Verilog-A",
        phase: Some(4),
        locate: Locate::Bin(&["openvaf-r", "openvaf"]),
        version_args: &[],
        note: Some("compiles Verilog-A models (OSDI) for ngspice/VACASK"),
    },
    ToolSpec {
        name: "PySpice",
        role: "Sim (Py)",
        phase: None,
        locate: Locate::Python {
            module: "PySpice",
            dist: "PySpice",
        },
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "spicelib",
        role: "Sim (Py)",
        phase: Some(5),
        locate: Locate::Python {
            module: "spicelib",
            dist: "spicelib",
        },
        version_args: &[],
        note: Some("raw-file reader, reference for our parser"),
    },
    ToolSpec {
        name: "pygmid",
        role: "Sizing (Py)",
        phase: Some(4),
        locate: Locate::Python {
            module: "pygmid",
            dist: "pygmid",
        },
        version_args: &[],
        note: Some("gm/Id lookup-table sizing"),
    },
    ToolSpec {
        name: "CACE",
        role: "Characterize",
        phase: Some(4),
        locate: Locate::Bin(&["cace"]),
        version_args: &["--version"],
        note: Some("datasheet-driven characterisation; compare with ADE-lite specs"),
    },
    ToolSpec {
        name: "Verilator",
        role: "Digital sim",
        phase: Some(8),
        locate: Locate::Bin(&["verilator"]),
        version_args: &["--version"],
        note: None,
    },
    ToolSpec {
        name: "Icarus",
        role: "Digital sim",
        phase: Some(8),
        locate: Locate::Bin(&["iverilog"]),
        version_args: &["-V"],
        note: None,
    },
    ToolSpec {
        name: "NVC",
        role: "VHDL sim",
        phase: Some(8),
        locate: Locate::Bin(&["nvc"]),
        version_args: &["--version"],
        note: None,
    },
    ToolSpec {
        name: "cocotb",
        role: "Testbench (Py)",
        phase: Some(8),
        locate: Locate::Python {
            module: "cocotb",
            dist: "cocotb",
        },
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "spicebind",
        role: "Co-sim (Py)",
        phase: Some(8),
        locate: Locate::Python {
            module: "spicebind",
            dist: "spicebind",
        },
        version_args: &[],
        note: Some("ngspice inside an HDL simulator, via cocotb"),
    },
    ToolSpec {
        name: "Yosys",
        role: "Synthesis",
        phase: Some(8),
        locate: Locate::Bin(&["yosys"]),
        version_args: &["-V"],
        note: None,
    },
    ToolSpec {
        name: "OpenROAD",
        role: "Digital P&R",
        phase: Some(8),
        locate: Locate::Bin(&["openroad"]),
        version_args: &["-version"],
        note: None,
    },
    ToolSpec {
        name: "OpenSTA",
        role: "Timing",
        phase: Some(8),
        locate: Locate::Bin(&["sta"]),
        version_args: &["-version"],
        note: None,
    },
    ToolSpec {
        name: "LibreLane",
        role: "Digital flow",
        phase: Some(8),
        locate: Locate::Bin(&["librelane"]),
        version_args: &["--version"],
        note: None,
    },
    ToolSpec {
        name: "SiliconCompiler",
        role: "Digital flow",
        phase: Some(8),
        locate: Locate::Python {
            module: "siliconcompiler",
            dist: "siliconcompiler",
        },
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "CharLib",
        role: "Liberty char.",
        phase: Some(8),
        locate: Locate::Python {
            module: "charlib",
            dist: "CharLib",
        },
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "lctime",
        role: "Liberty char.",
        phase: Some(8),
        locate: Locate::Python {
            module: "lctime",
            dist: "lctime",
        },
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "padring",
        role: "Pad ring",
        phase: Some(8),
        locate: Locate::Bin(&["padring"]),
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "OpenRAM",
        role: "Memory macro",
        phase: Some(8),
        locate: Locate::Python {
            module: "openram",
            dist: "openram",
        },
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "netgen",
        role: "LVS",
        phase: Some(7),
        locate: Locate::Bin(&["netgen-lvs", "netgen"]),
        version_args: &["-batch"],
        note: None,
    },
    ToolSpec {
        name: "CVC",
        role: "ERC",
        phase: Some(7),
        locate: Locate::Bin(&["cvc_rv", "cvc"]),
        version_args: &["-v"],
        note: Some("circuit validity check (analog ERC)"),
    },
    ToolSpec {
        name: "KLayout-PEX",
        role: "PEX",
        phase: Some(7),
        locate: Locate::Python {
            module: "klayout_pex",
            dist: "klayout-pex",
        },
        version_args: &[],
        note: Some("CLI is `kpex`"),
    },
    ToolSpec {
        name: "FastHenry2",
        role: "Inductance ext.",
        phase: Some(7),
        locate: Locate::Bin(&["fasthenry"]),
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "FasterCap",
        role: "Capacitance ext.",
        phase: Some(7),
        locate: Locate::Bin(&["FasterCap"]),
        version_args: &[],
        note: None,
    },
    ToolSpec {
        name: "gdsfill",
        role: "Fill",
        phase: Some(7),
        locate: Locate::Python {
            module: "gdsfill",
            dist: "gdsfill",
        },
        version_args: &[],
        note: Some("IHP metal fill"),
    },
    ToolSpec {
        name: "openEMS",
        role: "EM solver",
        phase: None,
        locate: Locate::Bin(&["openEMS"]),
        version_args: &["--help"],
        note: Some("with openems-scripts-ihp-muehlhaus"),
    },
    ToolSpec {
        name: "scikit-rf",
        role: "RF (Py)",
        phase: None,
        locate: Locate::Python {
            module: "skrf",
            dist: "scikit-rf",
        },
        version_args: &[],
        note: Some("S-parameter / Touchstone analysis"),
    },
    ToolSpec {
        name: "UberWave",
        role: "Waveforms",
        phase: Some(5),
        locate: Locate::Bin(&["UberWaveViewer", "uberwave"]),
        version_args: &[],
        note: Some("large ngspice raw / Spectre PSF; `UberWaveViewer <file> [--analysis <name>]`"),
    },
    ToolSpec {
        name: "Surfer",
        role: "Waveforms",
        phase: Some(5),
        locate: Locate::Bin(&["surfer"]),
        version_args: &["--version"],
        note: None,
    },
    ToolSpec {
        name: "GTKWave",
        role: "Waveforms",
        phase: Some(5),
        locate: Locate::Bin(&["gtkwave"]),
        version_args: &[],
        note: Some("`--version` prints the help, so no probe"),
    },
    ToolSpec {
        name: "gaw",
        role: "Waveforms",
        phase: Some(5),
        locate: Locate::Bin(&["gaw"]),
        version_args: &[],
        note: Some("xschem's waveform viewer; probing writes ~/.gaw"),
    },
    ToolSpec {
        name: "LibMan",
        role: "Library mgr",
        phase: Some(1),
        locate: Locate::Bin(&["libman", "LibMan"]),
        version_args: &[],
        note: Some(
            "IHP-GmbH/LibMan, a Qt library manager over xschem/KLayout: reference for Phase 1, open-with interop",
        ),
    },
    ToolSpec {
        name: "ciel",
        role: "PDK manager",
        phase: Some(2),
        locate: Locate::Bin(&["ciel"]),
        version_args: &["--version"],
        note: Some("fetches/enables open_pdks versions under $PDK_ROOT"),
    },
    ToolSpec {
        name: "Python",
        role: "Scripting",
        phase: Some(3),
        locate: Locate::Bin(&["python3", "python"]),
        version_args: &["--version"],
        note: Some("needed to detect and run the Python tools"),
    },
];

/// Result of detecting one tool.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub spec: ToolSpec,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

impl ToolInfo {
    pub fn found(&self) -> bool {
        self.path.is_some()
    }
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Detect all [`KNOWN_TOOLS`], probing versions in parallel.
pub fn detect_all() -> Vec<ToolInfo> {
    let handles: Vec<_> = KNOWN_TOOLS
        .iter()
        .map(|spec| thread::spawn(move || detect(spec)))
        .collect();
    handles
        .into_iter()
        .zip(KNOWN_TOOLS)
        .map(|(h, spec)| {
            h.join().unwrap_or(ToolInfo {
                spec: *spec,
                path: None,
                version: None,
            })
        })
        .collect()
}

pub fn detect(spec: &ToolSpec) -> ToolInfo {
    let (path, version) = match spec.locate {
        Locate::Bin(names) => {
            let path = names.iter().find_map(|b| find_in_path(b));
            let version = match &path {
                Some(p) if !spec.version_args.is_empty() => probe_version(p, spec.version_args),
                _ => None,
            };
            (path, version)
        }
        Locate::Python { module, dist } => probe_python(module, dist),
        Locate::KlayoutSalt(grain) => find_salt(grain)
            .map(|dir| {
                let version = salt_version(&dir);
                (Some(dir), version)
            })
            .unwrap_or_default(),
    };
    tracing::debug!(tool = spec.name, ?path, ?version, "tool detection");
    ToolInfo {
        spec: *spec,
        path,
        version,
    }
}

/// The Python interpreter used for Python tools: `$CDEPT_PYTHON`, else `python3` on PATH.
pub fn python() -> Option<PathBuf> {
    match env::var_os("CDEPT_PYTHON") {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => find_in_path("python3").or_else(|| find_in_path("python")),
    }
}

/// Locate a module without importing it, then read its distribution version.
const PY_PROBE: &str = "\
import sys, importlib.util as u, importlib.metadata as m
s = u.find_spec(sys.argv[1])
if s is None: sys.exit(1)
print(s.origin or next(iter(s.submodule_search_locations or []), ''))
try: print(m.version(sys.argv[2]))
except Exception: print('')
";

fn probe_python(module: &str, dist: &str) -> (Option<PathBuf>, Option<String>) {
    let Some(python) = python() else {
        return (None, None);
    };
    let Some(out) = run_with_timeout(&python, &["-c", PY_PROBE, module, dist], false) else {
        return (None, None);
    };
    let mut lines = out.lines();
    let path = lines.next().filter(|l| !l.is_empty()).map(PathBuf::from);
    let version = lines.next().map(str::trim).filter(|v| !v.is_empty());
    match path {
        Some(p) => (Some(p), version.map(|v| format!("{module} {v}"))),
        None => (None, None),
    }
}

/// KLayout salt directories, in KLayout's own search order.
fn salt_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("KLAYOUT_HOME") {
        dirs.push(PathBuf::from(home).join("salt"));
    } else if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".klayout/salt"));
    }
    if let Some(path) = env::var_os("KLAYOUT_PATH") {
        dirs.extend(env::split_paths(&path).map(|p| p.join("salt")));
    }
    // System-wide packages next to the klayout binary (`<prefix>/bin/klayout`).
    if let Some(prefix) = find_in_path("klayout")
        .as_deref()
        .and_then(Path::parent)
        .and_then(Path::parent)
    {
        dirs.push(prefix.join("share/klayout/salt"));
    }
    dirs.push("/usr/local/share/klayout/salt".into());
    dirs.push("/usr/share/klayout/salt".into());
    dirs
}

fn find_salt(grain: &str) -> Option<PathBuf> {
    salt_dirs()
        .into_iter()
        .map(|d| d.join(grain))
        .find(|d| d.is_dir())
}

/// `<version>` of a grain's `grain.xml`.
fn salt_version(dir: &Path) -> Option<String> {
    let xml = std::fs::read_to_string(dir.join("grain.xml")).ok()?;
    let start = xml.find("<version>")? + "<version>".len();
    let end = start + xml[start..].find("</version>")?;
    let v = xml[start..end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Find an executable on `PATH`.
pub fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(binary))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Run `path args...` with a timeout and extract a version line from its output.
fn probe_version(path: &Path, args: &[&str]) -> Option<String> {
    extract_version(&run_with_timeout(path, args, true)?)
}

/// Run `path args...`, killing it after [`PROBE_TIMEOUT`]. Returns stdout (plus stderr
/// if `with_stderr`), or `None` if it could not start, timed out, or failed
/// (a failure only counts when stderr is not wanted, i.e. for exact probes).
fn run_with_timeout(path: &Path, args: &[&str], with_stderr: bool) -> Option<String> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if with_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .ok()?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < PROBE_TIMEOUT => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(tool = %path.display(), "probe timed out");
                break None;
            }
        }
    };
    if !with_stderr && !status?.success() {
        return None;
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut out);
    }
    Some(out)
}

/// First output line that contains a digit, trimmed of banner decoration.
pub fn extract_version(output: &str) -> Option<String> {
    let line = output
        .lines()
        .map(|l| l.trim_matches(|c: char| c == '*' || c.is_whitespace()))
        .find(|l| l.chars().any(|c| c.is_ascii_digit()))?;
    const MAX: usize = 60;
    Some(if line.chars().count() > MAX {
        format!("{}…", line.chars().take(MAX).collect::<String>())
    } else {
        line.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_banner() {
        let ngspice = "******\n** ngspice-42 : Circuit level simulation program\n******\n";
        assert_eq!(
            extract_version(ngspice).as_deref(),
            Some("ngspice-42 : Circuit level simulation program")
        );
        assert_eq!(
            extract_version("KLayout 0.29.8\n").as_deref(),
            Some("KLayout 0.29.8")
        );
        assert_eq!(extract_version("no digits here\n"), None);
    }

    #[test]
    fn long_lines_are_truncated() {
        let long = format!("v1 {}", "x".repeat(100));
        let v = extract_version(&long).unwrap();
        assert_eq!(v.chars().count(), 61);
        assert!(v.ends_with('…'));
    }

    #[test]
    fn registry_is_consistent() {
        let mut names = std::collections::HashSet::new();
        for t in KNOWN_TOOLS {
            assert!(names.insert(t.name), "duplicate tool {}", t.name);
            assert!(
                t.role.len() <= 16,
                "role too long for the tables: {}",
                t.role
            );
            match t.locate {
                Locate::Bin(bins) => assert!(!bins.is_empty(), "{}", t.name),
                Locate::Python { module, dist } => {
                    assert!(!module.is_empty() && !dist.is_empty(), "{}", t.name);
                    assert!(
                        t.version_args.is_empty(),
                        "{}: version comes from metadata",
                        t.name
                    );
                }
                Locate::KlayoutSalt(g) => assert!(!g.is_empty(), "{}", t.name),
            }
            assert!(t.phase.is_none_or(|p| p <= 9), "{}", t.name);
        }
    }

    #[test]
    fn salt_version_from_grain_xml() {
        let dir = std::env::temp_dir().join(format!("cdept-salt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("grain.xml"),
            "<salt-grain><name>klive</name><version> 0.4.3 </version></salt-grain>",
        )
        .unwrap();
        assert_eq!(salt_version(&dir).as_deref(), Some("0.4.3"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn finds_sh_on_path() {
        assert!(find_in_path("sh").is_some());
        assert!(find_in_path("definitely-not-a-cdept-tool").is_none());
    }
}
