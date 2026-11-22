//! Finding PDKs on this machine: the folders that hold PDKs, the PDKs in a folder,
//! what a PDK says about itself.
//!
//! A folder that holds PDKs (a *root*, what `$PDK_ROOT` means) has one subfolder per
//! PDK, and a PDK is a folder with `libs.tech` in it. [`Search`] says which roots to
//! look at, [`discover`] lists every PDK in them, [`find`] looks one up by name and
//! [`info`] reads what a PDK says about itself.
//!
//! Discovery is cheap and careful about what it touches: it lists each root once and
//! then only asks whether `<child>/libs.tech` exists. It never lists a subfolder that
//! is not a PDK. A checkout that holds PDKs one level further down (a clone of
//! IHP-Open-PDK holds `ihp-sg13g2` and `ihp-sg13cmos5l`) is recognised by asking for
//! the well-known names in it ([`NESTED_NAMES`]), not by walking it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::ciel;
use super::pdk::{Family, PdkDir, PdkInfo};

/// PDK names asked for inside a subfolder of a root that is not a PDK itself.
pub const NESTED_NAMES: &[&str] = &[
    "ihp-sg13g2",
    "ihp-sg13cmos5l",
    "sky130A",
    "sky130B",
    "gf180mcuA",
    "gf180mcuB",
    "gf180mcuC",
    "gf180mcuD",
    "gf180mcu",
];

/// The variable with more folders that hold PDKs (separated like `$PATH`).
pub const PATH_VAR: &str = "CDEPT_PDK_PATH";

/// Why a folder is looked at for PDKs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// `pdk_root` of the project.
    Project,
    /// `$PDK_ROOT`.
    Environment,
    /// `$CDEPT_PDK_PATH`.
    Path,
    /// A folder the user added in the GUI settings.
    User,
    /// ciel's or volare's own folder (`~/.ciel`, `~/.volare`).
    Ciel,
    /// A place PDKs are usually installed to (`/usr/share/pdk`, …).
    System,
    /// A folder named on the spot (a command line option, "Other folder…").
    Given,
}

impl Origin {
    pub fn name(self) -> &'static str {
        match self {
            Origin::Project => "project",
            Origin::Environment => "$PDK_ROOT",
            Origin::Path => "$CDEPT_PDK_PATH",
            Origin::User => "settings",
            Origin::Ciel => "ciel",
            Origin::System => "system",
            Origin::Given => "given",
        }
    }
}

/// A folder that holds PDKs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub path: PathBuf,
    pub origin: Origin,
}

/// Where to look for PDKs.
#[derive(Debug, Clone)]
pub struct Search {
    /// The project's PDK folder (`pdk_root`, as resolved), looked at first.
    pub project: Option<PathBuf>,
    /// Folders the user added (GUI settings).
    pub extra: Vec<PathBuf>,
    /// Folders named on the spot, looked at before everything else.
    pub given: Vec<PathBuf>,
    /// The usual places too (`~/.ciel`, `~/.volare`, `/usr/share/pdk`, …).
    pub usual: bool,
    env: fn(&str) -> Option<String>,
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl Default for Search {
    fn default() -> Self {
        Search::new()
    }
}

impl Search {
    /// The environment of this process and the usual places.
    pub fn new() -> Search {
        Search::with_env(process_env)
    }

    /// Like [`Search::new`] with another environment (for tests).
    pub fn with_env(env: fn(&str) -> Option<String>) -> Search {
        Search {
            project: None,
            extra: Vec::new(),
            given: Vec::new(),
            usual: true,
            env,
        }
    }

    /// Also the PDK folder of this project (what `$PDK_ROOT` means in it).
    pub fn project(mut self, root: Option<impl Into<PathBuf>>) -> Search {
        self.project = root.map(Into::into);
        self
    }

    /// Also these folders (the user's GUI settings).
    pub fn extra(mut self, folders: impl IntoIterator<Item = PathBuf>) -> Search {
        self.extra.extend(folders);
        self
    }

    /// Also these folders, first.
    pub fn given(mut self, folders: impl IntoIterator<Item = PathBuf>) -> Search {
        self.given.extend(folders);
        self
    }

    /// Only the folders named, none of the usual places.
    pub fn only(mut self) -> Search {
        self.usual = false;
        self
    }

