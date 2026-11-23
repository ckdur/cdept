//! netgen: `netgen -batch lvs "<netlist1> <cell1>" "<netlist2> <cell2>" setup.tcl lvs.log`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use super::{Adapter, Job};

/// Environment variable that names the netgen program.
pub const NETGEN_ENV: &str = "CDEPT_NETGEN";

pub const ADAPTER: Adapter = Adapter {
    name: "netgen",
    env_var: NETGEN_ENV,
    tool: "netgen",
};

/// The result netgen states at the end of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `Circuits match uniquely.` (or correctly: with symmetries).
    Match,
    /// The circuits match but a property (a width, a value) differs beyond its tolerance.
    PropertyErrors,
    /// The top-level pins do not match.
    PinMismatch,
    /// `Netlists do not match.`
    Mismatch,
    /// No `Final result` in the output.
    Unknown,
}

impl Verdict {
    /// LVS is clean.
    pub fn clean(self) -> bool {
        self == Verdict::Match
    }
}

/// What a comparison found.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub verdict: Verdict,
    /// The text after `Final result:`.
    pub result: String,
    /// `Circuit 1 contains N devices, Circuit 2 contains M devices.`
    pub devices: Option<(u64, u64)>,
    /// `Circuit 1 contains N nets, Circuit 2 contains M nets.`
    pub nets: Option<(u64, u64)>,
    /// The cells listed after `The following cells had property errors:`.
    pub property_errors: Vec<String>,
    /// The log netgen wrote (`lvs.log`), when it exists.
    pub log: Option<PathBuf>,
}

/// Read what netgen printed. `text` is the log it wrote and/or what it printed; the
/// last `Final result:` counts.
pub fn parse(text: &str) -> Report {
    let lines: Vec<&str> = text.lines().collect();
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = line.split_once("Final result:").map(|(_, r)| r.trim()) {
            // The text may be on the next line.
            result = if rest.is_empty() {
                lines[i + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .to_string()
            } else {
                rest.to_string()
            };
        }
    }
    let low = result.to_lowercase();
    let property_cells = !property_errors_of(&lines).is_empty();
    let verdict = if result.is_empty() {
        Verdict::Unknown
    } else if low.contains("pin matching") {
        Verdict::PinMismatch
    } else if low.contains("property error") {
        Verdict::PropertyErrors
    } else if low.contains("not match") || low.contains("fail") {
        Verdict::Mismatch
    } else if low.contains("match") {
        // netgen says `match uniquely` of circuits whose properties differ, and lists the cells.
        if property_cells {
            Verdict::PropertyErrors
        } else {
            Verdict::Match
        }
    } else {
        Verdict::Unknown
    };
    let pair = |word: &str| {
        lines.iter().rev().find_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix("Circuit 1 contains ")?;
            let (a, rest) = l.split_once(' ')?;
            let (_, rest) = rest.split_once("Circuit 2 contains ")?;
            let (b, rest) = rest.split_once(' ')?;
            rest.starts_with(word)
                .then(|| Some((a.parse().ok()?, b.parse().ok()?)))?
        })
    };
    let property_errors = property_errors_of(&lines);
    Report {
        verdict,
        result,
        devices: pair("devices"),
        nets: pair("nets"),
        property_errors,
        log: None,
    }
}

/// The cells listed after `The following cells had property errors:`.
fn property_errors_of(lines: &[&str]) -> Vec<String> {
    let Some(at) = lines
        .iter()
        .position(|l| l.contains("cells had property errors"))
    else {
        return vec![];
    };
    lines[at + 1..]
        .iter()
        .map(|l| l.trim())
        .take_while(|l| !l.is_empty() && !l.starts_with("Logging") && !l.starts_with("LVS"))
        .map(str::to_string)
        .collect()
}

/// One comparison: two netlists with the cell to compare in each, and the set-up file.
#[derive(Debug, Clone)]
pub struct Lvs {
    /// Netlist of the layout (extracted) and its cell.
    pub layout: PathBuf,
    pub layout_cell: String,
    /// Netlist of the schematic and its cell.
    pub schematic: PathBuf,
    pub schematic_cell: String,
    /// The PDK's set-up file. `None`: an empty one is written (every device on its own
    /// rules, no property compared).
    pub setup: Option<PathBuf>,
}

/// The job. Files go in `dir` (`lvs.log` for netgen's comparison, `netgen.stdout` for
/// what it printed). Paths are given absolute, because netgen reads them from `dir`.
/// netgen splits an argument at spaces, so a netlist path with one is refused.
pub fn lvs_job(program: &Path, lvs: &Lvs, dir: &Path, env: &[(OsString, OsString)]) -> Result<Job> {
    let abs = |p: &Path| std::path::absolute(p).wrap_err_with(|| p.display().to_string());
    let layout = abs(&lvs.layout)?;
    let schematic = abs(&lvs.schematic)?;
    for p in [&layout, &schematic] {
        if p.to_string_lossy().contains(char::is_whitespace) {
            bail!("netgen cannot read {}: spaces in the path", p.display());
        }
    }
    let setup = match &lvs.setup {
        Some(s) => abs(s)?,
        None => {
            std::fs::create_dir_all(dir)?;
            let empty = dir.join("empty_setup.tcl");
            std::fs::write(
                &empty,
                "# no PDK set-up file: devices are compared as they are\n",
            )?;
            abs(&empty)?
        }
    };
    Ok(Job::new("netgen", program, dir, "netgen.stdout")
        .args([
            OsString::from("-batch"),
            "lvs".into(),
            format!("{} {}", layout.display(), lvs.layout_cell).into(),
            format!("{} {}", schematic.display(), lvs.schematic_cell).into(),
            setup.into_os_string(),
            "lvs.log".into(),
        ])
        .env(env))
}

