//! ciel (formerly volare): installed PDK versions under a PDK root.
//!
//! ciel keeps every version it fetched and links the one in use into the root:
//!
//! ```text
//! <root>/ciel/<family>/versions/<version>/<pdk>/   a fetched version (sky130A, sky130B)
//! <root>/ciel/<family>/current                     the version in use (the text of it)
//! <root>/<pdk> -> ciel/<family>/versions/<version>/<pdk>
//! ```
//!
//! (`volare` instead of `ciel` for installs made by its predecessor.) A version is the
//! commit of open_pdks it was built from. CDePT reads this layout itself, so listing
//! works without the program; switching to a version that is already there is the same
//! relinking ciel does ([`enable`]). Fetching and listing what can be fetched needs the
//! program ([`Program`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use eyre::{Context, Result, bail, eyre};

/// The folders ciel and volare keep their versions in, inside a root.
pub const STORES: &[&str] = &["ciel", "volare"];

/// One fetched version of a PDK family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// `ciel` or `volare`.
    pub store: String,
    /// `sky130`, `gf180mcu`, `ihp-sg13g2`.
    pub family: String,
    /// The open_pdks commit the version was built from.
    pub version: String,
    /// `<root>/ciel/<family>/versions/<version>`.
    pub path: PathBuf,
    /// The PDKs in it (`sky130A`, `sky130B`).
    pub pdks: Vec<String>,
    /// It is the one linked into the root.
    pub current: bool,
}

/// The version a PDK folder belongs to, from its real path
/// (`…/ciel/sky130/versions/<version>/sky130A`). `pdks` and `current` are not set.
pub fn version_of(real: &Path) -> Option<Version> {
    let version_dir = real.parent()?;
    let versions = version_dir.parent()?;
    if versions.file_name()? != "versions" {
        return None;
    }
    let family_dir = versions.parent()?;
    let store = family_dir.parent()?.file_name()?.to_str()?;
    if !STORES.contains(&store) {
        return None;
    }
    Some(Version {
        store: store.to_string(),
        family: family_dir.file_name()?.to_str()?.to_string(),
        version: version_dir.file_name()?.to_str()?.to_string(),
        path: version_dir.to_path_buf(),
        pdks: Vec::new(),
        current: false,
    })
}

