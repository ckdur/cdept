//! Netgen section of the technology snapshot: `libs.tech/netgen`.
//!
//! The LVS set-up file is Tcl. Read from it, without running it: the device groups
//! it sets rules for (`lappend devices …` followed by a loop over them: which pins
//! may be swapped, whether devices merge in series or parallel, which properties
//! are compared with which tolerance, which are dropped), the cell name patterns it
//! treats specially and the environment variables it looks at.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pdk::{ScanContext, Scanned, TechPath};

/// `[netgen]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetgenTech {
    /// The LVS set-up file (`<pdk>_setup.tcl`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<TechPath>,
    /// Other set-up files in the folder that differ from `setup`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_setups: Vec<TechPath>,
    /// Environment variables the set-up file looks at (`NETGEN_COLUMNS`,
    /// `MAGIC_EXT_USE_GDS`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub environment: Vec<String>,
}

/// The netgen part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetgenCatalog {
    /// Commands the set-up file gives for everything (`permute default`).
    pub defaults: Vec<String>,
    /// The device groups in the order of the file.
    pub groups: Vec<DeviceGroup>,
    /// Rules for cells by name pattern (decap, fill and tap cells of digital
    /// libraries).
    pub patterns: Vec<PatternRule>,
}

/// Devices the set-up file gives the same rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceGroup {
    /// The comment heading the group (`Resistors (except metal)`).
    pub title: String,
    pub devices: Vec<String>,
    /// Pins that may be swapped, as written (`1 2`, `c1 c0`).
    pub permute: Vec<String>,
    /// Devices in series are combined.
    pub series: bool,
    /// Devices in parallel are combined.
    pub parallel: bool,
    /// How properties are treated when combining (`series {w critical}`).
    pub combine: Vec<String>,
    /// Properties compared with a tolerance (`l 0.01`).
    pub tolerance: Vec<String>,
    /// Properties that are not compared.
    pub delete: Vec<String>,
    /// The devices are left out of the comparison (`ignore class`).
    pub ignore: bool,
}

/// A rule for cells whose name matches a regular expression.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternRule {
    pub pattern: String,
    /// The command given for them, without the cell (`property parallel enable`,
    /// `ignore class`).
    pub rule: String,
}

/// What a set-up file says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Setup {
    pub catalog: NetgenCatalog,
    pub environment: Vec<String>,
}

/// The words in braces or bare after `skip` words: `{l 0.01} {w 0.01}` → [`l 0.01`,
/// `w 0.01`]; `b` → [`b`].
fn items(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('{') {
            match inner.find('}') {
                Some(end) => {
                    out.push(inner[..end].trim().to_string());
                    rest = inner[end + 1..].trim_start();
                }
                None => break,
            }
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push(rest[..end].to_string());
            rest = rest[end..].trim_start();
        }
    }
    out.retain(|i| !i.is_empty());
    out
}

fn push_once(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// `$env(NAME)` and `$::env(NAME)` in a line.
fn environment_in(line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(at) = rest.find("env(") {
        let inner = &rest[at + 4..];
        let Some(end) = inner.find(')') else {
            break;
        };
        let name = &inner[..end];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            push_once(out, name.to_string());
        }
        rest = &inner[end..];
    }
}

