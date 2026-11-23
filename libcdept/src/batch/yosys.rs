//! Yosys: run a script, or synthesise sources to a netlist, with a Liberty file from
//! the technology snapshot, and read the `stat` it prints.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, bail, eyre};

use super::{Adapter, Job, Output};

/// Environment variable that names the Yosys program.
pub const YOSYS_ENV: &str = "CDEPT_YOSYS";

pub const ADAPTER: Adapter = Adapter {
    name: "Yosys",
    env_var: YOSYS_ENV,
    tool: "Yosys",
};

/// A synthesis: Verilog sources to a gate-level netlist.
#[derive(Debug, Clone)]
pub struct Synth {
    pub sources: Vec<PathBuf>,
    pub top: String,
    /// The cell library. `None`: Yosys's own gates (`$_AND_`, …), nothing is mapped.
    pub liberty: Option<PathBuf>,
    /// The netlist to write, relative to the run folder or absolute.
    pub netlist: PathBuf,
}

/// The Yosys script of a synthesis (`synth.ys`).
pub fn synth_script(s: &Synth) -> String {
    let mut out = String::from("# written by CDePT\n");
    for src in &s.sources {
        out.push_str(&format!("read_verilog -sv \"{}\"\n", src.display()));
    }
    out.push_str(&format!(
        "hierarchy -check -top {}\nsynth -top {} -flatten\n",
        s.top, s.top
    ));
    if let Some(lib) = &s.liberty {
        let lib = lib.display();
        out.push_str(&format!(
            "dfflibmap -liberty \"{lib}\"\nabc -liberty \"{lib}\"\nopt_clean -purge\n"
        ));
        out.push_str(&format!("tee -o stat.txt stat -liberty \"{lib}\"\n"));
    } else {
        out.push_str("tee -o stat.txt stat\n");
    }
    out.push_str(&format!(
        "write_verilog -noattr \"{}\"\n",
        s.netlist.display()
    ));
    out
}

/// The job for a script in `dir`: `yosys -s <script>`, everything printed in
/// `yosys.log`.
pub fn script_job(program: &Path, script: &Path, dir: &Path, env: &[(OsString, OsString)]) -> Job {
    Job::new("Yosys", program, dir, "yosys.log")
        .args([OsString::from("-s"), script.as_os_str().to_owned()])
        .env(env)
}

/// Run a script. An error when Yosys fails (a script error ends it with a status).
pub fn run_script(
    program: &Path,
    script: &Path,
    dir: &Path,
    env: &[(OsString, OsString)],
) -> Result<(Output, Option<Stat>)> {
    let script = std::path::absolute(script)?;
    let out = script_job(program, &script, dir, env).run()?;
    let stat = parse_stat(&out.text);
    Ok((out, stat))
}

/// What a synthesis made.
#[derive(Debug, Clone)]
pub struct SynthReport {
    pub netlist: PathBuf,
    pub script: PathBuf,
    pub log: PathBuf,
    pub stat: Option<Stat>,
}

/// Write the script and run it in `dir`. Sources are made absolute (Yosys runs in
/// `dir`), the netlist goes to `dir/<netlist>` when relative.
pub fn synth(
    program: &Path,
    s: &Synth,
    dir: &Path,
    env: &[(OsString, OsString)],
) -> Result<SynthReport> {
    if s.sources.is_empty() {
        bail!("no Verilog sources to synthesise");
    }
    let abs = |p: &PathBuf| std::path::absolute(p);
    let s = Synth {
        sources: s.sources.iter().map(abs).collect::<std::io::Result<_>>()?,
        liberty: s.liberty.as_ref().map(abs).transpose()?,
        ..s.clone()
    };
    std::fs::create_dir_all(dir)?;
    let script = dir.join("synth.ys");
    std::fs::write(&script, synth_script(&s))?;
    let netlist = dir.join(&s.netlist);
    let _ = std::fs::remove_file(&netlist);
    let (out, stat) = run_script(program, &script, dir, env)?;
    if !netlist.is_file() {
        bail!(
            "Yosys wrote no netlist ({})\nlog: {}",
            netlist.display(),
            out.log.display()
        );
    }
    Ok(SynthReport {
        netlist,
        script,
        log: out.log,
        stat,
    })
}

/// The design size `stat` printed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stat {
    /// The module, or `design hierarchy` for a design with submodules (then with
    /// everything below).
    pub module: String,
    pub cells: u64,
    /// `Chip area for module`, with a Liberty file.
    pub area: Option<f64>,
    /// Cells by type.
    pub by_type: BTreeMap<String, u64>,
}

