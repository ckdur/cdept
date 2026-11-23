//! Monte Carlo for VACASK: the statistical models of the technology, with CDePT's own
//! seeded draws.
//!
//! VACASK has no random numbers, and the PDKs' statistical models call `gauss()` and
//! `agauss()` in parameter expressions. The converter (`convert.py`) turns each call
//! into an expression that reads a draw from a vector the deck defines:
//!
//! - a call outside any subcircuit (process variation: `.param mc_x = gauss(...)`)
//!   reads `mcz[slot]`, one draw per technology-wide parameter and run;
//! - a call inside a subcircuit (mismatch: `w = agauss(w, dw, (mm_ok != 1 ? 0 : 1))`)
//!   reads `mmz[mmk*mmnz + slot]`: `mmk` is the *key* of the instance (a parameter of
//!   the subcircuit, -1 = no draws, the nominal value), `slot` the call's number inside
//!   the subcircuit, `mmnz` the slots per key.
//!
//! [`instrument`] gives every device instance of a run its own key: the keys are the
//! leaf devices counted in depth-first order through the testbench's own subcircuits
//! (each user subcircuit gets a `mmk` offset parameter), so every device of the
//! flattened circuit draws on its own, also when a cell is instantiated twice. The
//! draws are made here ([`normal`]) from a counter-based generator of (run seed,
//! stream, index): the same seed gives the same numbers on every machine and whatever
//! the number of devices. A call whose third argument (the number of sigmas) is 0, like
//! `mm_ok` of an instance that is not marked for mismatch, gives the nominal value, as
//! in ngspice.
//!
//! ngspice's draws are different numbers (its own generator, and it draws at other
//! moments), so a run of ngspice and a run of VACASK with the same seed are not the same
//! circuit: only the statistics agree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the converter found in one converted file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    /// The subcircuits the file defines.
    pub defs: Vec<String>,
    /// Subcircuits with per-instance draws: their calls' parameter names, in slot order.
    #[serde(default)]
    pub mm: BTreeMap<String, Vec<String>>,
    /// The technology-wide draws the file uses.
    #[serde(default)]
    pub globals: Vec<String>,
}

/// The statistical content of a generated model set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MmInfo {
    /// By file name.
    pub files: BTreeMap<String, FileInfo>,
    /// The technology-wide draws, `mcz[index]`.
    pub globals: Vec<String>,
}

/// The model lines of a run point after "the first definition of a subcircuit wins" and
/// what the effective files bring.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub lines: Vec<String>,
    /// Subcircuits with per-instance draws (name, slots).
    pub masters: BTreeMap<String, usize>,
    /// Some included file reads `mcz` or `mmz` (the deck must define them).
    pub uses_draws: bool,
}

