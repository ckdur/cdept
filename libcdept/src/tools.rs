//! Detection of the external tools CDePT orchestrates.
//!
//! Phase 0 only finds binaries on `PATH` and asks them for a version string. The
//! `ToolAdapter` trait and process supervisor arrive in Phase 3.

use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Static description of an external tool.
#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    /// What the tool is used for in CDePT.
    pub role: &'static str,
    /// Candidate executable names, first match wins.
    pub binaries: &'static [&'static str],
    /// Arguments that print a version and exit. Empty: don't probe (GUI-only tools).
    pub version_args: &'static [&'static str],
}

pub const KNOWN_TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "xschem",
        role: "Schematic",
        binaries: &["xschem"],
        version_args: &["-v"],
    },
    ToolSpec {
        name: "Qucs-S",
        role: "Schematic",
        binaries: &["qucs-s"],
        version_args: &[],
    },
    ToolSpec {
        name: "KLayout",
        role: "Layout",
        binaries: &["klayout"],
        version_args: &["-v"],
    },
    ToolSpec {
        name: "Magic",
        role: "Layout",
        binaries: &["magic"],
        version_args: &["--version"],
    },
    ToolSpec {
        name: "ngspice",
        role: "Simulation",
        binaries: &["ngspice"],
        version_args: &["-v"],
    },
    ToolSpec {
        name: "Xyce",
        role: "Simulation",
        binaries: &["Xyce", "xyce"],
        version_args: &["-v"],
    },
    ToolSpec {
        name: "netgen",
        role: "LVS",
        binaries: &["netgen-lvs", "netgen"],
        version_args: &["-batch"],
    },
    ToolSpec {
        name: "Yosys",
        role: "Synthesis",
        binaries: &["yosys"],
        version_args: &["-V"],
    },
    ToolSpec {
        name: "OpenROAD",
        role: "Digital P&R",
        binaries: &["openroad"],
        version_args: &["-version"],
    },
    ToolSpec {
        name: "Verilator",
        role: "Digital sim",
        binaries: &["verilator"],
        version_args: &["--version"],
    },
    ToolSpec {
        name: "Icarus",
        role: "Digital sim",
        binaries: &["iverilog"],
        version_args: &["-V"],
    },
    ToolSpec {
        name: "Surfer",
        role: "Waveforms",
        binaries: &["surfer"],
        version_args: &["--version"],
    },
    ToolSpec {
        name: "GTKWave",
        role: "Waveforms",
        binaries: &["gtkwave"],
        version_args: &["--version"],
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

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

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
    let path = spec.binaries.iter().find_map(|b| find_in_path(b));
    let version = match &path {
        Some(p) if !spec.version_args.is_empty() => probe_version(p, spec.version_args),
        _ => None,
    };
    tracing::debug!(tool = spec.name, ?path, ?version, "tool detection");
    ToolInfo {
        spec: *spec,
        path,
        version,
    }
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
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < VERSION_TIMEOUT => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(tool = %path.display(), "version probe timed out");
                break;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut out);
    }
    extract_version(&out)
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

    #[cfg(unix)]
    #[test]
    fn finds_sh_on_path() {
        assert!(find_in_path("sh").is_some());
        assert!(find_in_path("definitely-not-a-cdept-tool").is_none());
    }
}