/// The last `stat` of a Yosys log: its `design hierarchy` block when there is one,
/// else the last module. Both layouts of the numbers are read (`3 cells` and
/// `1   $_AND_`, and the older `Number of cells: 3` and `$_AND_   1`).
pub fn parse_stat(log: &str) -> Option<Stat> {
    let at = log.rfind("Printing statistics")?;
    let mut blocks: Vec<Stat> = vec![];
    for line in log[at..].lines().skip(1) {
        let t = line.trim();
        // The next pass ends the section.
        if t.contains("Executing") {
            break;
        }
        if let Some(name) = t.strip_prefix("===").and_then(|r| r.strip_suffix("===")) {
            blocks.push(Stat {
                module: name.trim().to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(block) = blocks.last_mut() else {
            continue;
        };
        if let Some(n) = t.strip_prefix("Number of cells:") {
            block.cells = n.trim().parse().unwrap_or(0);
        } else if let Some((head, rest)) = t.split_once(" area for module") {
            if head.contains("Chip") {
                block.area = rest.rsplit(':').next().and_then(|v| v.trim().parse().ok());
            }
        } else {
            let words: Vec<&str> = t.split_whitespace().collect();
            match words.as_slice() {
                [n, "cells" | "cell"] => block.cells = n.parse().unwrap_or(block.cells),
                [n, name] if n.parse::<u64>().is_ok() && is_cell_name(name) => {
                    block
                        .by_type
                        .insert(name.to_string(), n.parse().unwrap_or(0));
                }
                [name, n] if n.parse::<u64>().is_ok() && is_cell_name(name) => {
                    block
                        .by_type
                        .insert(name.to_string(), n.parse().unwrap_or(0));
                }
                _ => {}
            }
        }
    }
    blocks
        .iter()
        .position(|b| b.module == "design hierarchy")
        .map(|i| blocks.swap_remove(i))
        .or_else(|| blocks.pop())
}

fn is_cell_name(word: &str) -> bool {
    !matches!(
        word,
        "wires"
            | "wire"
            | "cells"
            | "ports"
            | "memories"
            | "memory"
            | "processes"
            | "bits"
            | "submodules"
            | "modules"
            | "area"
            | "count"
    ) && word.chars().all(|c| !c.is_whitespace() && c != ':')
        && word
            .chars()
            .any(|c| c.is_alphabetic() || c == '$' || c == '\\')
}

/// The Liberty file of a reference library in the project's snapshot (`[refs]`): the
/// library (`refs.digital`'s default when `library` is `None`) at a corner (the
/// library's default, else the digital default, when `corner` is `None`).
pub fn liberty_of(
    project: &crate::Project,
    library: Option<&str>,
    corner: Option<&str>,
) -> Result<PathBuf> {
    let tech = project
        .snapshot()
        .ok_or_else(|| eyre!("the project has no technology snapshot (cdept tech scan)"))?;
    let digital = tech.refs.digital.as_ref();
    let name = library
        .map(str::to_string)
        .or_else(|| digital.and_then(|d| d.default_library.clone()))
        .ok_or_else(|| {
            eyre!("no library given and the technology names no default digital library")
        })?;
    let lib = tech
        .refs
        .libraries
        .iter()
        .find(|l| l.name == name)
        .ok_or_else(|| eyre!("the technology has no reference library '{name}'"))?;
    let corner = corner
        .map(str::to_string)
        .or_else(|| lib.default_corner.clone())
        .or_else(|| digital.and_then(|d| d.default_corner.clone()))
        .ok_or_else(|| eyre!("no corner given and '{name}' names no default"))?;
    let file = lib.liberty.get(&corner).ok_or_else(|| {
        let known: Vec<&str> = lib.liberty.keys().map(String::as_str).collect();
        eyre!(
            "'{name}' has no Liberty corner '{corner}' (there are: {})",
            known.join(", ")
        )
    })?;
    let path = file.resolve(|n| project.var(n)).map_err(|e| eyre!(e))?;
    if path.to_string_lossy().contains('*') {
        bail!(
            "'{name}' has a Liberty file per cell at '{corner}' ({}); name a file",
            path.display()
        );
    }
    if !path.is_file() {
        bail!("{} is not on this machine", path.display());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = "3.27. Printing statistics.\n\n=== top ===\n\n        +----------Local Count, excluding submodules.\n        | \n\
        6 wires\n        5 ports\n        3 cells\n        1   $_AND_\n        1   $_OR_\n        1   $_XOR_\n\n\
        3.28. Executing CHECK pass (checking for obvious problems).\n";
    const OLD: &str = "2.9. Printing statistics.\n\n=== top ===\n\n   Number of wires:                  6\n   Number of cells:                  3\n\
        \x20    sg13g2_and2_1                   2\n     sg13g2_inv_1                    1\n\n   Chip area for module '\\top': 41.472\n\n\
        2.10. Executing CHECK pass.\n";

    #[test]
    fn reads_the_new_stat() {
        let s = parse_stat(NEW).unwrap();
        assert_eq!(s.module, "top");
        assert_eq!(s.cells, 3);
        assert_eq!(s.by_type["$_XOR_"], 1);
        assert_eq!(s.by_type.len(), 3, "{s:?}");
        assert_eq!(s.area, None);
    }

    #[test]
    fn reads_the_old_stat_with_area() {
        let s = parse_stat(OLD).unwrap();
        assert_eq!(s.cells, 3);
        assert_eq!(s.by_type["sg13g2_and2_1"], 2);
        assert_eq!(s.area, Some(41.472));
    }

    #[test]
    fn prefers_the_design_hierarchy_and_the_last_stat() {
        let log = format!(
            "{OLD}\n5. Printing statistics.\n\n=== sub ===\n   2 cells\n\n=== top ===\n   3 cells\n\n\
             === design hierarchy ===\n\n   2 submodules\n   9 cells\n   1   $_AND_\n\n6. Executing X\n"
        );
        let s = parse_stat(&log).unwrap();
        assert_eq!(s.module, "design hierarchy");
        assert_eq!(s.cells, 9);
        assert!(parse_stat("no stat here").is_none());
    }

    #[test]
    fn the_script_maps_with_a_liberty_file() {
        let mut s = Synth {
            sources: vec!["/s/top.v".into()],
            top: "top".into(),
            liberty: Some("/lib/tt.lib".into()),
            netlist: "out.v".into(),
        };
        let text = synth_script(&s);
        let at = |n: &str| text.find(n).unwrap_or_else(|| panic!("no {n} in\n{text}"));
        assert!(at("read_verilog -sv \"/s/top.v\"") < at("synth -top top -flatten"));
        assert!(at("synth -top") < at("dfflibmap -liberty \"/lib/tt.lib\""));
        assert!(at("dfflibmap") < at("abc -liberty") && at("abc") < at("stat -liberty"));
        assert!(text.contains("write_verilog -noattr \"out.v\""));
        s.liberty = None;
        assert!(!synth_script(&s).contains("liberty"));
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_fake_yosys() {
        let dir = tempfile::tempdir().unwrap();
        let fake = super::super::testutil::fake(
            dir.path(),
            "yosys",
            "echo '3. Printing statistics.'; echo '=== top ==='; echo '   2 cells'; echo '   2   $_AND_'; echo 'x' > out.v",
        );
        let s = Synth {
            sources: vec!["a.v".into()],
            top: "top".into(),
            liberty: None,
            netlist: "out.v".into(),
        };
        let r = synth(&fake, &s, &dir.path().join("w"), &[]).unwrap();
        assert_eq!(r.stat.unwrap().cells, 2);
        assert!(r.netlist.is_file() && r.script.is_file());
        let bad =
            super::super::testutil::fake(dir.path(), "badyosys", "echo 'ERROR: no top'; exit 1");
        assert!(
            format!(
                "{}",
                synth(&bad, &s, &dir.path().join("w2"), &[]).unwrap_err()
            )
            .contains("no top")
        );
        assert!(
            synth(
                &fake,
                &Synth {
                    sources: vec![],
                    ..s
                },
                dir.path(),
                &[]
            )
            .is_err()
        );
    }

    /// Against the real program, when there is one.
    #[test]
    fn real_yosys_synthesises() {
        let Some(program) = ADAPTER.locate() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("top.v");
        std::fs::write(&v, "module top(input a,b,c, output y, output z);\nassign y = (a&b)|c;\nassign z = a^b;\nendmodule\n").unwrap();
        let s = Synth {
            sources: vec![v],
            top: "top".into(),
            liberty: None,
            netlist: "out.v".into(),
        };
        let r = synth(&program, &s, &dir.path().join("run"), &[]).unwrap();
        let stat = r.stat.unwrap();
        assert_eq!(stat.cells, 3, "{stat:?}");
        assert_eq!(stat.by_type["$_XOR_"], 1);
        assert!(
            std::fs::read_to_string(r.netlist)
                .unwrap()
                .contains("module top")
        );
    }
}