fn subfolders(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// The families with fetched versions under `root`.
pub fn families(root: &Path) -> Vec<String> {
    let mut all: Vec<String> = STORES
        .iter()
        .flat_map(|store| subfolders(&root.join(store)))
        .collect();
    all.sort();
    all.dedup();
    all
}

/// The version of `family` in use under `root`: what the PDK links in the root point
/// at, else what the `current` file says.
pub fn current(root: &Path, family: &str) -> Option<String> {
    for store in STORES {
        let family_dir = root.join(store).join(family);
        let versions = family_dir.join("versions");
        // The links are what the tools follow, so they have the last word.
        for version in subfolders(&versions) {
            for pdk in subfolders(&versions.join(&version)) {
                let link = root.join(&pdk);
                if !link.is_symlink() {
                    continue;
                }
                if let Some(found) = fs::canonicalize(&link)
                    .ok()
                    .as_deref()
                    .and_then(version_of)
                    .filter(|v| v.family == family)
                {
                    return Some(found.version);
                }
            }
        }
        if let Ok(text) = fs::read_to_string(family_dir.join("current")) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// The fetched versions under `root`, of one family or of all, sorted by family and
/// version.
pub fn installed(root: &Path, family: Option<&str>) -> Vec<Version> {
    let mut all = Vec::new();
    for store in STORES {
        for found in subfolders(&root.join(store)) {
            if family.is_some_and(|wanted| wanted != found) {
                continue;
            }
            let in_use = current(root, &found);
            let versions = root.join(store).join(&found).join("versions");
            for version in subfolders(&versions) {
                let path = versions.join(&version);
                all.push(Version {
                    store: store.to_string(),
                    family: found.clone(),
                    current: in_use.as_deref() == Some(version.as_str()),
                    pdks: subfolders(&path),
                    version,
                    path,
                });
            }
        }
    }
    all.sort_by(|a, b| (&a.family, &a.version).cmp(&(&b.family, &b.version)));
    all
}

/// The fetched version `wanted` names: the whole text or the start of it, when only
/// one version starts so.
pub fn resolve(root: &Path, family: &str, wanted: &str) -> Result<Version> {
    let versions = installed(root, Some(family));
    if versions.is_empty() {
        bail!(
            "no version of {family} is installed under {} (ciel keeps them in {}/ciel/{family}/versions)",
            root.display(),
            root.display()
        );
    }
    let matching: Vec<&Version> = versions
        .iter()
        .filter(|v| v.version.starts_with(wanted))
        .collect();
    match matching.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(eyre!(
            "version {wanted} of {family} is not installed under {}; installed: {}",
            root.display(),
            versions
                .iter()
                .map(|v| v.version.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        several => Err(eyre!(
            "{wanted} is the start of {} installed versions of {family}",
            several.len()
        )),
    }
}

/// Put a fetched version in use, the way `ciel enable` does: every PDK of the version
/// gets a relative link in the root (an older link is replaced; a real folder in the
/// way is an error and nothing is changed), and `current` gets the version. Returns
/// the PDKs linked.
#[cfg(unix)]
pub fn enable(root: &Path, family: &str, wanted: &str) -> Result<Vec<String>> {
    let version = resolve(root, family, wanted)?;
    if version.pdks.is_empty() {
        bail!("{} holds no PDK", version.path.display());
    }
    for pdk in &version.pdks {
        let link = root.join(pdk);
        if link.exists() && !link.is_symlink() {
            bail!(
                "{} is a folder, not a link to a version: not replacing it",
                link.display()
            );
        }
    }
    let family_dir = root.join(&version.store).join(family);
    for pdk in &version.pdks {
        let link = root.join(pdk);
        if link.is_symlink() {
            fs::remove_file(&link).wrap_err_with(|| format!("cannot remove {}", link.display()))?;
        }
        let target = Path::new(&version.store)
            .join(family)
            .join("versions")
            .join(&version.version)
            .join(pdk);
        std::os::unix::fs::symlink(&target, &link)
            .wrap_err_with(|| format!("cannot link {}", link.display()))?;
    }
    fs::write(family_dir.join("current"), &version.version)
        .wrap_err_with(|| format!("cannot write {}", family_dir.join("current").display()))?;
    Ok(version.pdks)
}

#[cfg(not(unix))]
pub fn enable(_root: &Path, _family: &str, _wanted: &str) -> Result<Vec<String>> {
    bail!("switching PDK versions needs symbolic links; use `ciel enable`")
}

/// The ciel (or volare) program, when it is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub path: PathBuf,
    /// `ciel` or `volare`: they name the family option differently.
    pub name: &'static str,
}

impl Program {
    /// `ciel` on `PATH`, else `volare`.
    pub fn find() -> Option<Program> {
        ["ciel", "volare"]
            .into_iter()
            .find_map(|name| crate::tools::find_in_path(name).map(|path| Program { path, name }))
    }

    fn command(&self, verb: &str, root: &Path, family: &str) -> Command {
        let mut command = Command::new(&self.path);
        command.arg(verb).arg("--pdk-root").arg(root);
        // ciel: --pdk-family; volare: --pdk.
        command.arg(if self.name == "ciel" {
            "--pdk-family"
        } else {
            "--pdk"
        });
        command.arg(family);
        command
    }

    /// The command line that fetches (if needed) and enables a version, for showing.
    pub fn enable_line(&self, root: &Path, family: &str, version: &str) -> String {
        let option = if self.name == "ciel" {
            "--pdk-family"
        } else {
            "--pdk"
        };
        format!(
            "{} enable --pdk-root {} {option} {family} {version}",
            self.name,
            root.display()
        )
    }

    /// The versions that can be fetched (`ciel ls-remote`): the first word of each
    /// line that looks like a version.
    pub fn remote(&self, root: &Path, family: &str) -> Result<Vec<String>> {
        let output = self
            .command("ls-remote", root, family)
            .output()
            .wrap_err_with(|| format!("cannot run {}", self.path.display()))?;
        if !output.status.success() {
            bail!(
                "{} ls-remote failed: {}",
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(parse_versions(&String::from_utf8_lossy(&output.stdout)))
    }

    /// Fetch (if needed) and enable a version (`ciel enable`). Returns what the
    /// program printed.
    pub fn enable(&self, root: &Path, family: &str, version: &str) -> Result<String> {
        let output = self
            .command("enable", root, family)
            .arg(version)
            .output()
            .wrap_err_with(|| format!("cannot run {}", self.path.display()))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(eyre!("{} enable failed: {}", self.name, text.trim()));
        }
        Ok(text)
    }
}

/// The versions in the output of `ciel ls` / `ls-remote`: 40 hexadecimal digits
/// somewhere in a line (the lines are decorated with marks and dates).
fn parse_versions(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            line.split(|c: char| !c.is_ascii_hexdigit())
                .find(|word| word.len() == 40)
                .map(str::to_string)
        })
        .collect()
}

/// The file LibreLane pins PDK versions in: `<family>: <version>` per line.
pub const PIN_FILE: &str = "pdk_hashes.yaml";

/// The PDK versions a design folder pins for LibreLane (`pdk_hashes.yaml` in the
/// folder or in its `librelane` subfolder), family → version. Empty without the file.
pub fn pins(dir: &Path) -> BTreeMap<String, String> {
    [dir.join(PIN_FILE), dir.join("librelane").join(PIN_FILE)]
        .iter()
        .find_map(|file| fs::read_to_string(file).ok())
        .map(|text| parse_pins(&text))
        .unwrap_or_default()
}

fn parse_pins(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next()?.trim();
            let (family, version) = line.split_once(':')?;
            let version = version.trim().trim_matches(['"', '\'']);
            (!family.trim().is_empty() && !version.is_empty())
                .then(|| (family.trim().to_string(), version.to_string()))
        })
        .collect()
}

/// A pinned version that is not the one in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinMismatch {
    pub family: String,
    pub pinned: String,
    /// The version in use under the root; `None` when ciel has none there.
    pub current: Option<String>,
}