/// Run the comparison. An error only when netgen cannot run or fails; a mismatch is a
/// [`Report`] with a verdict. Netgen exits 0 on a mismatch, and prints `Error:` and
/// stops (also with 0) on a file it cannot read, which is an error here too.
pub fn run(program: &Path, lvs: &Lvs, dir: &Path, env: &[(OsString, OsString)]) -> Result<Report> {
    let job = lvs_job(program, lvs, dir, env)?;
    let _ = std::fs::remove_file(dir.join("lvs.log"));
    let out = job.run()?;
    let lvs_log = dir.join("lvs.log");
    let from_log = std::fs::read_to_string(&lvs_log).ok();
    let mut report = parse(from_log.as_deref().unwrap_or(&out.text));
    if report.verdict == Verdict::Unknown
        && let Some(e) = out
            .text
            .lines()
            .find(|l| l.trim_start().starts_with("Error:"))
    {
        return Err(eyre!("netgen: {}\nlog: {}", e.trim(), out.log.display()));
    }
    if report.devices.is_none() && report.nets.is_none() {
        let again = parse(&out.text);
        report.devices = again.devices;
        report.nets = again.nets;
        if report.property_errors.is_empty() {
            report.property_errors = again.property_errors;
        }
    }
    report.log = lvs_log.is_file().then_some(lvs_log);
    Ok(report)
}

/// The set-up file of the project's technology snapshot (`[netgen] setup`), when it
/// has one on this machine.
pub fn project_setup(project: &crate::Project) -> Option<PathBuf> {
    let path = project
        .snapshot()?
        .netgen
        .setup
        .as_ref()?
        .resolve(|n| project.var(n))
        .ok()?;
    path.is_file().then_some(path)
}

/// The netlist `file` for netgen: when it has devices written as `M`, `D` or `Q` lines
/// of the technology's models (the netlist KLayout extracts), a copy in `dir` with them
/// written as `X` calls, as the PDK's own netlists have them (netgen does not compare a
/// MOSFET with a call of the same model). Else `file` itself. The note says what was
/// done.
pub fn prepare_netlist(
    project: &crate::Project,
    file: &Path,
    dir: &Path,
) -> Result<(PathBuf, Option<String>)> {
    let text = std::fs::read_to_string(file)
        .wrap_err_with(|| format!("cannot read {}", file.display()))?;
    let catalog = project.catalog().ok().flatten();
    let models = crate::klayout::reference::DeviceModels::from_catalog(catalog.as_ref());
    let rewrite = crate::klayout::reference::calls_of_devices(&text, &models);
    if rewrite.converted == 0 {
        return Ok((file.to_path_buf(), None));
    }
    std::fs::create_dir_all(dir)?;
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = dir.join(format!("{stem}_calls.spice"));
    std::fs::write(&out, &rewrite.text)
        .wrap_err_with(|| format!("cannot write {}", out.display()))?;
    let note = format!(
        "{}: {} device line(s) written as calls of {} (netgen does not compare a MOSFET with a \
         call of its model, which is how the PDK's netlists have them); copy {}",
        file.display(),
        rewrite.converted,
        rewrite
            .models
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        out.display()
    );
    Ok((out, Some(note)))
}