/// Read a set-up file.
pub fn parse(text: &str) -> Setup {
    let mut setup = Setup::default();
    let mut title = String::new();
    // The last comment block: its first line with words is the title.
    let mut block: Vec<String> = Vec::new();
    let mut in_group = false;
    let mut pattern: Option<String> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if let Some(comment) = line.strip_prefix('#') {
            let comment = comment.trim_matches(|c: char| c == '#' || c == '-' || c.is_whitespace());
            if comment.chars().any(char::is_alphabetic) {
                block.push(comment.to_string());
            }
            continue;
        }
        if line.is_empty() {
            if let Some(first) = block.first() {
                title = first.clone();
            }
            block.clear();
            continue;
        }
        if let Some(first) = block.first() {
            // A comment directly above code inside a loop is a remark, not a title.
            if !in_group || line.starts_with("set devices") {
                title = first.clone();
            }
        }
        block.clear();
        environment_in(line, &mut setup.environment);

        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["set", "devices", rest @ ..] => {
                let listed = rest.join(" ");
                let listed = listed.trim_matches(|c| c == '{' || c == '}');
                setup.catalog.groups.push(DeviceGroup {
                    title: title.clone(),
                    devices: listed.split_whitespace().map(str::to_string).collect(),
                    ..DeviceGroup::default()
                });
                in_group = true;
                pattern = None;
                continue;
            }
            ["lappend", "devices", rest @ ..] => {
                if let Some(group) = setup.catalog.groups.last_mut() {
                    for device in rest {
                        push_once(&mut group.devices, device.to_string());
                    }
                }
                continue;
            }
            ["permute", "default"]
            | ["property", "default"]
            | ["property", "parallel", _]
            | ["property", "series", _] => {
                push_once(&mut setup.catalog.defaults, line.to_string());
                continue;
            }
            _ => {}
        }

        // `if {[regexp {pattern} $cell match]} {`
        if let Some(at) = line.find("[regexp ") {
            let rest = line[at + "[regexp ".len()..].trim_start();
            pattern = if let Some(inner) = rest.strip_prefix('{') {
                inner.find('}').map(|end| inner[..end].to_string())
            } else if let Some(inner) = rest.strip_prefix('"') {
                inner.find('"').map(|end| inner[..end].to_string())
            } else {
                None
            };
            in_group = false;
            continue;
        }
        if line.starts_with("foreach cell") || line.starts_with("foreach dev $cells") {
            in_group = false;
        }

        // The end of the block a pattern stands for.
        if line == "}" {
            pattern = None;
            continue;
        }

        // Commands on a device or cell: `property "-circuit1 $dev" series enable`.
        let (Some(command), Some(open)) = (words.first(), line.find('"')) else {
            continue;
        };
        if !matches!(*command, "permute" | "property" | "ignore" | "equate") {
            continue;
        }
        let Some(close) = line[open + 1..].find('"').map(|i| open + 1 + i) else {
            continue;
        };
        let target = &line[open + 1..close];
        // Both circuits get the same rules; one is enough.
        if !target.starts_with("-circuit1") {
            continue;
        }
        let between = line[command.len()..open].trim();
        let after = line[close + 1..].trim();
        if target.ends_with("$dev") && in_group {
            let Some(group) = setup.catalog.groups.last_mut() else {
                continue;
            };
            match (*command, between) {
                ("permute", _) => push_once(&mut group.permute, after.to_string()),
                ("ignore", "class") => group.ignore = true,
                ("property", _) => {
                    let mut parts = after.splitn(2, char::is_whitespace);
                    let kind = parts.next().unwrap_or_default();
                    let rest = parts.next().unwrap_or_default().trim();
                    match kind {
                        "series" | "parallel" if rest == "enable" => {
                            if kind == "series" {
                                group.series = true;
                            } else {
                                group.parallel = true;
                            }
                        }
                        "series" | "parallel" => {
                            for item in items(rest) {
                                push_once(&mut group.combine, format!("{kind} {{{item}}}"));
                            }
                        }
                        "tolerance" => {
                            for item in items(rest) {
                                push_once(&mut group.tolerance, item);
                            }
                        }
                        "delete" => {
                            for item in items(rest) {
                                push_once(&mut group.delete, item);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        } else if target.ends_with("$cell")
            && let Some(pattern) = &pattern
        {
            let rule = format!("{command} {between} {after}");
            let rule = rule.split_whitespace().collect::<Vec<_>>().join(" ");
            let found = PatternRule {
                pattern: pattern.clone(),
                rule,
            };
            if !setup.catalog.patterns.contains(&found) {
                setup.catalog.patterns.push(found);
            }
        }
    }
    setup.catalog.groups.retain(|g| !g.devices.is_empty());
    setup
}

fn setup_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "tcl"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

pub fn scan(ctx: &ScanContext) -> Scanned<NetgenTech, NetgenCatalog> {
    let pdk = &ctx.pdk;
    let mut scanned: Scanned<NetgenTech, NetgenCatalog> = Scanned::default();
    let dir = pdk.tech("netgen");
    if !dir.is_dir() {
        scanned.notes.push(format!(
            "netgen: the PDK has no {} folder",
            pdk.portable(&dir)
        ));
        return scanned;
    }
    let files = setup_files(&dir);
    let name_of = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    // `<pdk>_setup.tcl` is the one open_pdks names; `setup.tcl` is its copy.
    let default = files
        .iter()
        .find(|f| name_of(f) == format!("{}_setup.tcl", pdk.name))
        .or_else(|| files.iter().find(|f| name_of(f) == "setup.tcl"))
        .or_else(|| files.iter().find(|f| name_of(f).contains("setup")))
        .cloned();
    let Some(default) = default else {
        scanned
            .notes
            .push(format!("netgen: no set-up file in {}", pdk.portable(&dir)));
        return scanned;
    };
    scanned.section.setup = Some(pdk.portable(&default));
    let text = std::fs::read_to_string(&default).unwrap_or_default();
    for other in files.iter().filter(|f| **f != default) {
        // A copy under another name says nothing new.
        if std::fs::read_to_string(other).is_ok_and(|t| t != text) {
            scanned.section.other_setups.push(pdk.portable(other));
        }
    }
    let setup = parse(&text);
    scanned.section.environment = setup.environment;
    scanned.catalog = setup.catalog;
    if scanned.catalog.groups.is_empty() {
        scanned.notes.push(format!(
            "netgen: no device rules could be read out of {}",
            pdk.portable(&default)
        ));
    }
    scanned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tech::pdk::PdkDir;

    const SETUP: &str = r#"
#---------------------------------------------------------------
# Setup file for netgen LVS
#---------------------------------------------------------------
permute default
property default
property parallel none

catch {format $env(NETGEN_COLUMNS)}

set cells1 [cells list -all -circuit1]

#-------------------------------------------
# Resistors (except metal)
#-------------------------------------------

set devices {}
lappend devices rsil
lappend devices rppd rhigh

foreach dev $devices {
    if {[lsearch $cells1 $dev] >= 0} {
	permute "-circuit1 $dev" 1 2
	property "-circuit1 $dev" series enable
	property "-circuit1 $dev" series {w critical}
	property "-circuit1 $dev" series {l add}
	property "-circuit1 $dev" parallel enable
	property "-circuit1 $dev" tolerance {l 0.01} {w 0.01}
	# Ignore these properties
	property "-circuit1 $dev" delete b par1
    }
    if {[lsearch $cells2 $dev] >= 0} {
	permute "-circuit2 $dev" 1 2
	property "-circuit2 $dev" delete other
    }
}

#-------------------------------------------
# (MOS) transistors
#-------------------------------------------

set devices {sg13_lv_nmos sg13_lv_pmos}

foreach dev $devices {
    if {[lsearch $cells1 $dev] >= 0} {
	permute "-circuit1 $dev" 1 3
	property "-circuit1 $dev" parallel enable
	property "-circuit1 $dev" parallel {l critical}
    }
}

# Empty group
set devices {}

#---------------------------------------------------------------
# Schematic cells which are not extractable
#---------------------------------------------------------------
set devices {sealring}
foreach dev $devices {
    if {[lsearch $cells1 $dev] >= 0} {
	ignore class "-circuit1 $dev"
    }
}

# Digital cells (ignore decap, fill, and tap cells)
if { [info exist ::env(MAGIC_EXT_USE_GDS)] && $::env(MAGIC_EXT_USE_GDS) } {
    foreach cell $cells1 {
        if {[regexp {sky130_fd_sc_[^_]+__decap_[[:digit:]]+} $cell match]} {
            ignore class "-circuit1 $cell"
        }
    }
}
foreach cell $cells1 {
    if {[regexp {sky130_fd_sc_[^_]+__fill_[[:digit:]]+} $cell match]} {
	property "-circuit1 $cell" parallel enable
    }
    if {[regexp "(.+)__(.+)" $cell match library cellname]} {
        equate classes "-circuit1 $cell" "-circuit2 $cellname"
    }
}
"#;

    #[test]
    fn groups_patterns_and_environment() {
        let setup = parse(SETUP);
        assert_eq!(setup.environment, ["NETGEN_COLUMNS", "MAGIC_EXT_USE_GDS"]);
        let c = &setup.catalog;
        assert_eq!(
            c.defaults,
            [
                "permute default",
                "property default",
                "property parallel none"
            ]
        );
        assert_eq!(
            c.groups.len(),
            3,
            "the empty group is dropped: {:#?}",
            c.groups
        );
        let res = &c.groups[0];
        assert_eq!(res.title, "Resistors (except metal)");
        assert_eq!(res.devices, ["rsil", "rppd", "rhigh"]);
        assert_eq!(res.permute, ["1 2"]);
        assert!(res.series && res.parallel && !res.ignore);
        assert_eq!(res.combine, ["series {w critical}", "series {l add}"]);
        assert_eq!(res.tolerance, ["l 0.01", "w 0.01"]);
        assert_eq!(res.delete, ["b", "par1"], "circuit 2 is not read again");
        let mos = &c.groups[1];
        assert_eq!(mos.title, "(MOS) transistors");
        assert_eq!(mos.devices, ["sg13_lv_nmos", "sg13_lv_pmos"]);
        assert_eq!(mos.permute, ["1 3"]);
        assert!(mos.parallel && !mos.series);
        assert_eq!(mos.combine, ["parallel {l critical}"]);
        let ignored = &c.groups[2];
        assert_eq!(ignored.title, "Schematic cells which are not extractable");
        assert!(ignored.ignore);
        assert_eq!(
            c.patterns,
            [
                PatternRule {
                    pattern: "sky130_fd_sc_[^_]+__decap_[[:digit:]]+".into(),
                    rule: "ignore class".into()
                },
                PatternRule {
                    pattern: "sky130_fd_sc_[^_]+__fill_[[:digit:]]+".into(),
                    rule: "property parallel enable".into()
                },
                PatternRule {
                    pattern: "(.+)__(.+)".into(),
                    rule: "equate classes \"-circuit2 $cellname\"".into()
                },
            ]
        );
    }

    #[test]
    fn the_setup_named_after_the_pdk_is_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = PdkDir::new(dir.path(), "sky130A");
        let netgen = pdk.tech("netgen");
        std::fs::create_dir_all(&netgen).unwrap();
        std::fs::write(netgen.join("setup.tcl"), SETUP).unwrap();
        std::fs::write(netgen.join("sky130A_setup.tcl"), SETUP).unwrap();
        std::fs::write(netgen.join("other_setup.tcl"), "permute default\n").unwrap();
        let scanned = scan(&ScanContext::new(pdk));
        assert_eq!(
            scanned.section.setup.as_ref().unwrap().as_str(),
            "$PDK_ROOT/sky130A/libs.tech/netgen/sky130A_setup.tcl"
        );
        assert_eq!(
            scanned.section.other_setups.len(),
            1,
            "the copy is not listed"
        );
        assert_eq!(scanned.section.environment.len(), 2);
        assert_eq!(scanned.catalog.groups.len(), 3);
        assert!(scanned.notes.is_empty());
        let text = toml::to_string(&scanned.section).unwrap();
        assert_eq!(
            toml::from_str::<NetgenTech>(&text).unwrap(),
            scanned.section
        );

        let none = scan(&ScanContext::new(PdkDir::new(dir.path(), "none")));
        assert_eq!(none.section, NetgenTech::default());
        assert_eq!(none.notes.len(), 1);
    }
}