impl std::fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.current {
            Some(current) => write!(
                f,
                "{PIN_FILE} pins {} at {}, but {} is in use",
                self.family, self.pinned, current
            ),
            None => write!(
                f,
                "{PIN_FILE} pins {} at {}, but no ciel version of it is in use",
                self.family, self.pinned
            ),
        }
    }
}

/// The pins of `dir` that differ from what is in use under `root`. Only families
/// ciel has under the root are compared: a PDK installed another way has no version
/// to compare.
pub fn pin_mismatches(dir: &Path, root: &Path) -> Vec<PinMismatch> {
    let known = families(root);
    pins(dir)
        .into_iter()
        .filter(|(family, _)| known.contains(family))
        .filter_map(|(family, pinned)| {
            let current = current(root, &family);
            (current.as_deref() != Some(pinned.as_str())).then_some(PinMismatch {
                family,
                pinned,
                current,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "1111111111111111111111111111111111111111";
    const V2: &str = "2222222222222222222222222222222222222222";

    fn version(root: &Path, family: &str, version: &str, pdks: &[&str]) {
        for pdk in pdks {
            fs::create_dir_all(
                root.join("ciel")
                    .join(family)
                    .join("versions")
                    .join(version)
                    .join(pdk)
                    .join("libs.tech"),
            )
            .unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn versions_are_listed_and_switched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        version(root, "sky130", V1, &["sky130A", "sky130B"]);
        version(root, "sky130", V2, &["sky130A", "sky130B"]);
        assert_eq!(families(root), ["sky130"]);
        assert_eq!(current(root, "sky130"), None);
        assert_eq!(installed(root, None).len(), 2);

        assert_eq!(
            enable(root, "sky130", "1111").unwrap(),
            ["sky130A", "sky130B"]
        );
        assert_eq!(current(root, "sky130").as_deref(), Some(V1));
        let link = fs::read_link(root.join("sky130A")).unwrap();
        assert_eq!(
            link,
            Path::new("ciel/sky130/versions").join(V1).join("sky130A")
        );
        assert_eq!(
            fs::read_to_string(root.join("ciel/sky130/current")).unwrap(),
            V1
        );
        let listed = installed(root, Some("sky130"));
        assert_eq!(
            listed.iter().map(|v| v.current).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(listed[0].pdks, ["sky130A", "sky130B"]);

        // Switching replaces the links.
        enable(root, "sky130", V2).unwrap();
        assert_eq!(current(root, "sky130").as_deref(), Some(V2));
        let real = fs::canonicalize(root.join("sky130B")).unwrap();
        let found = version_of(&real).unwrap();
        assert_eq!(
            (found.family.as_str(), found.version.as_str()),
            ("sky130", V2)
        );

        // What is not there, or ambiguous, is said.
        assert!(enable(root, "sky130", "3333").is_err());
        assert!(enable(root, "gf180mcu", V1).is_err());
        version(root, "sky130", "1111aaaa", &["sky130A"]);
        assert!(enable(root, "sky130", "1111").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_real_folder_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        version(root, "sky130", V1, &["sky130A", "sky130B"]);
        fs::create_dir_all(root.join("sky130B/libs.tech")).unwrap();
        let error = enable(root, "sky130", V1).unwrap_err().to_string();
        assert!(error.contains("not replacing"), "{error}");
        assert!(!root.join("sky130A").exists(), "nothing was linked");
    }

    #[test]
    fn the_current_file_counts_without_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        version(root, "gf180mcu", V1, &["gf180mcuD"]);
        fs::write(root.join("ciel/gf180mcu/current"), format!("{V1}\n")).unwrap();
        assert_eq!(current(root, "gf180mcu").as_deref(), Some(V1));
    }

    #[test]
    fn pins_are_compared_with_what_is_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("pdks");
        let design = dir.path().join("design");
        fs::create_dir_all(&design).unwrap();
        version(&root, "sky130", V1, &["sky130A"]);
        fs::write(root.join("ciel/sky130/current"), V1).unwrap();
        fs::write(
            design.join(PIN_FILE),
            format!("# pins\nsky130: {V2}\ngf180mcu: \"{V1}\"\n"),
        )
        .unwrap();
        assert_eq!(pins(&design).len(), 2);
        let wrong = pin_mismatches(&design, &root);
        assert_eq!(wrong.len(), 1, "gf180mcu is not a ciel install here");
        assert_eq!(wrong[0].family, "sky130");
        assert!(wrong[0].to_string().contains(V2));
        fs::write(design.join(PIN_FILE), format!("sky130: {V1}\n")).unwrap();
        assert!(pin_mismatches(&design, &root).is_empty());
    }

    #[test]
    fn versions_are_picked_out_of_the_programs_output() {
        let text = format!(
            "In use:\n  * {V1} (2025-01-01) (enabled)\n    {V2} (2024-12-01)\nnothing here\n"
        );
        assert_eq!(parse_versions(&text), [V1, V2]);
    }
}
