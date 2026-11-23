//! Qucs-S: schematic to SPICE netlist without a window, and launching the GUI tools.
//!
//! `qucs-s -n -i in.sch -o out.cir --ngspice|--xyce` converts a Qucs-S schematic to a
//! netlist and exits. It needs no display (checked without `DISPLAY`; the job sets
//! `QT_QPA_PLATFORM=offscreen` as a default anyway) and its exit status is 0 even
//! when it fails, so success is judged from its messages and the netlist. The
//! netlist it writes holds the analysis of the schematic's simulation component
//! (in a `.control` block for ngspice, as `.ac`/`.tran`/… cards for Xyce);
//! [`sim_netlist`] removes those so that a testbench's `setup.toml` supplies the
//! analyses. Design: docs/QUCS.md.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use super::{Adapter, Job};
use crate::Project;
use crate::entry::Format;

/// Environment variable that names the Qucs-S program.
pub const QUCS_ENV: &str = "CDEPT_QUCS";
/// Environment variable that names the S-parameter viewer.
pub const SPAR_ENV: &str = "CDEPT_QUCS_SPAR";

pub const ADAPTER: Adapter = Adapter { name: "Qucs-S", env_var: QUCS_ENV, tool: "Qucs-S" };
pub const SPAR_ADAPTER: Adapter =
    Adapter { name: "Qucs-S S-parameter viewer", env_var: SPAR_ENV, tool: "Qucs-S S-param viewer" };

/// The simulator flavour of the netlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    Ngspice,
    Xyce,
    /// `--cdl`
    Cdl,
}

impl Flavour {
    pub fn flag(self) -> &'static str {
        match self {
            Flavour::Ngspice => "--ngspice",
            Flavour::Xyce => "--xyce",
            Flavour::Cdl => "--cdl",
        }
    }

    /// For a `setup.toml` `simulator` value (`xyce`, else ngspice).
    pub fn for_simulator(simulator: &str) -> Flavour {
        if simulator.eq_ignore_ascii_case("xyce") { Flavour::Xyce } else { Flavour::Ngspice }
    }
}

/// The job that converts `schematic` to `out` (both made absolute: the job runs in
/// `dir`, a folder of its own, because Qucs-S drops files next to where it runs).
pub fn export_job(
    program: &Path,
    schematic: &Path,
    out: &Path,
    flavour: Flavour,
    dir: &Path,
    env: &[(OsString, OsString)],
) -> Job {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut job = Job::new("Qucs-S", program, dir, "qucs.log")
        .args(["-n", "-i"])
        .args([abs(schematic).into_os_string(), "-o".into(), abs(out).into_os_string(), flavour.flag().into()])
        .env(env);
    job.env_defaults = vec![("QT_QPA_PLATFORM".into(), "offscreen".into())];
    job
}

/// The lines of Qucs-S's output that are errors (it prints `Error: …`), without the
/// Qt noise it also prints.
pub fn errors(log: &str) -> Vec<String> {
    log.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Error") || l.starts_with("ERROR"))
        .map(String::from)
        .collect()
}

/// Whether `netlist` has a device or subcircuit card, i.e. is more than the header
/// Qucs-S writes for a schematic it could not convert.
pub fn has_devices(netlist: &str) -> bool {
    // The control block (commands such as `exit`) does not count.
    sim_netlist(netlist, Flavour::Ngspice)
        .lines()
        .map(str::trim)
        .any(|l| l.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
}

/// Convert `schematic` to the netlist `out`, as Qucs-S writes it (analysis included).
/// Errors with Qucs-S's messages when it reports any or writes no devices.
pub fn export(project: Option<&Project>, schematic: &Path, out: &Path, flavour: Flavour) -> Result<()> {
    let program = ADAPTER.require()?;
    if !schematic.is_file() {
        bail!("no schematic {}", schematic.display());
    }
    // A folder of its own so that nothing Qucs-S leaves behind lands in the library.
    let dir = tempdir("export")?;
    let env = project.map(Project::tool_env).unwrap_or_default();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(out);
    let job = export_job(&program, schematic, out, flavour, &dir, &env);
    let run = job.run_unchecked();
    let _ = std::fs::remove_dir_all(&dir);
    let run = run?;
    let errs = errors(&run.text);
    let text = std::fs::read_to_string(out).unwrap_or_default();
    if !run.status.success() || !errs.is_empty() || !has_devices(&text) {
        let why = if errs.is_empty() { "no devices in its netlist".to_string() } else { errs.join("; ") };
        bail!(
            "Qucs-S could not convert {}: {why}. (Parts from a library need it installed in \
             the Qucs home, see docs/QUCS.md.)",
            schematic.display()
        );
    }
    Ok(())
}

fn tempdir(tag: &str) -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cdept-qucs-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).wrap_err("cannot make a temporary folder")?;
    Ok(dir)
}

