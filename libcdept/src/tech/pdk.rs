//! Shared types of the technology snapshot: a PDK folder, portable paths, what a
//! section scan gets and returns.
//!
//! This file is the contract between the section modules. Add to it, do not change
//! what is here without telling the others.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The PDK families CDePT knows the quirks of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    Sky130,
    Gf180mcu,
    IhpSg13g2,
    IhpSg13cmos5l,
    #[default]
    Other,
}

impl Family {
    /// From the PDK name alone (`sky130A`, `gf180mcuD`, `ihp-sg13g2`, …).
    pub fn of_name(name: &str) -> Family {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("sky130") {
            Family::Sky130
        } else if lower.starts_with("gf180mcu") {
            Family::Gf180mcu
        } else if lower.starts_with("ihp-sg13cmos5l") {
            Family::IhpSg13cmos5l
        } else if lower.starts_with("ihp-sg13g2") {
            Family::IhpSg13g2
        } else {
            Family::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Family::Sky130 => "sky130",
            Family::Gf180mcu => "gf180mcu",
            Family::IhpSg13g2 => "ihp-sg13g2",
            Family::IhpSg13cmos5l => "ihp-sg13cmos5l",
            Family::Other => "other",
        }
    }
}

/// One installed PDK: `<root>/<name>`, where `root` is what `$PDK_ROOT` means and
/// `name` what `$PDK` means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdkDir {
    pub root: PathBuf,
    pub name: String,
    pub family: Family,
}

impl PdkDir {
    pub fn new(root: impl Into<PathBuf>, name: impl Into<String>) -> PdkDir {
        let name = name.into();
        PdkDir {
            root: root.into(),
            family: Family::of_name(&name),
            name,
        }
    }

    /// `<root>/<name>`.
    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.name)
    }

    /// `<root>/<name>/libs.tech/<tool>`.
    pub fn tech(&self, tool: &str) -> PathBuf {
        self.dir().join("libs.tech").join(tool)
    }

    /// `<root>/<name>/libs.ref`.
    pub fn refs(&self) -> PathBuf {
        self.dir().join("libs.ref")
    }

    /// A path as the snapshot stores it: below the PDK root it reads
    /// `$PDK_ROOT/<name>/…`, so the snapshot works on a machine with the PDKs
    /// elsewhere; anything else stays absolute.
    pub fn portable(&self, path: &Path) -> TechPath {
        match path.strip_prefix(&self.root) {
            Ok(rest) if rest.as_os_str().is_empty() => TechPath("$PDK_ROOT".into()),
            Ok(rest) => TechPath(format!("$PDK_ROOT/{}", rest.to_string_lossy())),
            Err(_) => TechPath(path.to_string_lossy().into_owned()),
        }
    }
}

/// A path as written in the snapshot: `$PDK_ROOT/ihp-sg13g2/libs.tech/…`, or absolute.
/// `$VAR`, `${VAR}` and a leading `~` are expanded like in `cdept.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TechPath(pub String);

impl TechPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path on this machine. `var` gives the value of a variable (`PDK_ROOT`,
    /// `PDK`, `HOME`, …), e.g. `|n| project.var(n)`.
    pub fn resolve(&self, var: impl Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
        crate::project::expand(&self.0, var).map(PathBuf::from)
    }

    /// The path on this machine for a PDK folder (only `$PDK_ROOT` and `$PDK` are
    /// known; for scans and tests).
    pub fn resolve_in(&self, pdk: &PdkDir) -> Result<PathBuf, String> {
        self.resolve(|name| match name {
            "PDK_ROOT" => Some(pdk.root.to_string_lossy().into_owned()),
            "PDK" => Some(pdk.name.clone()),
            _ => std::env::var(name).ok(),
        })
    }
}

impl std::fmt::Display for TechPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What is known about the PDK itself (`[pdk]` in `tech.toml`). Filled in by
/// [`super::discover::info`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdkInfo {
    /// `$PDK`: `ihp-sg13g2`, `sky130A`.
    pub name: String,
    pub family: Family,
    /// A version, tag or commit of the PDK, when it says one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// How it was installed: `open_pdks`, `ciel`, `git`, `folder`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The tools the PDK has a `libs.tech/<tool>` folder for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// The branch of a PDK that is a git checkout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Tool versions the PDK says it was built or tested with (tool → version).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tested_with: std::collections::BTreeMap<String, String>,
}

/// A library the PDK brings, as the project will list it (read-only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TechLibrary {
    pub name: String,
    pub path: TechPath,
    /// A [`crate::LibraryLayout`] name: `flat`, `cells`, …
    pub layout: String,
    /// `primitives`, `stdcells`, `io`, `memory`, `tests`, `examples`, `other`.
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Listed in a new project. Tests and examples are found but left out.
    pub enabled: bool,
}

/// What a section scan gets.
#[derive(Debug, Clone)]
pub struct ScanContext {
    pub pdk: PdkDir,
    /// The scan may start the helper programs (KLayout to list PCells, xschem to
    /// evaluate the `xschemrc`). Without them it reads files only.
    pub helpers: bool,
    /// Environment for programs the scan starts (`PDK_ROOT`, `PDK`, …).
    pub env: Vec<(OsString, OsString)>,
}

impl ScanContext {
    pub fn new(pdk: PdkDir) -> ScanContext {
        let env = vec![
            ("PDK_ROOT".into(), pdk.root.clone().into_os_string()),
            ("PDK".into(), pdk.name.clone().into()),
        ];
        ScanContext {
            pdk,
            helpers: true,
            env,
        }
    }

    pub fn files_only(mut self) -> ScanContext {
        self.helpers = false;
        self
    }
}

/// What a section scan returns: its part of `tech.toml` (small, hand-editable), its
/// part of the catalog (bulky lists: layers, PCells, symbols, cells) and notes for
/// the user (what was not found, what was guessed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scanned<S, C> {
    pub section: S,
    pub catalog: C,
    pub notes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_below_the_root_are_portable() {
        let pdk = PdkDir::new("/opt/pdks", "ihp-sg13g2");
        assert_eq!(pdk.family, Family::IhpSg13g2);
        let p = pdk.portable(&pdk.tech("klayout").join("tech/sg13g2.lyt"));
        assert_eq!(
            p.as_str(),
            "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyt"
        );
        assert_eq!(
            p.resolve_in(&pdk).unwrap(),
            PathBuf::from("/opt/pdks/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyt")
        );
        assert_eq!(
            pdk.portable(Path::new("/usr/share/x")).as_str(),
            "/usr/share/x"
        );
    }

    #[test]
    fn families_by_name() {
        assert_eq!(Family::of_name("sky130B"), Family::Sky130);
        assert_eq!(Family::of_name("gf180mcuD"), Family::Gf180mcu);
        assert_eq!(Family::of_name("ihp-sg13cmos5l"), Family::IhpSg13cmos5l);
        assert_eq!(Family::of_name("xh018"), Family::Other);
    }
}