/// What to say when netgen does not accept a netlist that KLayout extracted: the
/// differences left are KLayout's, not the layout's. `None` for any other netlist.
pub fn klayout_hint(layout: &Path) -> Option<String> {
    let text = std::fs::read_to_string(layout).ok()?;
    let from_klayout = text
        .lines()
        .take(3)
        .any(|l| l.starts_with('*') && l.contains("Extracted by KLayout"));
    let from_copy = layout
        .file_stem()
        .is_some_and(|s| s.to_string_lossy().ends_with("_calls"));
    if !from_klayout && !from_copy {
        return None;
    }
    Some(
        "the layout netlist was extracted by KLayout. Its decks add what a netlist of the PDK \
         does not have: a substrate pin of their own for a cell without a tap (sky130: \
         `sky130_gnd`, so the pin lists differ) and sizes in other units (sky130: scaled by \
         1e6, so the properties differ). Netgen cannot be told to ignore either: compare a \
         netlist extracted with Magic (`cdept magic extract`), or use `cdept layout lvs` for \
         KLayout's own comparison"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = "Circuit 1 contains 1 devices, Circuit 2 contains 1 devices.\n\
        Circuit 1 contains 2 nets,    Circuit 2 contains 2 nets.\n\n\nFinal result: \nCircuits match uniquely.\n\nLogging disabled\n";

    #[test]
    fn reads_a_match() {
        let r = parse(CLEAN);
        assert_eq!(r.verdict, Verdict::Match);
        assert!(r.verdict.clean());
        assert_eq!(r.devices, Some((1, 1)));
        assert_eq!(r.nets, Some((2, 2)));
    }

    #[test]
    fn reads_the_other_results() {
        let v = |t: &str| parse(t).verdict;
        assert_eq!(
            v("Final result: Netlists do not match.\n"),
            Verdict::Mismatch
        );
        assert_eq!(
            v("Final result: Top level cell failed pin matching.\n"),
            Verdict::PinMismatch
        );
        assert_eq!(
            v("Final result: Circuits match with property errors.\n"),
            Verdict::PropertyErrors
        );
        assert_eq!(
            v("Final result: Circuits match correctly.\n"),
            Verdict::Match
        );
        assert_eq!(v("nothing here"), Verdict::Unknown);
        let r = parse(
            "Final result: Circuits match with property errors.\n\nThe following cells had property errors:\n a\n b\n\nLVS Done.\n",
        );
        assert_eq!(r.property_errors, ["a", "b"]);
        // `match uniquely`, but with cells listed for property errors: not clean.
        assert_eq!(
            v(
                "Final result: Circuits match uniquely.\n\nThe following cells had property errors:\n top\n"
            ),
            Verdict::PropertyErrors
        );
        // The last Final result counts (a log with several comparisons).
        assert_eq!(
            v("Final result: Netlists do not match.\nFinal result: Circuits match uniquely.\n"),
            Verdict::Match
        );
    }

    #[test]
    fn the_command_line_is_netgens() {
        let dir = tempfile::tempdir().unwrap();
        let lvs = Lvs {
            layout: "/l/top.spice".into(),
            layout_cell: "top".into(),
            schematic: "/s/top.spice".into(),
            schematic_cell: "top".into(),
            setup: Some("/pdk/setup.tcl".into()),
        };
        let j = lvs_job(Path::new("netgen"), &lvs, dir.path(), &[]).unwrap();
        let args: Vec<_> = j
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-batch",
                "lvs",
                "/l/top.spice top",
                "/s/top.spice top",
                "/pdk/setup.tcl",
                "lvs.log"
            ]
        );
        let spaced = Lvs {
            layout: "/a b/x.spice".into(),
            ..lvs.clone()
        };
        assert!(lvs_job(Path::new("netgen"), &spaced, dir.path(), &[]).is_err());
        let none = Lvs { setup: None, ..lvs };
        assert!(lvs_job(Path::new("netgen"), &none, dir.path(), &[]).is_ok());
        assert!(dir.path().join("empty_setup.tcl").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_fake_netgen() {
        let dir = tempfile::tempdir().unwrap();
        let fake = super::super::testutil::fake(
            dir.path(),
            "netgen",
            "echo 'Circuit 1 contains 4 devices, Circuit 2 contains 5 devices.' > lvs.log\n\
             echo 'Final result: Netlists do not match.' >> lvs.log\necho done",
        );
        let lvs = Lvs {
            layout: "a.spice".into(),
            layout_cell: "t".into(),
            schematic: "b.spice".into(),
            schematic_cell: "t".into(),
            setup: None,
        };
        let r = run(&fake, &lvs, &dir.path().join("w"), &[]).unwrap();
        assert_eq!(r.verdict, Verdict::Mismatch);
        assert_eq!(r.devices, Some((4, 5)));
        assert!(r.log.is_some());
        // An unreadable file: netgen prints Error: and exits 0.
        let broken = super::super::testutil::fake(
            dir.path(),
            "netgen2",
            "echo 'Error:  Setup file x does not exist.'",
        );
        let err = run(&broken, &lvs, &dir.path().join("w2"), &[]).unwrap_err();
        assert!(format!("{err}").contains("Setup file"), "{err}");
    }

    /// Against the real program, when there is one.
    #[test]
    fn real_netgen_compares_two_netlists() {
        let Some(program) = ADAPTER.locate() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let w = |name: &str, text: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, text).unwrap();
            p
        };
        let a = w("a.spice", ".subckt top x y\nR1 x y 1k\n.ends\n");
        let b = w("b.spice", ".subckt top x y\nR9 x y 1k\n.ends\n");
        let c = w("c.spice", ".subckt top x y\nR9 x y 2k\n.ends\n");
        let lvs = |l: &Path, s: &Path| Lvs {
            layout: l.into(),
            layout_cell: "top".into(),
            schematic: s.into(),
            schematic_cell: "top".into(),
            setup: None,
        };
        let same = run(&program, &lvs(&a, &b), &dir.path().join("r1"), &[]).unwrap();
        assert_eq!(same.verdict, Verdict::Match, "{same:?}");
        assert_eq!(same.devices, Some((1, 1)));
        let off = run(&program, &lvs(&a, &c), &dir.path().join("r2"), &[]).unwrap();
        assert_eq!(off.verdict, Verdict::PropertyErrors, "{off:?}");
    }
}
