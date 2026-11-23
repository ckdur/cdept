//! The technology's models for VACASK, made on demand.
//!
//! No PDK installed here brings VACASK models (`libs.tech/vacask`); they bring ngspice
//! and Xyce models and the Verilog-A sources of their compact models. So CDePT makes
//! the VACASK set from what there is, once per technology, and records it in the
//! snapshot as `[models.vacask]` with `generated` saying where it came from:
//!
//! 1. the library files of `[models.ngspice]` (and the files the corner files include)
//!    are converted by VACASK's own converter (`ng2vc`, run as a separate Python
//!    process, `convert.py`) into `<project>/.cdept/osdi/<pdk>-<hash>/models/`;
//! 2. the Verilog-A models behind the set's `osdi` files are compiled with OpenVAF
//!    ([`crate::batch::openvaf`]) into the same folder;
//! 3. a [`ModelSet`] with the converted corner entries is recorded in `tech.toml`.
//!
//! A file the converter cannot read, a Verilog-A model that does not compile, a
//! corner entry that needs either: each becomes a note `vacask: …` in the snapshot,
//! and a testbench that needs such a device is refused with the PDK's and the device's
//! names ([`super::deck::check_devices`]). Nothing is skipped silently.
//!
//! The hash covers the sources and the converter script, so a changed PDK or CDePT
//! makes a new folder; the old one stays until someone removes `.cdept/osdi`.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::Project;
use crate::batch::openvaf;
use crate::tech::Tech;
use crate::tech::models::ModelSet;
use crate::tech::pdk::TechPath;

/// The converter glue ([`self`] runs it with `python3`).
const GLUE: &str = include_str!("convert.py");

/// The name of the generated set in `[models]`.
pub const SET: &str = "vacask";

/// File inside the cache folder that says the generation finished.
const REPORT: &str = "cdept-vacask.json";

/// `$CDEPT_PROJECT` stands for the project folder in the snapshot's paths.
const PROJECT_VAR: &str = "$CDEPT_PROJECT";

/// What generation made; saved next to its files so that a later run only reads it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Generated {
    pub hash: String,
    pub pdk: String,
    pub set: ModelSet,
    /// `subckt` and `model` names the converted files define.
    pub defined: BTreeSet<String>,
    /// What could not be done, as snapshot notes (`vacask: …`).
    pub notes: Vec<String>,
    /// Converted files, compiled libraries (relative to the folder).
    pub converted: Vec<String>,
    pub compiled: Vec<String>,
    /// What the converted files draw from the Monte Carlo vectors ([`super::stat`]).
    #[serde(default)]
    pub stat: super::stat::MmInfo,
}

/// A library file to convert.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Source {
    /// Name inside the set's folder (the converted file keeps it).
    name: String,
    path: PathBuf,
    /// The file has `.lib <section>` blocks (corner file).
    corner: bool,
}

/// What generation reads.
#[derive(Debug, Clone)]
pub struct Plan {
    pub pdk: String,
    sources: Vec<Source>,
    /// The `.osdi` stems of the ngspice set and the Verilog-A file behind each.
    osdi: Vec<(String, Option<PathBuf>)>,
    pub hash: String,
    ngspice: ModelSet,
    notes: Vec<String>,
}

fn lines_of(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim)
}

/// `.lib <section>` with no file: the file defines sections.
fn has_sections(text: &str) -> bool {
    lines_of(text).any(|l| {
        let mut w = l.split_whitespace();
        w.next().is_some_and(|c| c.eq_ignore_ascii_case(".lib"))
            && w.next().is_some()
            && w.next().is_none()
    })
}

/// The files `.include`d and `.lib`ed (`.lib file section`) by a file, as written. With
/// `sections`, only those outside sections or inside one of the named sections.
fn included(text: &str, sections: Option<&BTreeSet<&str>>) -> Vec<String> {
    let mut out = vec![];
    let mut current: Option<String> = None;
    for l in lines_of(text) {
        let mut w = l.split_whitespace();
        let Some(card) = w.next().map(str::to_lowercase) else {
            continue;
        };
        if card == ".endl" {
            current = None;
            continue;
        }
        if card == ".lib" {
            let rest: Vec<&str> = l.split_whitespace().skip(1).collect();
            if rest.len() == 1 {
                current = Some(rest[0].to_string());
                continue;
            }
        }
        if let (Some(only), Some(cur)) = (sections, &current)
            && !only.contains(cur.as_str())
        {
            continue;
        }
        let file = match card.as_str() {
            ".include" | ".inc" => w.next(),
            ".lib" => {
                let f = w.next();
                if w.next().is_some() { f } else { None }
            }
            _ => None,
        };
        if let Some(f) = file {
            out.push(f.trim_matches(['"', '\'']).to_string());
        }
    }
    out
}

