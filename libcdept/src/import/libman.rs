//! IHP LibMan (now DOOR): reading its project file, writing one for a CDePT project.
//!
//! What LibMan keeps on disk (worked out from its documentation and example projects;
//! the code here is our own):
//!
//! - A project file, `<name>.projects` or `<name>.lib`: lines of
//!   `define("library", "path");`, with `#` comments. The path is relative to the file
//!   and names a view file (`lib/cell/cell.gds`), a library folder (`lib`) or a
//!   pattern (`lib/*`: every view below `lib`; `lib/cell/*`: the views of one cell).
//!   A library is usually defined by many lines. Paths written on Windows use `\`.
//!   Also `pdk("name", "path");` (the PDK and where it is) and `attach("library",
//!   "reference library");`. Older files have `PROJECT name path` and `GROUP name
//!   library…` lines instead.
//! - A library is a folder of cell folders, the views are files in the cell folder:
//!   `<library>/<cell>/<cell>.<suffix>` with the suffixes `.gds`, `.oas`, `.lstr`,
//!   `.sch`, `.sym` and, in DOOR, `<cell>.<view>.room`. That is CDePT's `cells`
//!   layout with the files directly in the cell folder.
//! - Categories: a file `<library>/<name>.group` listing cell names (read by
//!   [`crate::category::read_libman`]).
//!
//! LibMan is started with the project file as its only argument.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Context, Result};

use crate::Project;

/// Suffixes of a LibMan project file.
pub const PROJECT_SUFFIXES: [&str; 2] = ["projects", "lib"];

/// A LibMan project file, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibmanProject {
    /// The file it was read from (empty when parsed from text).
    pub file: PathBuf,
    /// The libraries in the order they first appear.
    pub libraries: Vec<LibmanLibrary>,
    /// `pdk("name", "path")`.
    pub pdk: Option<(String, String)>,
    /// `attach("library", "reference")`.
    pub attached: Vec<(String, String)>,
    /// `GROUP name library…` of older files.
    pub groups: Vec<(String, Vec<String>)>,
    /// Lines that were not understood, with their number.
    pub problems: Vec<String>,
}

/// One library of a LibMan project: its name and every path defined for it, as written
/// (with `/` as the separator).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibmanLibrary {
    pub name: String,
    pub paths: Vec<String>,
}

/// The arguments of `name(arg, arg)`: quoted or bare, separated by commas.
fn arguments(text: &str) -> Option<Vec<String>> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut had_quote = false;
    for c in text[open + 1..close].chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                had_quote = true;
            }
            None if c == ',' => {
                out.push(std::mem::take(&mut current).trim().to_string());
                had_quote = false;
            }
            None => current.push(c),
        }
    }
    if !current.trim().is_empty() || had_quote {
        out.push(current.trim().to_string());
    }
    Some(out)
}

/// A path as LibMan wrote it, with `/` separators. `\\` in a quoted path is one `\`.
fn slashes(path: &str) -> String {
    let mut out = path.replace("\\\\", "/").replace('\\', "/");
    while out.contains("//") && !out.starts_with("//") {
        out = out.replace("//", "/");
    }
    out
}

/// Read the text of a LibMan project file.
pub fn parse(text: &str) -> LibmanProject {
    let mut project = LibmanProject::default();
    let add = |project: &mut LibmanProject, name: &str, path: &str| {
        let path = slashes(path);
        match project.libraries.iter_mut().find(|l| l.name == name) {
            Some(library) => {
                if !library.paths.contains(&path) {
                    library.paths.push(path);
                }
            }
            None => project.libraries.push(LibmanLibrary {
                name: name.to_string(),
                paths: vec![path],
            }),
        }
    };
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let word: String = line
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let bad = |project: &mut LibmanProject| {
            project
                .problems
                .push(format!("line {}: not understood: {line}", number + 1));
        };
        match (word.as_str(), arguments(line)) {
            ("define", Some(args)) if args.len() == 2 => add(&mut project, &args[0], &args[1]),
            ("pdk", Some(args)) if args.len() == 2 => {
                project.pdk = Some((args[0].clone(), slashes(&args[1])));
            }
            ("attach", Some(args)) if args.len() == 2 => {
                project.attached.push((args[0].clone(), args[1].clone()));
            }
            ("PROJECT", _) => {
                let mut words = line.split_whitespace().skip(1);
                match (words.next(), words.next()) {
                    (Some(name), Some(path)) => add(&mut project, name, path),
                    _ => bad(&mut project),
                }
            }
            ("GROUP", _) => {
                let mut words = line.split_whitespace().skip(1).map(str::to_string);
                match words.next() {
                    Some(name) => project.groups.push((name, words.collect())),
                    None => bad(&mut project),
                }
            }
            _ => bad(&mut project),
        }
    }
    project
}