    fn home(&self) -> Option<PathBuf> {
        (self.env)("HOME").map(PathBuf::from)
    }

    /// The folders to look at, in order, each once. Folders that do not exist are
    /// left out. A folder that is itself a PDK stands for the folder it is in.
    pub fn roots(&self) -> Vec<Root> {
        let mut candidates: Vec<(PathBuf, Origin)> = Vec::new();
        for path in &self.given {
            candidates.push((path.clone(), Origin::Given));
        }
        if let Some(path) = &self.project {
            candidates.push((path.clone(), Origin::Project));
        }
        if let Some(value) = (self.env)("PDK_ROOT") {
            candidates.push((PathBuf::from(value), Origin::Environment));
        }
        if let Some(value) = (self.env)(PATH_VAR) {
            for path in std::env::split_paths(&value) {
                if !path.as_os_str().is_empty() {
                    candidates.push((path, Origin::Path));
                }
            }
        }
        for path in &self.extra {
            candidates.push((path.clone(), Origin::User));
        }
        if self.usual {
            if let Some(home) = self.home() {
                candidates.push((home.join(".ciel"), Origin::Ciel));
                candidates.push((home.join(".volare"), Origin::Ciel));
            }
            candidates.push(("/usr/share/pdk".into(), Origin::System));
            candidates.push(("/usr/local/share/pdk".into(), Origin::System));
            if let Some(home) = self.home() {
                candidates.push((home.join("share/pdk"), Origin::System));
            }
        }

        let mut roots: Vec<Root> = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for (path, origin) in candidates {
            let path = expand_home(&path, self.home().as_deref());
            if !path.is_dir() {
                continue;
            }
            // `PDK_ROOT=/opt/pdks/sky130A` is a common slip: the root is its parent.
            let path = match (is_pdk(&path), path.parent()) {
                (true, Some(parent)) => parent.to_path_buf(),
                _ => path,
            };
            let real = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen.contains(&real) {
                continue;
            }
            seen.push(real);
            roots.push(Root { path, origin });
        }
        roots
    }
}

fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

/// A folder is a PDK when it has `libs.tech`.
pub fn is_pdk(dir: &Path) -> bool {
    dir.join("libs.tech").is_dir()
}

/// The PDKs in a root: its subfolders with `libs.tech` (symbolic links followed),
/// and the well-known PDKs one level further down (see the module text). Sorted by
/// name; hidden folders and ciel's version store are not looked at.
pub fn list(root: &Path) -> Vec<PdkDir> {
    let mut found: Vec<PdkDir> = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.') && !ciel::STORES.contains(&name.as_str()))
        .collect();
    names.sort();
    let mut nested: Vec<PdkDir> = Vec::new();
    for name in names {
        let child = root.join(&name);
        if is_pdk(&child) {
            found.push(PdkDir::new(root, name));
        } else if child.is_dir() {
            for known in NESTED_NAMES {
                if is_pdk(&child.join(known)) {
                    nested.push(PdkDir::new(&child, *known));
                }
            }
        }
    }
    found.extend(nested);
    found
}

/// A PDK found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub pdk: PdkDir,
    /// Why its root was looked at.
    pub origin: Origin,
    /// Its folder with symbolic links resolved.
    pub real: PathBuf,
}

impl Found {
    /// What the PDK says about itself.
    pub fn info(&self) -> PdkInfo {
        info(&self.pdk)
    }
}

/// Every PDK in the roots of `search`, in root order. A PDK reached twice (through a
/// symbolic link, or a checkout that is also linked into a root) is listed once, the
/// way it was reached first.
pub fn discover(search: &Search) -> Vec<Found> {
    let mut all: Vec<Found> = Vec::new();
    for root in search.roots() {
        for pdk in list(&root.path) {
            let real = fs::canonicalize(pdk.dir()).unwrap_or_else(|_| pdk.dir());
            if all.iter().any(|f| f.real == real) {
                continue;
            }
            all.push(Found {
                pdk,
                origin: root.origin,
                real,
            });
        }
    }
    all
}