/// Find `stem.va` below `dir` (the folder of the same name first).
fn find_va(dir: &Path, stem: &str) -> Option<PathBuf> {
    let direct = [
        dir.join(stem).join(format!("{stem}.va")),
        dir.join(format!("{stem}.va")),
    ];
    if let Some(p) = direct.iter().find(|p| p.is_file()) {
        return Some(p.clone());
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path().join(format!("{stem}.va"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

impl Plan {
    /// What converting this technology's ngspice models reads. An error when the
    /// technology has no ngspice models to start from.
    pub fn new(project: &Project, tech: &Tech) -> Result<Plan> {
        let (_, ngspice) = tech.models.set("ngspice").ok_or_else(|| {
            eyre!(
                "the technology '{}' has no ngspice models to make VACASK models from",
                tech.pdk.name
            )
        })?;
        let var = |n: &str| project.var(n);
        let dir = ngspice.dir.resolve(var).map_err(|e| eyre!(e))?;
        let mut notes = vec![];
        let mut wanted: Vec<String> = ngspice
            .libraries
            .iter()
            .chain(&ngspice.includes)
            .cloned()
            .collect();
        for entries in ngspice.corners.values() {
            for e in entries {
                if let Some(f) = e.split_whitespace().next() {
                    wanted.push(f.to_string());
                }
            }
        }
        let used_sections: Vec<(String, String)> = ngspice
            .corners
            .values()
            .flatten()
            .filter_map(|e| {
                let mut w = e.split_whitespace();
                let file = Path::new(w.next()?)
                    .file_name()?
                    .to_string_lossy()
                    .into_owned();
                Some((file, w.next()?.to_string()))
            })
            .collect();
        let mut search: Vec<PathBuf> = vec![dir.clone()];
        for s in &ngspice.search_path {
            if let Ok(p) = s.resolve(var) {
                search.push(p);
            }
        }
        let mut sources: Vec<Source> = vec![];
        let mut seen = BTreeSet::new();
        let mut queue: Vec<(String, PathBuf)> = vec![];
        for w in wanted {
            let p = if Path::new(&w).is_absolute() {
                PathBuf::from(&w)
            } else {
                dir.join(&w)
            };
            queue.push((w, p));
        }
        while let Some((name, path)) = queue.pop() {
            let base = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(name.clone());
            if !seen.insert(base.clone()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                notes.push(format!(
                    "vacask: {name} is named by the ngspice models and is not there ({})",
                    path.display()
                ));
                continue;
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let corner = has_sections(&text);
            if corner {
                // The files a corner file brings in are converted on their own, those
                // of the sections some corner set uses (the statistical ones too: their
                // `gauss()` calls read CDePT's draws, see `stat`).
                let used: BTreeSet<&str> = used_sections
                    .iter()
                    .filter(|(f, _)| *f == base)
                    .map(|(_, s)| s.as_str())
                    .collect();
                for inc in included(&text, Some(&used)) {
                    let found = search
                        .iter()
                        .map(|d| d.join(&inc))
                        .chain(path.parent().map(|d| d.join(&inc)))
                        .find(|p| p.is_file());
                    match found {
                        Some(p) => queue.push((inc, p)),
                        None => {
                            notes.push(format!("vacask: {base} includes {inc}, which is not there"))
                        }
                    }
                }
            }
            sources.push(Source {
                name: base,
                path,
                corner,
            });
        }
        sources.sort_by(|a, b| a.name.cmp(&b.name));
        let va_dir = tech
            .models
            .verilog_a
            .as_ref()
            .and_then(|p| p.resolve(var).ok());
        let mut osdi = vec![];
        for o in &ngspice.osdi {
            let stem = Path::new(o.as_str())
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let va = va_dir.as_ref().and_then(|d| find_va(d, &stem));
            osdi.push((stem, va));
        }
        osdi.sort();
        osdi.dedup();
        // The hash: every source file, every Verilog-A file next to a model, the glue.
        let mut parts: Vec<Vec<u8>> =
            vec![GLUE.as_bytes().to_vec(), tech.pdk.name.as_bytes().to_vec()];
        for s in &sources {
            parts.push(s.name.as_bytes().to_vec());
            parts.push(std::fs::read(&s.path).unwrap_or_default());
        }
        for (stem, va) in &osdi {
            parts.push(stem.as_bytes().to_vec());
            if let Some(va) = va {
                for f in openvaf::source_files(va) {
                    parts.push(
                        f.file_name()
                            .unwrap_or_default()
                            .as_encoded_bytes()
                            .to_vec(),
                    );
                    parts.push(std::fs::read(&f).unwrap_or_default());
                }
            }
        }
        let hash = openvaf::hash_parts(parts.iter().map(|p| p.as_slice()));
        Ok(Plan {
            pdk: tech.pdk.name.clone(),
            sources,
            osdi,
            hash,
            ngspice: ngspice.clone(),
            notes,
        })
    }

    /// The folder this plan generates into.
    pub fn dir(&self, project: &Project) -> PathBuf {
        openvaf::cache_dir(&project.root, &self.pdk, &self.hash)
    }
}

/// The program that runs the converter: `python3`.
fn python() -> Result<PathBuf> {
    crate::tools::find_in_path("python3")
        .or_else(|| crate::tools::find_in_path("python"))
        .ok_or_else(|| eyre!("python3 not found (VACASK's model converter is a Python program)"))
}

/// VACASK's Python folder (`ng2vclib` is in it): `$CDEPT_VACASK_PYTHON`, else
/// `<prefix>/lib/vacask/python` of the VACASK program, else what `vacask -dp` prints.
pub fn converter_dir() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("CDEPT_VACASK_PYTHON").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let vacask = crate::batch::vacask::ADAPTER.require()?;
    // `<prefix>/bin/vacask` (a link to the real one is followed).
    let real = std::fs::canonicalize(&vacask).unwrap_or(vacask.clone());
    for p in [&vacask, &real] {
        if let Some(prefix) = p.parent().and_then(|b| b.parent()) {
            let d = prefix.join("lib/vacask/python");
            if d.join("ng2vclib").is_dir() {
                return Ok(d);
            }
        }
    }
    let out = std::process::Command::new(&vacask)
        .arg("-dp")
        .envs(super::tool_env_for(&vacask))
        .output()
        .wrap_err("cannot run vacask -dp")?;
    let text = String::from_utf8_lossy(&out.stdout);
    for l in text.lines() {
        if let Some(p) = l.strip_prefix("Python path addition:") {
            let d = PathBuf::from(p.trim());
            if d.join("ng2vclib").is_dir() {
                return Ok(d);
            }
        }
    }
    Err(eyre!(
        "VACASK's converter (ng2vclib) was not found next to {}; set CDEPT_VACASK_PYTHON",
        vacask.display()
    ))
}

fn project_path(project: &Project, p: &Path) -> TechPath {
    match p.strip_prefix(&project.root) {
        Ok(rel) => TechPath(format!("{PROJECT_VAR}/{}", rel.display())),
        Err(_) => TechPath(p.display().to_string()),
    }
}

/// Subcircuits of a converted file with a node voltage `v(a,b)` in an instance or
/// parameter expression (a behavioural element): VACASK has no such function, the
/// subcircuit cannot be simulated. Returns the names.
fn behavioural_subckts(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut current: Option<String> = None;
    for l in lines_of(text) {
        let mut w = l.split(|c: char| c.is_whitespace() || c == '(');
        match (w.next(), w.find(|s| !s.is_empty())) {
            (Some("subckt"), Some(name)) => current = Some(name.to_string()),
            (Some("ends"), _) => current = None,
            _ => {}
        }
        if l.starts_with("//") {
            continue;
        }
        let bytes = l.as_bytes();
        let uses_v = l.match_indices("v(").any(|(i, _)| {
            (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
                && l[i + 2..].split(')').next().is_some_and(|a| {
                    a.contains(',') || a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
        });
        if uses_v
            && let Some(c) = &current
            && !out.contains(c)
        {
            out.push(c.clone());
        }
    }
    out
}

/// The first file that section `section` of a converted file includes and that is in
/// `failed` (could not be converted): the section cannot be used then.
fn section_needs(text: &str, section: &str, failed: &BTreeSet<String>) -> Option<String> {
    let mut inside = false;
    for l in lines_of(text) {
        let mut w = l.split_whitespace();
        match w.next() {
            Some("section") => inside = w.next() == Some(section),
            Some("endsection") => inside = false,
            Some("include") if inside => {
                let f = w.next().unwrap_or_default().trim_matches('"');
                let base = Path::new(f)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if failed.contains(&base) {
                    return Some(base);
                }
            }
            _ => {}
        }
    }
    None
}

/// `subckt NAME` and `model NAME` of a converted file.
fn defined_names(text: &str, into: &mut BTreeSet<String>) {
    for l in lines_of(text) {
        let mut w = l.split(|c: char| c.is_whitespace() || c == '(');
        if let (Some(kind), Some(name)) = (w.next(), w.find(|s| !s.is_empty()))
            && matches!(kind, "subckt" | "model")
        {
            into.insert(name.to_string());
        }
    }
}

/// Make (or find in the cache) the VACASK models of the technology.
pub fn generate(project: &Project, tech: &Tech, plan: &Plan) -> Result<Generated> {
    let dir = plan.dir(project);
    let report = dir.join(REPORT);
    if let Ok(text) = std::fs::read_to_string(&report)
        && let Ok(g) = serde_json::from_str::<Generated>(&text)
        && g.hash == plan.hash
    {
        return Ok(g);
    }
    let env = project.tool_env();
    let models = dir.join("models");
    std::fs::create_dir_all(&models)
        .wrap_err_with(|| format!("cannot create {}", models.display()))?;
    let mut notes = plan.notes.clone();
    let prefix = format!("{}_default_mod_", plan.pdk.replace(['-', '.'], "_"));

    // 1. convert the library files.
    let spec = serde_json::json!({
        "python_path": converter_dir()?,
        "prefix": prefix,
        "profile": if plan.pdk.starts_with("ihp-sg13g2") { "ihp-sg13g2" } else { "" },
        "sourcepath": std::iter::once(".".to_string())
            .chain(plan.sources.iter().filter_map(|s| s.path.parent().map(|p| p.display().to_string())))
            .collect::<BTreeSet<_>>(),
        "files": plan.sources.iter().map(|s| serde_json::json!({
            "src": s.path, "dst": models.join(&s.name), "corner": s.corner })).collect::<Vec<_>>(),
        "result": dir.join("convert-result.json"),
    });
    std::fs::write(
        dir.join("convert-spec.json"),
        serde_json::to_vec_pretty(&spec)?,
    )?;
    std::fs::write(dir.join("convert.py"), GLUE)?;
    let mut job = crate::batch::Job::new("ng2vc", &python()?, &dir, "convert.log")
        .args(["-I", "convert.py", "convert-spec.json"])
        .env(&env);
    job.env_defaults = vec![("PYTHONDONTWRITEBYTECODE".into(), "1".into())];
    let out = job.run_unchecked()?;
    out.check()
        .wrap_err("VACASK's model converter did not run")?;
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("convert-result.json"))
            .wrap_err("the model converter wrote no result")?,
    )?;
    let results: Vec<serde_json::Value> = result["files"].as_array().cloned().unwrap_or_default();
    let mut stat = super::stat::MmInfo {
        globals: result["globals"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g.as_str().map(String::from))
            .collect(),
        ..Default::default()
    };
    let mut defined = BTreeSet::new();
    let mut converted = vec![];
    let mut builtin_osdi: BTreeSet<String> = BTreeSet::new();
    let mut default_models: BTreeMap<String, String> = BTreeMap::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    for r in &results {
        let src = r["src"].as_str().unwrap_or_default();
        let name = Path::new(src)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if r["ok"].as_bool() == Some(true) {
            converted.push(name.clone());
            if let Ok(info) = serde_json::from_value::<super::stat::FileInfo>(r.clone()) {
                stat.files.insert(name.clone(), info);
            }
            for o in r["osdi"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| o.as_str())
            {
                builtin_osdi.insert(o.to_string());
            }
            if let Some(m) = r["default_models"].as_object() {
                for (k, v) in m {
                    default_models.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
                }
            }
            if let Ok(text) = std::fs::read_to_string(models.join(&name)) {
                defined_names(&text, &mut defined);
                for sub in behavioural_subckts(&text) {
                    defined.remove(&sub);
                    notes.push(format!(
                        "vacask: {sub} (in {name}) cannot be simulated: it uses a node voltage v() in an expression, a behavioural element VACASK does not have"
                    ));
                }
            }
        } else {
            failed.insert(name.clone());
            let why = r["error"]
                .as_str()
                .unwrap_or("unknown error")
                .replace('\n', " ");
            notes.push(format!("vacask: {name} could not be converted: {why}"));
        }
    }

    // 2. compile the Verilog-A models.
    let mut compiled = vec![];
    let mut osdi_paths: Vec<TechPath> = vec![];
    match openvaf::ADAPTER.require() {
        Err(e) => {
            if !plan.osdi.is_empty() {
                notes.push(format!(
                    "vacask: the Verilog-A models ({}) were not compiled: {e}",
                    plan.osdi
                        .iter()
                        .map(|o| o.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        Ok(program) => {
            let mut env2: Vec<(OsString, OsString)> = super::tool_env_for(&program);
            env2.extend(env.iter().cloned());
            for (stem, va) in &plan.osdi {
                let Some(va) = va else {
                    notes.push(format!("vacask: no Verilog-A source for '{stem}' under the technology's verilog-a folder"));
                    continue;
                };
                let c = openvaf::Compile {
                    source: va.clone(),
                    ..Default::default()
                };
                match openvaf::compile_cached(&program, &c, &dir, &env2) {
                    Ok(r) => {
                        compiled.push(format!("{stem}.osdi"));
                        osdi_paths.push(project_path(project, &r.osdi));
                    }
                    Err(e) => notes.push(format!(
                        "vacask: {stem}.va did not compile: {}",
                        format!("{e:#}").lines().next().unwrap_or("")
                    )),
                }
            }
        }
    }
    for o in &builtin_osdi {
        osdi_paths.push(TechPath(o.clone()));
    }

    // 3. the set.
    let ng = &plan.ngspice;
    let ok = |file: &str| !failed.contains(file) && converted.iter().any(|c| c == file);
    let mut corners = BTreeMap::new();
    for (name, entries) in &ng.corners {
        let mut kept = vec![];
        let mut broken = false;
        for e in entries {
            let file = e.split_whitespace().next().unwrap_or_default();
            let base = Path::new(file)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if ok(&base) {
                // A section that includes a file that failed cannot be used: VACASK would
                // stop at the missing file, so the corner is left out, with the reason.
                let section = e.split_whitespace().nth(1).unwrap_or_default();
                let text = std::fs::read_to_string(models.join(&base)).unwrap_or_default();
                if let Some(missing) = section_needs(&text, section, &failed) {
                    notes.push(format!("vacask: corner set {name} left out: {base} section {section} includes {missing}, which could not be converted"));
                    broken = true;
                    break;
                }
                kept.push(e.clone());
            }
        }
        if !broken {
            corners.insert(name.clone(), kept);
        }
    }
    let mut options = vec![];
    // SOA checks of the PSP wrappers read a global that the ngspice flow defines.
    let uses_swsoa = converted.iter().any(|f| {
        std::fs::read_to_string(models.join(f))
            .is_ok_and(|t| t.contains("swsoa=swsoa") || t.contains("swsoa ="))
    });
    if uses_swsoa {
        options.push("parameters swsoa=0".to_string());
    }
    // Library parameters several files define alike (VACASK refuses redefinitions).
    for (k, v) in result["shared"].as_object().into_iter().flatten() {
        options.push(format!("parameters {k}={}", v.as_str().unwrap_or("0")));
    }
    for (letter, module) in &default_models {
        options.push(format!("model {prefix}{letter} {module}"));
    }
    let set = ModelSet {
        simulator: "vacask".into(),
        generated: Some(format!(
            "from [models.ngspice] with VACASK's ng2vc and OpenVAF, hash {}; made by `cdept vacask models`",
            plan.hash
        )),
        dir: project_path(project, &models),
        libraries: ng.libraries.iter().filter(|l| ok(l)).cloned().collect(),
        includes: ng.includes.iter().filter(|l| ok(l)).cloned().collect(),
        options,
        osdi: osdi_paths,
        tnom: ng.tnom,
        corners,
        ..Default::default()
    };
    let _ = tech;
    let g = Generated {
        hash: plan.hash.clone(),
        pdk: plan.pdk.clone(),
        set,
        defined,
        notes,
        converted,
        compiled,
        stat,
    };
    std::fs::write(&report, serde_json::to_vec_pretty(&g)?)?;
    Ok(g)
}

/// Put a generated set in the snapshot file (`tech.toml` of the project), replacing an
/// older generated set and its `vacask:` notes. A set that is not generated (someone
/// wrote `[models.vacask]` by hand, or a PDK brought one) is left alone.
pub fn record(project: &Project, g: &Generated) -> Result<()> {
    let Some(mut tech) = crate::tech::file::load_tech(&project.root)? else {
        return Err(eyre!("the project has no technology snapshot"));
    };
    if let Some(old) = tech.models.sets.get(SET)
        && old.generated.is_none()
    {
        bail!("[models.vacask] in tech.toml is not a generated set; not replacing it");
    }
    tech.models.sets.insert(SET.into(), g.set.clone());
    tech.notes.retain(|n| !n.starts_with("vacask:"));
    tech.notes.extend(g.notes.iter().cloned());
    let text = crate::tech::file::tech_text(&tech)?;
    std::fs::write(crate::tech::file::tech_file(&project.root), text)?;
    Ok(())
}

/// What a run needs: the snapshot with a `vacask` model set in it (made and recorded
/// when it is missing or older than the PDK's files), and the names the set defines.
pub struct Ready {
    pub tech: Tech,
    /// Names defined by the converted files; `None` for a set that is not generated.
    pub defined: Option<BTreeSet<String>>,
    /// What the statistical models draw (empty for a set that is not generated).
    pub stat: super::stat::MmInfo,
}

pub fn ready(project: &Project) -> Result<Ready> {
    // The tests of a suite ask at once: one makes the models, the others wait for them.
    static GENERATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = GENERATING.lock().unwrap_or_else(|e| e.into_inner());
    let tech = project
        .tech
        .as_ref()
        .ok_or_else(|| eyre!("the project has no technology snapshot (cdept tech scan)"))?;
    if let Some(set) = tech.models.sets.get(SET)
        && set.generated.is_none()
    {
        return Ok(Ready {
            tech: tech.clone(),
            defined: None,
            stat: Default::default(),
        });
    }
    let plan = Plan::new(project, tech)?;
    let g = generate(project, tech, &plan)?;
    if g.set.corners.is_empty() {
        let why: Vec<&str> = g.notes.iter().take(6).map(String::as_str).collect();
        return Err(eyre!(
            "the technology '{}' has no usable VACASK models: none of the ngspice corner sets converted\n  {}\n  (all of them: `cdept vacask models`)",
            g.pdk,
            why.join("\n  ")
        ));
    }
    let mut tech = tech.clone();
    if tech.models.sets.get(SET) != Some(&g.set) {
        if let Err(e) = record(project, &g) {
            tracing::warn!("cannot record the VACASK models in tech.toml: {e:#}");
        }
        tech.models.sets.insert(SET.into(), g.set.clone());
    }
    tech.notes.retain(|n| !n.starts_with("vacask:"));
    tech.notes.extend(g.notes.iter().cloned());
    Ok(Ready {
        tech,
        defined: Some(g.defined),
        stat: g.stat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_and_includes() {
        let corner = "* c\n.lib mos_tt\n.param a=1\n.include sg13g2_moslv_mod.lib\n.endl\n.lib mos_ff\n.include \"x.lib\"\n.endl\n";
        assert!(has_sections(corner));
        assert!(!has_sections(".lib /a/b.lib tt\n.include y\n"));
        assert_eq!(included(corner, None), ["sg13g2_moslv_mod.lib", "x.lib"]);
        let only: BTreeSet<&str> = ["mos_ff"].into();
        assert_eq!(included(corner, Some(&only)), ["x.lib"]);
        assert_eq!(
            included(".lib other.lib tt\n.lib tt2\n", None),
            ["other.lib"]
        );
    }

    #[test]
    fn collects_defined_names() {
        let mut s = BTreeSet::new();
        defined_names(
            "subckt sg13_lv_nmos(d g s b)\nmodel sg13g2_lv_nmos_psp psp103va (\n  type=1\nsubckt  x2 (a)\n// subckt no\n",
            &mut s,
        );
        assert_eq!(
            s.iter().cloned().collect::<Vec<_>>(),
            ["sg13_lv_nmos", "sg13g2_lv_nmos_psp", "x2"]
        );
    }

    #[test]
    fn a_section_that_needs_a_failed_file_is_found() {
        let text = "section tt\n  parameters a=1\n  include \"models_fet.spice\"\nendsection\nsection ff\n  include \"ok.lib\"\nendsection\n";
        let failed: BTreeSet<String> = ["models_fet.spice".to_string()].into();
        assert_eq!(
            section_needs(text, "tt", &failed).as_deref(),
            Some("models_fet.spice")
        );
        assert_eq!(section_needs(text, "ff", &failed), None);
    }

    #[test]
    fn finds_behavioural_subcircuits() {
        let text = "subckt good(a b)\nr1 (a b) res r=1k\nends\nsubckt bad(1 2 3)\nnr (1 2) rm (\n r=rw*(v(2,4))+3\n)\nends\n// subckt x\n";
        assert_eq!(behavioural_subckts(text), ["bad"]);
        assert!(behavioural_subckts("subckt s(a)\nparameters vth=1 dv(x)=2\nends\n").is_empty());
    }

    /// The converter on canned model text (needs VACASK's ng2vc and python3: skipped
    /// without): `gauss()`/`agauss()` become reads of the draw vectors, a parameter that
    /// two files define alike moves into the shared list.
    #[test]
    fn the_converter_turns_gauss_calls_into_draw_reads() {
        let (Ok(py), Some(python)) = (converter_dir(), crate::tools::find_in_path("python3"))
        else {
            eprintln!("skipped: VACASK's converter or python3 not found");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("dev.lib"),
            ".subckt dev d s w=1u mm_ok=0\n.param rr='agauss(1k, 10, (mm_ok != 1 ? 0 : 1))'\nR1 d s r=rr\n.ends dev\n\
             .param nsig=1\n.param gp='gauss(2, 0.1, nsig)'\n",
        )
        .unwrap();
        std::fs::write(
            src.join("other.lib"),
            ".param nsig=1\n.param gq='agauss(0, 1, 1)'\n",
        )
        .unwrap();
        let out = dir.path().join("out");
        let files: Vec<_> = ["dev.lib", "other.lib"]
            .iter()
            .map(|f| serde_json::json!({ "src": src.join(f), "dst": out.join(f), "corner": false }))
            .collect();
        let spec = serde_json::json!({ "python_path": py, "prefix": "t_", "profile": "", "sourcepath": [src],
            "files": files, "result": dir.path().join("result.json") });
        std::fs::write(dir.path().join("spec.json"), spec.to_string()).unwrap();
        std::fs::write(dir.path().join("convert.py"), GLUE).unwrap();
        let status = std::process::Command::new(python)
            .current_dir(dir.path())
            .args(["-I", "convert.py", "spec.json"])
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .status()
            .unwrap();
        assert!(status.success());
        let result: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("result.json")).unwrap())
                .unwrap();
        assert_eq!(
            result["globals"],
            serde_json::json!(["gp", "gq"]),
            "{result}"
        );
        assert_eq!(
            result["shared"],
            serde_json::json!({ "nsig": "1" }),
            "{result}"
        );
        let dev = std::fs::read_to_string(out.join("dev.lib")).unwrap();
        assert!(dev.contains("parameters mmk=-1"), "{dev}");
        assert!(
            dev.contains("mmz[mmk*mmnz+0]") && dev.contains("||mmk<0"),
            "{dev}"
        );
        assert!(!dev.contains("!="), "{dev}");
        assert!(
            dev.contains("mcz[0]") && !dev.contains("parameters nsig"),
            "{dev}"
        );
        let files = result["files"].as_array().unwrap();
        assert_eq!(files[0]["mm"]["dev"], serde_json::json!(["rr"]));
        let other = std::fs::read_to_string(out.join("other.lib")).unwrap();
        assert!(other.contains("mcz[1]"), "{other}");
    }

    #[test]
    fn finds_the_verilog_a_of_a_model() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("psp103")).unwrap();
        std::fs::write(d.path().join("psp103/psp103_nqs.va"), "").unwrap();
        std::fs::write(d.path().join("r.va"), "").unwrap();
        assert!(
            find_va(d.path(), "psp103_nqs")
                .unwrap()
                .ends_with("psp103/psp103_nqs.va")
        );
        assert!(find_va(d.path(), "r").unwrap().ends_with("r.va"));
        assert!(find_va(d.path(), "nothing").is_none());
    }
}