/// What a testbench needs of a Qucs-S netlist: devices and models, without the
/// analysis cards or control block of the schematic's own simulation component and
/// without `.end`; the deck adds the analyses of `setup.toml`.
pub fn sim_netlist(netlist: &str, flavour: Flavour) -> String {
    const ANALYSIS: &[&str] = &[
        ".ac", ".tran", ".dc", ".op", ".noise", ".tf", ".sens", ".four", ".step", ".print", ".probe", ".plot",
    ];
    let mut out = String::new();
    let mut control = false;
    for line in netlist.lines() {
        let word = line.trim().to_ascii_lowercase();
        if word.starts_with(".control") {
            control = true;
        }
        let drop = control
            || word == ".end"
            || (flavour == Flavour::Xyce
                && ANALYSIS.iter().any(|a| word == *a || word.starts_with(&format!("{a} "))));
        if !drop {
            out.push_str(line);
            out.push('\n');
        }
        if word.starts_with(".endc") {
            control = false;
        }
    }
    out
}

/// Whether `file` is a Qucs-S schematic (a `.sch` that starts with `<Qucs`).
pub fn is_qucs_schematic(file: &Path) -> bool {
    Format::of_file(file) == Some(Format::QucsSchematic)
}

/// Netlist `schematic` for a testbench into `out`: [`export`] then [`sim_netlist`].
pub fn netlist_for_sim(project: &Project, schematic: &Path, out: &Path, simulator: &str) -> Result<()> {
    let flavour = Flavour::for_simulator(simulator);
    export(Some(project), schematic, out, flavour)?;
    let text = std::fs::read_to_string(out)?;
    std::fs::write(out, sim_netlist(&text, flavour))?;
    Ok(())
}

/// The schematic file of a testbench. `kind` is `schematic_kind` of the setup: `qucs`,
/// `xschem`, or empty (the named view whatever it is, else the cell's first xschem
/// schematic, else its first Qucs-S one).
pub fn schematic_for(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
    kind: &str,
) -> Result<PathBuf> {
    let found = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?
        .cell(cell)
        .ok_or_else(|| eyre!("no cell '{cell}' in library '{library}'"))?;
    let want_qucs = match kind {
        "qucs" | "qucs-s" => Some(true),
        "" => None,
        "xschem" => Some(false),
        other => return Err(eyre!("schematic_kind '{other}' is not xschem or qucs")),
    };
    let entry = match view {
        Some(name) => found.entry(name).ok_or_else(|| eyre!("cell '{cell}' has no view '{name}'"))?,
        None => {
            let first = |f: Format| found.entries.iter().find(|e| e.format == Some(f));
            match want_qucs {
                Some(true) => first(Format::QucsSchematic),
                Some(false) => first(Format::XschemSchematic),
                None => first(Format::XschemSchematic).or_else(|| first(Format::QucsSchematic)),
            }
            .ok_or_else(|| eyre!("cell '{cell}' has no {} schematic", kind_name(want_qucs)))?
        }
    };
    if let Some(q) = want_qucs
        && (entry.format == Some(Format::QucsSchematic)) != q
    {
        bail!("view '{}' of '{cell}' is not a {} schematic", entry.name, kind_name(want_qucs));
    }
    entry.main_file.clone().ok_or_else(|| eyre!("view '{}' of '{cell}' has no file", entry.name))
}

fn kind_name(q: Option<bool>) -> &'static str {
    match q {
        Some(true) => "Qucs-S",
        Some(false) => "xschem",
        None => "xschem or Qucs-S",
    }
}

/// Start the Qucs-S window on `file` (a schematic or nothing) and return at once.
/// The project's tool environment is given (`PDK_ROOT`, …).
pub fn launch(project: Option<&Project>, file: Option<&Path>) -> Result<std::process::Child> {
    spawn(&ADAPTER, project, file)
}

/// Start the S-parameter viewer on a Touchstone file (`.s2p`, `.snp`).
pub fn launch_spar_viewer(project: Option<&Project>, file: Option<&Path>) -> Result<std::process::Child> {
    spawn(&SPAR_ADAPTER, project, file)
}

fn spawn(adapter: &Adapter, project: Option<&Project>, file: Option<&Path>) -> Result<std::process::Child> {
    use std::process::{Command, Stdio};
    let program = adapter.require()?;
    let mut cmd = Command::new(&program);
    if let Some(f) = file {
        cmd.arg(std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf()));
        if let Some(dir) = f.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
    }
    if let Some(p) = project {
        cmd.envs(p.tool_env().iter().map(|(k, v)| (k, v)));
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().wrap_err_with(|| format!("cannot start {} ({})", adapter.name, program.display()))
}