/// The PDK called `name` in the first root that has it. `name` may also be a path to
/// a PDK folder.
pub fn find(name: &str, search: &Search) -> Option<PdkDir> {
    if name.contains('/') {
        return from_folder(Path::new(name));
    }
    for root in search.roots() {
        if is_pdk(&root.path.join(name)) {
            return Some(PdkDir::new(&root.path, name));
        }
    }
    // One level further down (checkouts), and names that differ in case only.
    let all = discover(search);
    all.iter()
        .find(|f| f.pdk.name == name)
        .or_else(|| all.iter().find(|f| f.pdk.name.eq_ignore_ascii_case(name)))
        .map(|f| f.pdk.clone())
}

/// The PDK a folder is: the folder itself when it has `libs.tech`.
pub fn from_folder(dir: &Path) -> Option<PdkDir> {
    if !is_pdk(dir) {
        return None;
    }
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let name = dir.file_name()?.to_str()?.to_string();
    Some(PdkDir::new(dir.parent()?, name))
}

/// The PDKs a folder the user picked stands for: the folder itself when it is a PDK,
/// else the PDKs in it.
pub fn in_folder(dir: &Path) -> Vec<PdkDir> {
    match from_folder(dir) {
        Some(pdk) => vec![pdk],
        None => list(dir),
    }
}

/// `$PDK` may list several PDKs separated by colons (`sky130A:gf180mcuD:ihp-sg13g2`):
/// the PDKs that are available, not the one in use.
pub fn names(value: &str) -> Vec<String> {
    value
        .split(':')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// What the PDK says about itself (`[pdk]` in `tech.toml`): only small files are
/// read, no program is started.
pub fn info(pdk: &PdkDir) -> PdkInfo {
    let dir = pdk.dir();
    let mut info = PdkInfo {
        name: pdk.name.clone(),
        family: pdk.family,
        tools: tools(&dir),
        ..PdkInfo::default()
    };
    let real = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());

    // open_pdks leaves a description of the build in `.config/nodeinfo.json`.
    let node = fs::read_to_string(dir.join(".config/nodeinfo.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    if let Some(node) = &node {
        let text = |path: &[&str]| {
            let mut value = node;
            for key in path {
                value = value.get(key)?;
            }
            value.as_str().map(str::to_string)
        };
        info.description = text(&["description"]);
        info.version = text(&["build", "open_pdks"]);
        info.commit = text(&["commit", "open_pdks"]);
        info.source = Some("open_pdks".into());
        if let Some(magic) = text(&["build", "magic"]) {
            info.tested_with.insert("magic".into(), magic);
        }
    }
    // `SOURCES` names the open_pdks commit too (older builds have no nodeinfo).
    if info.commit.is_none()
        && let Ok(text) = fs::read_to_string(dir.join("SOURCES"))
        && let Some(commit) = text
            .lines()
            .find_map(|line| line.strip_prefix("open_pdks "))
    {
        info.commit = Some(commit.trim().to_string());
        info.source.get_or_insert_with(|| "open_pdks".into());
    }
    // Installed by ciel: the version is the folder it is in.
    if let Some(version) = ciel::version_of(&real) {
        info.source = Some(version.store.clone());
        if info.commit.is_none() {
            info.commit = Some(version.version.clone());
        }
    }

    // A checkout (IHP-Open-PDK): the commit from `.git`, the date of the last
    // changelog entry, the tool versions it was tested with.
    if node.is_none()
        && let Some(checkout) = real.parent()
    {
        if let Some(git) = git_head(checkout) {
            info.commit = info.commit.or(git.commit);
            info.branch = git.branch;
            info.source.get_or_insert_with(|| "git".into());
        }
        if info.version.is_none() {
            info.version = changelog_version(checkout);
        }
        if let Ok(text) = fs::read_to_string(checkout.join("versions.txt")) {
            info.tested_with.extend(tool_versions(&text));
        }
    }
    info.source.get_or_insert_with(|| "folder".into());
    if info.description.is_none() {
        info.description = family_description(pdk.family).map(str::to_string);
    }
    info
}

/// The `libs.tech/<tool>` folders of a PDK, sorted.
fn tools(dir: &Path) -> Vec<String> {
    let mut tools: Vec<String> = fs::read_dir(dir.join("libs.tech"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    tools.sort();
    tools
}

fn family_description(family: Family) -> Option<&'static str> {
    match family {
        Family::Sky130 => Some("SkyWater 130 nm CMOS"),
        Family::Gf180mcu => Some("GlobalFoundries 180 nm MCU CMOS"),
        Family::IhpSg13g2 => Some("IHP SG13G2 130 nm SiGe BiCMOS"),
        Family::IhpSg13cmos5l => Some("IHP SG13CMOS5L 130 nm CMOS, 5 metal layers"),
        Family::Other => None,
    }
}

/// What `.git` of a checkout says without running git.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GitHead {
    branch: Option<String>,
    commit: Option<String>,
}

fn git_head(checkout: &Path) -> Option<GitHead> {
    let dot_git = checkout.join(".git");
    // A linked worktree has a file `.git` saying where its part of the repository is;
    // the branches are in the main repository (`commondir`).
    let git_dir = if dot_git.is_file() {
        let text = fs::read_to_string(&dot_git).ok()?;
        let path = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
        checkout.join(path)
    } else if dot_git.is_dir() {
        dot_git
    } else {
        return None;
    };
    let common = fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|text| git_dir.join(text.trim()))
        .unwrap_or_else(|| git_dir.clone());
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref:").map(str::trim) else {
        // A detached HEAD holds the commit itself.
        return Some(GitHead {
            branch: None,
            commit: is_hash(head).then(|| head.to_string()),
        });
    };
    let branch = reference
        .strip_prefix("refs/heads/")
        .unwrap_or(reference)
        .to_string();
    let loose = [&git_dir, &common]
        .into_iter()
        .find_map(|dir| fs::read_to_string(dir.join(reference)).ok())
        .map(|text| text.trim().to_string());
    let commit = loose.or_else(|| {
        let packed = fs::read_to_string(common.join("packed-refs")).ok()?;
        packed.lines().find_map(|line| {
            let (hash, name) = line.split_once(' ')?;
            (name.trim() == reference).then(|| hash.to_string())
        })
    });
    Some(GitHead {
        branch: Some(branch),
        commit: commit.filter(|c| is_hash(c)),
    })
}