fn base(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The files `section` of a converted corner file includes, in order.
pub fn section_includes(text: &str, section: &str) -> Vec<String> {
    let mut inside = false;
    let mut out = vec![];
    for l in text.lines() {
        let t = l.trim();
        let mut w = t.split_whitespace();
        match w.next() {
            Some("section") => inside = w.next() == Some(section),
            Some("endsection") => inside = false,
            Some("include") if inside => {
                out.push(w.next().unwrap_or_default().trim_matches('"').to_string());
            }
            _ => {}
        }
    }
    out
}

/// The lines of `section` of a converted corner file that are not includes, as written.
fn section_body(text: &str, section: &str) -> Vec<String> {
    let mut inside = false;
    let mut out = vec![];
    for l in text.lines() {
        let t = l.trim();
        let mut w = t.split_whitespace();
        match w.next() {
            Some("section") => inside = w.next() == Some(section),
            Some("endsection") => inside = false,
            Some("include") => {}
            Some(_) if inside => out.push(t.to_string()),
            _ => {}
        }
    }
    out
}

/// Apply "the first definition of a subcircuit wins" (what ngspice does) to the model
/// lines (`.include path`, `.lib path section`, anything else stays): an include whose
/// subcircuits an earlier line defined is dropped, and a corner section that includes
/// such a file is written out line by line without it. Then collect the per-instance
/// draws of the files that remain.
pub fn resolve(lines: &[String], info: &MmInfo) -> Resolved {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = vec![];
    let mut effective: Vec<String> = vec![];
    let defs_of = |file: &str| {
        info.files
            .get(&base(file))
            .map(|f| f.defs.clone())
            .unwrap_or_default()
    };
    for line in lines {
        let t = line.trim();
        let (card, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
        let rest = rest.trim();
        match card.to_lowercase().as_str() {
            ".include" | ".inc" => {
                let file = rest.trim_matches('"');
                let defs = defs_of(file);
                if defs.iter().any(|d| seen.contains(d)) {
                    continue;
                }
                seen.extend(defs);
                effective.push(base(file));
                out.push(line.clone());
            }
            ".lib" => {
                let (file, section) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
                    Some((f, s)) => (f.to_string(), s.trim().to_string()),
                    None => {
                        let (f, s) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                        (f.to_string(), s.trim().to_string())
                    }
                };
                let Ok(text) = std::fs::read_to_string(&file) else {
                    out.push(line.clone());
                    continue;
                };
                let dir = Path::new(&file)
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                let incs = section_includes(&text, &section);
                let clash = incs
                    .iter()
                    .any(|f| defs_of(f).iter().any(|d| seen.contains(d)));
                if !clash {
                    for f in &incs {
                        seen.extend(defs_of(f));
                        effective.push(base(f));
                    }
                    out.push(line.clone());
                    continue;
                }
                // Written out: the section's own lines, then the includes that remain.
                for l in section_body(&text, &section) {
                    out.push(format!(".vcline {l}"));
                }
                for f in &incs {
                    let defs = defs_of(f);
                    if defs.iter().any(|d| seen.contains(d)) {
                        continue;
                    }
                    seen.extend(defs);
                    effective.push(base(f));
                    let p: PathBuf = if Path::new(f).is_absolute() {
                        f.into()
                    } else {
                        dir.join(f)
                    };
                    out.push(format!(".include {}", p.display()));
                }
            }
            _ => out.push(line.clone()),
        }
    }
    let mut masters = BTreeMap::new();
    let mut uses_draws = false;
    for f in &effective {
        if let Some(info) = info.files.get(f) {
            for (s, labels) in &info.mm {
                masters.insert(s.clone(), labels.len());
            }
            uses_draws |= !info.mm.is_empty() || !info.globals.is_empty();
        }
    }
    Resolved {
        lines: out,
        masters,
        uses_draws,
    }
}

fn splitmix(z: u64) -> u64 {
    let mut z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A standard normal draw for (run seed, stream, index): Box-Muller on two uniform
/// numbers hashed from the three, so a draw does not depend on any other.
pub fn normal(seed: u64, stream: u64, index: u64) -> f64 {
    let h = splitmix(splitmix(splitmix(seed) ^ stream.wrapping_mul(0xD6E8_FEB8_6659_FD93)) ^ index);
    let unit = |x: u64| ((x >> 11) as f64 + 0.5) / 9_007_199_254_740_992.0;
    let (u1, u2) = (unit(h), unit(splitmix(h ^ 0xA5A5_A5A5_5A5A_5A5A)));
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// The stream of the technology-wide draws and the one of the per-instance draws.
const GLOBAL_STREAM: u64 = 1;
const INSTANCE_STREAM: u64 = 2;

fn vector(len: usize, f: impl Fn(u64) -> f64) -> String {
    if len == 0 {
        return "[0.0]".into();
    }
    format!(
        "[{}]",
        (0..len as u64)
            .map(|i| format!("{:.7e}", f(i)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The deck lines that define the draw vectors. Without a `seed` (a run that is not a
/// Monte Carlo point) every draw is 0: the nominal circuit.
pub fn draw_lines(seed: Option<u64>, nz: usize, leaves: usize, globals: usize) -> Vec<String> {
    let mut out = vec![format!(
        "// Monte Carlo draws{}",
        seed.map(|s| format!(" (seed {s})")).unwrap_or_default()
    )];
    out.push(format!("parameters mmnz={}", nz.max(1)));
    match seed {
        Some(s) => {
            out.push(format!(
                "parameters mcz={}",
                vector(globals, |i| normal(s, GLOBAL_STREAM, i))
            ));
            out.push(format!(
                "parameters mmz={}",
                vector(leaves * nz.max(1), |i| normal(s, INSTANCE_STREAM, i))
            ));
        }
        None => {
            out.push(format!("parameters mcz=vector({}, 0.0)", globals.max(1)));
            out.push("parameters mmz=vector(1, 0.0)".into());
        }
    }
    out
}

/// A line `name (nodes) master ...` of a VACASK netlist: the master.
fn master_of(line: &str) -> Option<&str> {
    let t = line.trim();
    let (first, rest) = t.split_once(char::is_whitespace)?;
    if first == "subckt"
        || first == "model"
        || first == "parameters"
        || first == "global"
        || first == "ground"
    {
        return None;
    }
    let rest = rest.trim_start();
    let close = rest.strip_prefix('(')?.find(')')?;
    rest[close + 2..].split_whitespace().next()
}

/// The instances (name, master) of each subcircuit of some VACASK texts, by lower-case
/// subcircuit name, and the ones outside any subcircuit under the empty name.
pub fn structure(texts: &[&str]) -> BTreeMap<String, Vec<(String, String)>> {
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for text in texts {
        let mut current = String::new();
        for l in text.lines() {
            if let Some(name) = header_name(l) {
                current = name.to_lowercase();
                out.entry(current.clone()).or_default();
            } else if l.trim() == "ends" || l.trim().starts_with("ends ") {
                current.clear();
            } else if let Some(m) = master_of(l) {
                let name = l.split_whitespace().next().unwrap_or_default().to_string();
                out.entry(current.clone())
                    .or_default()
                    .push((name, m.to_string()));
            }
        }
    }
    out
}

/// The parameters each subcircuit of some VACASK texts declares (`parameters a=1 b=2`
/// right inside it), by lower-case subcircuit name.
fn declared_params(texts: &[&str]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for text in texts {
        let mut current: Option<String> = None;
        for l in text.lines() {
            if let Some(name) = header_name(l) {
                current = Some(name.to_lowercase());
                out.entry(name.to_lowercase()).or_default();
            } else if l.trim() == "ends" || l.trim().starts_with("ends ") {
                current = None;
            } else if let (Some(c), Some(rest)) = (&current, l.trim().strip_prefix("parameters ")) {
                for tok in rest.split_whitespace() {
                    if let Some((k, _)) = tok.split_once('=') {
                        out.get_mut(c).unwrap().push(k.to_string());
                    }
                }
            }
        }
    }
    out
}

/// SPICE has no case and VACASK has: an instance of a model subcircuit that writes `Nx=1`
/// for a parameter the converted subcircuit declares as `nx` (ng2vc lower-cases parameter
/// names) is given the declared name.
pub fn fix_param_case(netlist: &str, model_texts: &[&str]) -> String {
    let declared = declared_params(model_texts);
    let mut out = String::with_capacity(netlist.len());
    for l in netlist.lines() {
        let fixed = master_of(l)
            .and_then(|m| declared.get(&m.to_lowercase()))
            .map(|params| {
                let mut parts: Vec<String> = vec![];
                let mut after_master = false;
                let master = master_of(l).unwrap_or_default();
                for tok in l.split(' ') {
                    if after_master && let Some((k, v)) = tok.split_once('=') {
                        let actual = params
                            .iter()
                            .find(|p| p.eq_ignore_ascii_case(k))
                            .map_or(k, String::as_str);
                        parts.push(format!("{actual}={v}"));
                        continue;
                    }
                    if tok == master {
                        after_master = true;
                    }
                    parts.push(tok.to_string());
                }
                parts.join(" ")
            });
        out.push_str(fixed.as_deref().unwrap_or(l));
        out.push('\n');
    }
    out
}

/// ngspice's `@n.xdut.xm1.nsg13_lv_pmos[vds]` (a device's parameter or operating-point
/// value) as VACASK's `p('xdut:xm1:nsg13_lv_pmos', vds)`: the names are looked up in the
/// netlist and the model files without regard to case (SPICE has none, VACASK has).
pub fn device_save(
    item: &str,
    structure: &BTreeMap<String, Vec<(String, String)>>,
) -> Result<String, String> {
    let t = item.trim().trim_start_matches('@');
    let (path, var) = t
        .strip_suffix(']')
        .and_then(|r| r.split_once('['))
        .ok_or_else(|| format!("cannot read the save item '{item}'"))?;
    let mut parts: Vec<&str> = path.split('.').collect();
    // ngspice writes the device letter before the path of a device inside subcircuits.
    if parts.len() > 1 && parts[0].len() == 1 {
        parts.remove(0);
    }
    let mut names = vec![];
    let mut scope = String::new();
    for (i, want) in parts.iter().enumerate() {
        let found = structure
            .get(&scope)
            .and_then(|list| list.iter().find(|(n, _)| n.eq_ignore_ascii_case(want)))
            .ok_or_else(|| {
                format!(
                    "'{item}': there is no instance '{want}' in {}",
                    if scope.is_empty() {
                        "the netlist".to_string()
                    } else {
                        format!("'{scope}'")
                    }
                )
            })?;
        names.push(found.0.clone());
        if i + 1 < parts.len() {
            scope = found.1.to_lowercase();
        }
    }
    Ok(format!("p('{}', {})", names.join(":"), var.trim()))
}

fn header_name(line: &str) -> Option<String> {
    let t = line.trim().strip_prefix("subckt")?;
    if !t.starts_with(char::is_whitespace) {
        return None;
    }
    let t = t.trim_start();
    Some(
        t.split(|c: char| c.is_whitespace() || c == '(')
            .next()?
            .to_string(),
    )
}

/// Give the devices of a translated netlist their keys: every instance of a subcircuit
/// in `masters` (and of a user subcircuit that holds some) gets `mmk`, the leaf devices
/// counted depth-first. Returns the new text and the number of leaf devices (keys).
pub fn instrument(text: &str, masters: &BTreeMap<String, usize>) -> (String, usize) {
    let lines: Vec<&str> = text.lines().collect();
    // The user subcircuits and the instance lines in each.
    let mut users: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for (n, l) in lines.iter().enumerate() {
        if let Some(name) = header_name(l) {
            users.entry(name.clone()).or_default();
            current = Some(name);
        } else if l.trim() == "ends" || l.trim().starts_with("ends ") {
            current = None;
        } else if let (Some(c), Some(_)) = (&current, master_of(l)) {
            users.get_mut(c).unwrap().push(n);
        }
    }
    fn leaves(
        name: &str,
        masters: &BTreeMap<String, usize>,
        users: &BTreeMap<String, Vec<usize>>,
        lines: &[&str],
        memo: &mut BTreeMap<String, usize>,
        depth: usize,
    ) -> usize {
        if masters.contains_key(name) {
            return 1;
        }
        if let Some(n) = memo.get(name) {
            return *n;
        }
        let Some(insts) = users.get(name) else {
            return 0;
        };
        if depth > 32 {
            return 0;
        }
        let n = insts
            .iter()
            .filter_map(|i| master_of(lines[*i]))
            .map(|m| leaves(m, masters, users, lines, memo, depth + 1))
            .sum();
        memo.insert(name.to_string(), n);
        n
    }
    let mut memo = BTreeMap::new();
    let count =
        |m: &str, memo: &mut BTreeMap<String, usize>| leaves(m, masters, &users, &lines, memo, 0);
    let mut out = String::new();
    let mut current: Option<String> = None;
    let mut offset = 0usize;
    for l in &lines {
        if let Some(name) = header_name(l) {
            offset = 0;
            out.push_str(l);
            out.push('\n');
            if count(&name, &mut memo) > 0 {
                out.push_str("  parameters mmk=-1\n");
            }
            current = Some(name);
            continue;
        }
        if l.trim() == "ends" || l.trim().starts_with("ends ") {
            current = None;
            offset = 0;
            out.push_str(l);
            out.push('\n');
            continue;
        }
        if current.is_none() && l.trim().starts_with("subckt") {
            out.push_str(l);
            out.push('\n');
            continue;
        }
        match master_of(l).map(|m| count(m, &mut memo)) {
            Some(n) if n > 0 => {
                let key = match (&current, offset) {
                    (None, o) => format!("{o}"),
                    (Some(_), 0) => "mmk".to_string(),
                    (Some(_), o) => format!("(mmk<0?-1:mmk+{o})"),
                };
                out.push_str(&format!("{l} mmk={key}\n"));
                offset += n;
            }
            _ => {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    // The top level's leaf count is the offset after its last line; a subcircuit's
    // offset was reset at `ends`, so count the top level separately.
    let mut top = 0usize;
    let mut in_sub = false;
    for l in &lines {
        if header_name(l).is_some() {
            in_sub = true;
        } else if l.trim() == "ends" || l.trim().starts_with("ends ") {
            in_sub = false;
        } else if !in_sub && let Some(m) = master_of(l) {
            top += count(m, &mut memo);
        }
    }
    (out, top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_are_deterministic_and_standard_normal() {
        assert_eq!(normal(7, 2, 5), normal(7, 2, 5));
        assert_ne!(normal(7, 2, 5), normal(8, 2, 5));
        assert_ne!(normal(7, 2, 5), normal(7, 1, 5));
        let n = 20000;
        let xs: Vec<f64> = (0..n).map(|i| normal(42, 2, i)).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.03, "{mean}");
        assert!((var - 1.0).abs() < 0.05, "{var}");
        // About 0.27 % beyond 3 sigma, both sides.
        let tails = xs.iter().filter(|x| x.abs() > 3.0).count();
        assert!(tails < 3 * n as usize / 100 / 10 + 40, "{tails}");
    }

    #[test]
    fn a_draw_does_not_depend_on_the_size_of_the_table() {
        let a = draw_lines(Some(3), 4, 2, 5);
        let b = draw_lines(Some(3), 4, 10, 5);
        let first = |v: &Vec<String>| {
            v.iter()
                .find(|l| l.starts_with("parameters mmz="))
                .unwrap()
                .clone()
        };
        let (fa, fb) = (first(&a), first(&b));
        let head = |s: &str| {
            s.trim_start_matches("parameters mmz=[")
                .split(", ")
                .take(7)
                .collect::<Vec<_>>()
                .join(", ")
        };
        assert_eq!(head(&fa), head(&fb));
        assert!(
            draw_lines(None, 4, 2, 5)
                .iter()
                .any(|l| l.contains("vector(5, 0.0)"))
        );
    }

    const NET: &str = "global VDD\nmodel cdept_c sp_capacitor\nsubckt inv (A Y VDD VSS)\n  XM2 (VDD A Y VDD) pm w=1e-6 mm_ok=1\n  XM1 (Y A VSS VSS) nm w=1e-6 mm_ok=1\nends\n\
subckt buf (A Y VDD VSS)\n  x1 (A m VDD VSS) inv\n  x2 (m Y VDD VSS) inv\nends\n\
x1 (in out VDD 0) buf\nXM9 (a b c d) nm w=1e-6\nC1 (out 0) cdept_c c=10e-15\n";

    #[test]
    fn every_device_of_the_flattened_circuit_gets_its_own_key() {
        let masters: BTreeMap<String, usize> =
            [("pm".to_string(), 4), ("nm".to_string(), 4)].into();
        let (text, leaves) = instrument(NET, &masters);
        assert_eq!(leaves, 5, "{text}");
        for want in [
            "subckt inv (A Y VDD VSS)\n  parameters mmk=-1\n",
            "  XM2 (VDD A Y VDD) pm w=1e-6 mm_ok=1 mmk=mmk\n",
            "  XM1 (Y A VSS VSS) nm w=1e-6 mm_ok=1 mmk=(mmk<0?-1:mmk+1)\n",
            "subckt buf (A Y VDD VSS)\n  parameters mmk=-1\n",
            "  x1 (A m VDD VSS) inv mmk=mmk\n",
            "  x2 (m Y VDD VSS) inv mmk=(mmk<0?-1:mmk+2)\n",
            "x1 (in out VDD 0) buf mmk=0\n",
            "XM9 (a b c d) nm w=1e-6 mmk=4\n",
        ] {
            assert!(text.contains(want), "{want}\n{text}");
        }
        assert!(!text.contains("C1 (out 0) cdept_c c=10e-15 mmk"), "{text}");
        // Nothing to key: the text stays.
        let (same, n) = instrument(NET, &BTreeMap::new());
        assert_eq!((same.trim_end(), n), (NET.trim_end(), 0));
    }

    #[test]
    fn parameter_names_follow_the_case_of_the_converted_subcircuit() {
        let model = "subckt npn13G2 (c b e s)\nparameters nx=1\nparameters mm_ok=0\nends\n";
        let net = "XQ1 (c b 0 0) npn13G2 Nx=2 mm_ok=1\nXQ2 (c b 0 0) other Nx=2\n";
        let out = fix_param_case(net, &[model]);
        assert_eq!(
            out,
            "XQ1 (c b 0 0) npn13G2 nx=2 mm_ok=1\nXQ2 (c b 0 0) other Nx=2\n"
        );
    }

    #[test]
    fn ngspice_device_saves_become_instance_outvar_pairs() {
        let model = "subckt pm (d g s b)\n  Npm_core (d g s b) psp w=1\nends\n";
        let st = structure(&[NET, model]);
        assert_eq!(
            device_save("@n.x1.x1.xm2.npm_core[vds]", &st).unwrap(),
            "p('x1:x1:XM2:Npm_core', vds)"
        );
        assert!(device_save("@n.x1.nothing.npm_core[vds]", &st).is_err());
        assert!(device_save("v(out)", &st).is_err());
    }

    #[test]
    fn the_first_definition_of_a_subcircuit_wins() {
        let dir = tempfile::tempdir().unwrap();
        let corner = dir.path().join("corner.lib");
        std::fs::write(
            &corner,
            "section stat\n  parameters a=1\ninclude \"mod.lib\"\ninclude \"stat.lib\"\nendsection\nsection tt\ninclude \"mod.lib\"\nendsection\n",
        )
        .unwrap();
        let mut info = MmInfo::default();
        info.files.insert(
            "mm.lib".into(),
            FileInfo {
                defs: vec!["nmos".into()],
                mm: [("nmos".to_string(), vec!["w".into(), "l".into()])].into(),
                globals: vec![],
            },
        );
        info.files.insert(
            "mod.lib".into(),
            FileInfo {
                defs: vec!["nmos".into()],
                ..Default::default()
            },
        );
        info.files.insert(
            "stat.lib".into(),
            FileInfo {
                defs: vec![],
                mm: Default::default(),
                globals: vec!["g".into()],
            },
        );
        let c = corner.display().to_string();
        let lines = vec![
            format!(".include {}/mm.lib", dir.path().display()),
            format!(".lib {c} stat"),
            ".vcline parameters x=1".to_string(),
        ];
        let r = resolve(&lines, &info);
        assert_eq!(r.masters["nmos"], 2);
        assert!(r.uses_draws);
        assert!(
            r.lines.contains(&".vcline parameters a=1".to_string()),
            "{:?}",
            r.lines
        );
        assert!(
            r.lines
                .iter()
                .any(|l| l.starts_with(".include ") && l.ends_with("stat.lib")),
            "{:?}",
            r.lines
        );
        assert!(
            !r.lines.iter().any(|l| l.ends_with("mod.lib")),
            "{:?}",
            r.lines
        );
        assert_eq!(r.lines.last().unwrap(), ".vcline parameters x=1");
        // Without the mismatch file the section stays one line and nothing keys.
        let r = resolve(&[format!(".lib {c} tt")], &info);
        assert_eq!(r.lines, [format!(".lib {c} tt")]);
        assert!(r.masters.is_empty() && !r.uses_draws);
    }
}