/// Touchstone file (`.s1p` … `.s9p`, `.snp`): the S-parameter viewer opens these.
pub fn is_touchstone(file: &Path) -> bool {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == "snp"
        || (ext.len() == 3
            && ext.starts_with('s')
            && ext.ends_with('p')
            && ext[1..2].chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NGSPICE: &str = "* Qucs 25.2.0  /x/rc.sch\n.INCLUDE \"/m/ngspice_mathfunc.inc\"\n\
        V1 in 0 DC 0 SIN(0 1 1K 0 0 0) AC 1 ACPHASE 0\nR1 in out  1K tc1=0.0 tc2=0.0 \nC1 0 out  159N \n\n\
        .control\n\nac lin 50 10 100k \nwrite spice4qucs.ac1.plot v(in) v(out)\ndestroy all\nreset\n\nexit\n.endc\n.END\n";
    const XYCE: &str = "* Qucs 25.2.0  /x/rc.sch\nV1 in 0 DC 0\nR1 in out  1K\nC1 0 out  159N \n\
        .ac lin 50 10 100k\n.PRINT  ac format=raw file=spice4qucs.ac.plot v(in) v(out) \n.END\n";

    #[test]
    fn strips_the_schematics_own_analysis() {
        let n = sim_netlist(NGSPICE, Flavour::Ngspice);
        assert!(n.contains("R1 in out") && n.contains(".INCLUDE"), "{n}");
        assert!(!n.contains("control") && !n.contains("ac lin") && !n.to_lowercase().contains(".end"), "{n}");
        let x = sim_netlist(XYCE, Flavour::Xyce);
        assert!(x.contains("C1 0 out") && !x.contains(".ac") && !x.contains("PRINT") && !x.contains("END"), "{x}");
    }

    #[test]
    fn detects_a_failed_conversion() {
        let stub = "* Qucs 25.2.0  /x/a.sch\n.INCLUDE \"/m/ngspice_mathfunc.inc\"\n";
        assert!(!has_devices(stub));
        assert!(has_devices(NGSPICE));
        let log = "QGuiApplication::font(): no QGuiApplication\nError: Could not go throughAllComps\nError giving NodeNames\n";
        assert_eq!(errors(log).len(), 2);
        assert!(errors("Warning: QThreadStorage\n").is_empty());
    }

    #[test]
    fn the_command_line_is_qucs_s() {
        let job = export_job(
            Path::new("qucs-s"),
            Path::new("/a/rc.sch"),
            Path::new("/b/rc.cir"),
            Flavour::Xyce,
            Path::new("/t"),
            &[],
        );
        let args: Vec<_> = job.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-n", "-i", "/a/rc.sch", "-o", "/b/rc.cir", "--xyce"]);
        assert_eq!(job.env_defaults[0].0, "QT_QPA_PLATFORM");
    }

    #[test]
    fn the_setup_names_the_kind_and_the_example_is_a_qucs_testbench() {
        let s = crate::sim::Setup::parse("schematic_kind = \"qucs\"\nsimulator = \"xyce\"").unwrap();
        assert_eq!(s.schematic_kind, "qucs");
        assert_eq!(Flavour::for_simulator(&s.simulator), Flavour::Xyce);
        assert_eq!(Flavour::for_simulator(""), Flavour::Ngspice);
        let tb = include_str!("../../../examples/ihp-inv/libs/cells/rc_lowpass/testbench/setup.toml");
        assert_eq!(crate::sim::Setup::parse(tb).unwrap().schematic_kind, "qucs");
        let dir = tempdir("kind").unwrap();
        let sch = dir.join("rc.sch");
        std::fs::write(&sch, RC).unwrap();
        assert!(is_qucs_schematic(&sch));
        std::fs::write(&sch, "v {xschem version=3.4.5 file_version=1.2}\n").unwrap();
        assert!(!is_qucs_schematic(&sch));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn touchstone_names() {
        for yes in ["a.s2p", "a.S4P", "x.snp", "lna.s1p"] {
            assert!(is_touchstone(Path::new(yes)), "{yes}");
        }
        for no in ["a.sp", "a.spice", "a.s2", "a.sxp", "a"] {
            assert!(!is_touchstone(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn real_export_of_an_rc() {
        if ADAPTER.locate().is_none() {
            eprintln!("skipped: no Qucs-S");
            return;
        }
        let dir = tempdir("test").unwrap();
        let sch = dir.join("rc.sch");
        std::fs::write(&sch, RC).unwrap();
        for fl in [Flavour::Ngspice, Flavour::Xyce] {
            let out = dir.join(format!("rc{}.cir", fl.flag()));
            export(None, &sch, &out, fl).unwrap();
            let text = std::fs::read_to_string(&out).unwrap();
            assert!(text.contains("R1 in out") && text.contains("C1"), "{text}");
            let sim = sim_netlist(&text, fl);
            assert!(!sim.to_lowercase().contains(".ac") && !sim.contains("control"), "{sim}");
        }
        // a broken file is an error, not a stub netlist
        let bad = dir.join("bad.sch");
        std::fs::write(&bad, "<Qucs Schematic 25.2.0>\n").unwrap();
        assert!(export(None, &bad, &dir.join("bad.cir"), Flavour::Ngspice).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    const RC: &str = include_str!("../../../examples/ihp-inv/libs/cells/rc_lowpass/schematic/rc_lowpass.sch");
}