fn is_hash(text: &str) -> bool {
    text.len() >= 7 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// The first entry of a "Keep a Changelog" file: `## [1.2.0] - 2026-09-01` gives
/// `1.2.0 (2026-09-01)`. Only the head of the file is read.
fn changelog_version(checkout: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = fs::File::open(checkout.join("CHANGELOG.md")).ok()?;
    for line in BufReader::new(file).lines().take(60).map_while(Result::ok) {
        let Some(rest) = line.strip_prefix("## ") else {
            continue;
        };
        let (name, date) = match rest.split_once(" - ") {
            Some((name, date)) => (name, Some(date.trim())),
            None => (rest, None),
        };
        let name = name.trim().trim_matches(['[', ']']).to_ascii_lowercase();
        return Some(match date {
            Some(date) => format!("{name} ({date})"),
            None => name,
        });
    }
    None
}

/// `versions.txt` of IHP-Open-PDK: one `tool version` per line.
fn tool_versions(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (tool, version) = line.trim().split_once(char::is_whitespace)?;
            Some((tool.to_ascii_lowercase(), version.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdk(root: &Path, name: &str) {
        fs::create_dir_all(root.join(name).join("libs.tech/klayout")).unwrap();
        fs::create_dir_all(root.join(name).join("libs.tech/xschem")).unwrap();
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn pdks_are_the_folders_with_libs_tech() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pdk(root, "sky130A");
        pdk(root, "ihp-sg13g2");
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::create_dir_all(root.join(".hidden/libs.tech")).unwrap();
        // A checkout holding a PDK one level down, and a folder that is neither.
        pdk(&root.join("IHP-Open-PDK"), "ihp-sg13cmos5l");
        pdk(&root.join("secret"), "private_pdk");

        let names: Vec<String> = list(root).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["ihp-sg13g2", "sky130A", "ihp-sg13cmos5l"]);
        let nested = &list(root)[2];
        assert_eq!(nested.root, root.join("IHP-Open-PDK"));
        assert_eq!(nested.family, Family::IhpSg13cmos5l);
    }

    #[test]
    fn roots_come_in_order_once_and_a_pdk_stands_for_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        pdk(&a, "sky130A");
        pdk(&b, "gf180mcuD");
        let search = Search::with_env(no_env)
            .only()
            .project(Some(a.join("sky130A")))
            .extra([b.clone(), a.clone(), dir.path().join("missing")]);
        let roots = search.roots();
        assert_eq!(
            roots,
            [
                Root {
                    path: a.clone(),
                    origin: Origin::Project
                },
                Root {
                    path: b.clone(),
                    origin: Origin::User
                },
            ]
        );
        let found = discover(&search);
        assert_eq!(found.len(), 2);
        assert_eq!(find("gf180mcuD", &search).unwrap().root, b);
        assert_eq!(find("GF180MCUD", &search).unwrap().name, "gf180mcuD");
        assert!(find("nothing", &search).is_none());
        let by_path = find(&a.join("sky130A").to_string_lossy(), &search).unwrap();
        assert_eq!((by_path.root, by_path.name.as_str()), (a, "sky130A"));
    }

    #[cfg(unix)]
    #[test]
    fn a_pdk_reached_twice_is_listed_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pdk(&root.join("IHP-Open-PDK"), "ihp-sg13g2");
        std::os::unix::fs::symlink("IHP-Open-PDK/ihp-sg13g2", root.join("ihp-sg13g2")).unwrap();
        let search = Search::with_env(no_env).only().given([root.to_path_buf()]);
        let found = discover(&search);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].pdk.root, root);
        assert_eq!(found[0].origin, Origin::Given);
    }

    #[test]
    fn the_environment_names_roots() {
        fn env(name: &str) -> Option<String> {
            match name {
                "PDK_ROOT" => Some("/nonexistent/pdks".into()),
                _ => None,
            }
        }
        assert!(Search::with_env(env).only().roots().is_empty());
        assert_eq!(
            names("sky130A:gf180mcuD: ihp-sg13g2:"),
            ["sky130A", "gf180mcuD", "ihp-sg13g2"]
        );
    }

    #[test]
    fn open_pdks_says_its_version() {
        let dir = tempfile::tempdir().unwrap();
        pdk(dir.path(), "sky130A");
        let config = dir.path().join("sky130A/.config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("nodeinfo.json"),
            r#"{"description": "Skywater 0.13um CMOS",
                "build": {"open_pdks": "1.0.530", "magic": "8.3.525"},
                "commit": {"open_pdks": "8afc8346"}}"#,
        )
        .unwrap();
        let info = info(&PdkDir::new(dir.path(), "sky130A"));
        assert_eq!(info.version.as_deref(), Some("1.0.530"));
        assert_eq!(info.commit.as_deref(), Some("8afc8346"));
        assert_eq!(info.source.as_deref(), Some("open_pdks"));
        assert_eq!(info.description.as_deref(), Some("Skywater 0.13um CMOS"));
        assert_eq!(info.tools, ["klayout", "xschem"]);
        assert_eq!(info.tested_with["magic"], "8.3.525");
    }

    #[test]
    fn a_checkout_says_its_branch_commit_and_changelog() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("IHP-Open-PDK");
        pdk(&checkout, "ihp-sg13g2");
        fs::create_dir_all(checkout.join(".git/refs/heads")).unwrap();
        fs::write(checkout.join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        fs::write(
            checkout.join(".git/packed-refs"),
            "# pack-refs with: peeled\n1111111111111111111111111111111111111111 refs/heads/dev\n",
        )
        .unwrap();
        fs::write(
            checkout.join("CHANGELOG.md"),
            "# Changelog\n\n## [Unreleased] - 2026-09-01\n\n## [0.1.0] - 2024-10-14\n",
        )
        .unwrap();
        fs::write(
            checkout.join("versions.txt"),
            "ngspice 43\nxschem 3.4.8RC\n",
        )
        .unwrap();
        let pdk = PdkDir::new(&checkout, "ihp-sg13g2");
        let found = info(&pdk);
        assert_eq!(found.branch.as_deref(), Some("dev"));
        assert_eq!(
            found.commit.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(found.version.as_deref(), Some("unreleased (2026-09-01)"));
        assert_eq!(found.source.as_deref(), Some("git"));
        assert_eq!(found.tested_with["xschem"], "3.4.8RC");
        assert!(found.description.unwrap().contains("SG13G2"));

        // A loose ref wins over the packed one.
        fs::write(
            checkout.join(".git/refs/heads/dev"),
            "2222222222222222222222222222222222222222\n",
        )
        .unwrap();
        assert!(info(&pdk).commit.unwrap().starts_with("2222"));
    }
}