/// Read a LibMan project file.
pub fn read(file: &Path) -> Result<LibmanProject> {
    let text = std::fs::read_to_string(file)
        .wrap_err_with(|| format!("cannot read {}", file.display()))?;
    let mut project = parse(&text);
    project.file = file.to_path_buf();
    Ok(project)
}

/// The LibMan project files directly in `folder`: `*.projects`, and `*.lib` files that
/// hold `define(` lines (`.lib` is also Liberty's suffix).
pub fn find(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(items) = std::fs::read_dir(folder) else {
        return out;
    };
    for item in items.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("projects") => out.push(path),
            Some("lib") => {
                let small = std::fs::metadata(&path).is_ok_and(|m| m.len() < (1 << 20));
                let defines = small
                    && std::fs::read_to_string(&path).is_ok_and(|text| {
                        text.lines()
                            .any(|line| line.trim_start().starts_with("define("))
                    });
                if defines {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
    out.sort();
    out
}

/// `dir` looks like a cell folder: it holds a file named after it (`inv/inv.gds`,
/// `inv/inv.schematic.room`).
fn is_cell_folder(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let prefix = format!("{name}.");
    std::fs::read_dir(dir).is_ok_and(|items| {
        items.flatten().any(|item| {
            item.path().is_file() && item.file_name().to_string_lossy().starts_with(&prefix)
        })
    })
}

/// The library folder one defined path stands for, when it can be told.
fn folder_of(base: &Path, path: &str) -> Option<PathBuf> {
    let (plain, pattern) = match path.strip_suffix("/*") {
        Some(rest) => (rest, true),
        None => (path.trim_end_matches('/'), false),
    };
    if plain.contains(['*', '?']) {
        return None;
    }
    let full = base.join(plain);
    if full.is_dir() {
        // `lib/cell/*` and `lib/cell` name a cell folder, `lib/*` and `lib` the library.
        if (pattern || !full.join("lib.toml").exists()) && is_cell_folder(&full) {
            return full.parent().map(Path::to_path_buf);
        }
        return Some(full);
    }
    // A view file: `lib/cell/cell.gds` (the usual shape) or `lib/file.gds`.
    let parent = full.parent()?;
    if !pattern && full.extension().is_some() {
        let stem = full.file_name()?.to_string_lossy().into_owned();
        let cell = parent.file_name()?.to_string_lossy().into_owned();
        if stem.starts_with(&format!("{cell}.")) {
            return parent.parent().map(Path::to_path_buf);
        }
        return Some(parent.to_path_buf());
    }
    None
}

/// A library of a LibMan project and the folder(s) its paths stand for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLibrary {
    pub name: String,
    /// The library folders, the one most paths point into first.
    pub folders: Vec<PathBuf>,
    /// Paths no folder could be told for.
    pub unresolved: Vec<String>,
}

impl LibmanProject {
    /// The folder paths of the file are relative to.
    pub fn base(&self) -> PathBuf {
        self.file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The library folders behind the defined paths.
    pub fn resolve(&self) -> Vec<ResolvedLibrary> {
        let base = self.base();
        self.libraries
            .iter()
            .map(|library| {
                let mut count: BTreeMap<PathBuf, usize> = BTreeMap::new();
                let mut order: Vec<PathBuf> = Vec::new();
                let mut unresolved = Vec::new();
                for path in &library.paths {
                    match folder_of(&base, path) {
                        Some(folder) => {
                            let folder = folder.canonicalize().unwrap_or(folder);
                            if !order.contains(&folder) {
                                order.push(folder.clone());
                            }
                            *count.entry(folder).or_default() += 1;
                        }
                        None => unresolved.push(path.clone()),
                    }
                }
                order.sort_by_key(|folder| std::cmp::Reverse(count[folder]));
                ResolvedLibrary {
                    name: library.name.clone(),
                    folders: order,
                    unresolved,
                }
            })
            .collect()
    }
}

/// A LibMan project file for the libraries of a CDePT project: one `define("library",
/// "folder/*");` line each (absolute folders, so the file can be kept anywhere), the
/// PDK when the project names one.
pub fn export(project: &Project) -> String {
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = format!(
        "# Library definition file for LibMan / DOOR\n# Written by CDePT for project {}. \
         Rewritten every time: do not edit.\n\n",
        project.name
    );
    for library in project.libraries.iter().filter(|l| l.error.is_none()) {
        out.push_str(&format!(
            "define(\"{}\", \"{}/*\");\n",
            quote(&library.name),
            quote(&library.path.to_string_lossy())
        ));
    }
    if let (Some(root), Some(name)) = (project.pdk.var("PDK_ROOT"), project.pdk.var("PDK"))
        && !name.contains(':')
    {
        let dir = Path::new(root).join(name);
        out.push_str(&format!(
            "\npdk(\"{}\", \"{}\");\n",
            quote(name),
            quote(&dir.to_string_lossy())
        ));
    }
    out
}

/// Where the exported project file of `project` is kept: in the user's cache folder,
/// not in the project.
pub fn export_file(project: &Project) -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let name: String = project
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_-.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = if name.is_empty() {
        "project".into()
    } else {
        name
    };
    cache
        .join("cdept")
        .join("libman")
        .join(format!("{name}.projects"))
}

/// The LibMan program, when it is installed: `libman`, `LibMan` or `door` on `PATH`.
pub fn find_program() -> Option<PathBuf> {
    ["libman", "LibMan", "door"]
        .into_iter()
        .find_map(crate::tools::find_in_path)
}

/// Write the project's libraries as a LibMan project file and start LibMan on it.
/// Returns the file.
pub fn open(project: &Project) -> Result<PathBuf> {
    let program = find_program()
        .ok_or_else(|| eyre::eyre!("LibMan was not found (libman or door on PATH)"))?;
    let file = export_file(project);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(&file, export(project))
        .wrap_err_with(|| format!("cannot write {}", file.display()))?;
    std::process::Command::new(&program)
        .arg(&file)
        .current_dir(&project.root)
        .envs(project.tool_env())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .wrap_err_with(|| format!("cannot start {}", program.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_pdk_and_attach_are_read() {
        let text = r#"
# KLayout library definition file
# Generated by DOOR

define("analogLib", "analogLib\\*");
define("ihp_sg13g2", "sg13g2_stdcell\\Test\\Test.gds");
define("ihp_sg13g2", "sg13g2_stdcell\\Test\\Test.oas");
define(bare, some/folder)
pdk("ihp-sg13g2", "C:\\Work\\PDK\\IHP-Open-PDK\\ihp-sg13g2");
attach("EMTest", "analogLib");
nonsense here
"#;
        let p = parse(text);
        let names: Vec<&str> = p.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["analogLib", "ihp_sg13g2", "bare"]);
        assert_eq!(p.libraries[0].paths, ["analogLib/*"]);
        assert_eq!(
            p.libraries[1].paths,
            [
                "sg13g2_stdcell/Test/Test.gds",
                "sg13g2_stdcell/Test/Test.oas"
            ]
        );
        assert_eq!(p.libraries[2].paths, ["some/folder"]);
        assert_eq!(
            p.pdk,
            Some((
                "ihp-sg13g2".into(),
                "C:/Work/PDK/IHP-Open-PDK/ihp-sg13g2".into()
            ))
        );
        assert_eq!(p.attached, [("EMTest".into(), "analogLib".into())]);
        assert_eq!(p.problems.len(), 1);
        assert!(p.problems[0].contains("nonsense"));
    }

    #[test]
    fn older_project_and_group_lines_are_read() {
        let p = parse("GROUP Text test1 test2\nPROJECT test1 ~/test1\nPROJECT test2 ~/test2\n");
        assert_eq!(p.libraries.len(), 2);
        assert_eq!(p.libraries[0].paths, ["~/test1"]);
        assert_eq!(
            p.groups,
            [("Text".into(), vec!["test1".into(), "test2".into()])]
        );
        assert!(p.problems.is_empty());
    }

    #[test]
    fn commas_and_quotes_in_arguments() {
        assert_eq!(
            arguments(r#"define("a, b", 'c')"#).unwrap(),
            ["a, b".to_string(), "c".to_string()]
        );
        assert_eq!(arguments("define()").unwrap(), Vec::<String>::new());
        assert!(arguments("define").is_none());
    }
}
