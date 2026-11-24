//! VACASK as a testbench simulator (`simulator = "vacask"`). Design and status of
//! each PDK: docs/VACASK.md.
//!
//! VACASK reads its own netlist format (Spectre-like) and loads compact models as
//! OSDI libraries. A CDePT testbench stays an ordinary xschem schematic netlisted
//! as SPICE, so three pieces bridge the two:
//!
//! - [`translate`]: the xschem SPICE netlist into VACASK's format;
//! - [`models`]: the technology's models for VACASK, converted from the ngspice
//!   ones with VACASK's own converter and the Verilog-A sources compiled with
//!   OpenVAF, recorded as `[models.vacask]`;
//! - [`deck`]: the whole netlist of a run point, with its `control` block.
//!
//! VACASK runs as a separate process ([`crate::batch::vacask`]) and writes one raw
//! file per analysis, read by [`raw`]; `.meas`-style measurements of the setup are
//! evaluated by [`meas`].

pub mod deck;
pub mod meas;
pub mod models;
pub mod raw;
pub mod stat;
pub mod translate;

use std::ffi::OsString;
use std::path::Path;

use eyre::{Result, WrapErr, bail};

use crate::sim::run::Outcome;

/// Environment for a program of the VACASK install (`vacask`, `openvaf-r`): its
/// shared libraries are in `<prefix>/lib`, which the loader does not know, so that
/// folder is added to `LD_LIBRARY_PATH` when it exists.
pub fn tool_env_for(program: &Path) -> Vec<(OsString, OsString)> {
    let real = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_path_buf());
    let mut libs: Vec<std::path::PathBuf> = vec![];
    for p in [program, real.as_path()] {
        if let Some(prefix) = p.parent().and_then(|b| b.parent()) {
            let lib = prefix.join("lib");
            if lib.is_dir() && !libs.contains(&lib) {
                libs.push(lib);
            }
        }
    }
    if libs.is_empty() {
        return vec![];
    }
    let mut value = OsString::new();
    for l in &libs {
        value.push(l);
        value.push(":");
    }
    if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
        value.push(old);
    }
    vec![("LD_LIBRARY_PATH".into(), value)]
}

/// The deck of one run point for a technology whose snapshot has a VACASK model set
/// (`ready`, from [`models::ready`]): the set's OSDI files, the corner's libraries,
/// the translated netlist and the analyses. `set` is the name of the model set.
pub fn build_point(
    project: &crate::Project,
    ready: &models::Ready,
    setup: &crate::sim::Setup,
    set: &str,
    point: &crate::sim::RunPoint,
    title: &str,
    netlist: &str,
) -> Result<deck::Deck> {
    let (_, model_set) = ready
        .tech
        .models
        .set(set)
        .ok_or_else(|| eyre::eyre!("the technology has no models '{set}'"))?;
    let corner = ready
        .tech
        .models
        .lines(set, &point.corner, |n| project.var(n))
        .map_err(|e| eyre::eyre!(e))?;
    let lines =
        deck::model_lines(model_set, corner, |n| project.var(n)).map_err(|e| eyre::eyre!(e))?;
    deck::build(&deck::Inputs {
        setup,
        point,
        title,
        models: &lines,
        netlist,
        defined: ready.defined.as_ref(),
        mm: Some(&ready.stat),
        pdk: &ready.tech.pdk.name,
        notes: &ready.tech.notes,
    })
}

/// Marker lines [`deck::build`] leaves after the control block so that
/// [`run_vacask`] knows which raw file belongs to which analysis and what to measure
/// (the simulator runner gets the deck as text only).
pub const MARK_ANALYSIS: &str = "// cdept-analysis: ";
pub const MARK_MEASURE: &str = "// cdept-measure: ";

/// Write `deck` as `deck.sim` into `dir`, run VACASK, read what it wrote.
///
/// The raw file of the first analysis that is not `op` (else the first) is copied to
/// `out.raw`; every analysis also keeps its own `<name>.raw`. VACASK's own output is
/// `out.log` (with `vacask.stdout`). A failed analysis is an error even though VACASK
/// exits with 0.
pub fn run_vacask(
    program: &Path,
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
) -> Result<Outcome> {
    run_vacask_watched(program, deck, dir, env, None)
}

/// [`run_vacask`], killed when `cancel` is set (a job's cancel flag).
pub fn run_vacask_watched(
    program: &Path,
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Outcome> {
    run_vacask_launch(program, deck, dir, env, &|job| {
        job.run_watched(&crate::batch::Watch {
            cancel: cancel.clone(),
            timeout: None,
        })
    })
}

/// [`run_vacask_watched`] with the way the program is started chosen by the caller
/// (`launch` runs the job and returns its output): a remote run
/// ([`crate::jobs::remote`]) starts it elsewhere and the files are read back the same.
pub fn run_vacask_launch(
    program: &Path,
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    launch: &dyn Fn(&crate::batch::Job) -> Result<crate::batch::Output>,
) -> Result<Outcome> {
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    if deck.starts_with("// CDePT: this testbench cannot be written for VACASK") {
        bail!(
            "{}",
            deck.lines()
                .skip(1)
                .map(|l| l.trim_start_matches("// "))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let deck_file = dir.join("deck.sim");
    let raw = dir.join("out.raw");
    let log = dir.join("out.log");
    std::fs::write(&deck_file, deck)
        .wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "raw") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::remove_file(&log);
    let analyses: Vec<(String, String)> = deck
        .lines()
        .filter_map(|l| l.strip_prefix(MARK_ANALYSIS))
        .filter_map(|l| l.split_once(' '))
        .map(|(n, k)| (n.to_string(), k.trim().to_string()))
        .collect();
    let job = crate::batch::vacask::job(program, "deck.sim", dir, env);
    let out = launch(&job)?;
    let _ = std::fs::copy(&out.log, &log);
    let problems = crate::batch::vacask::problems(&out.text);
    let missing: Vec<&str> = analyses
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| !dir.join(format!("{n}.raw")).is_file())
        .collect();
    if !out.status.success() || !problems.is_empty() || !missing.is_empty() {
        bail!(
            "VACASK failed ({}){}\n{}\nlog: {}",
            out.status,
            if missing.is_empty() {
                String::new()
            } else {
                format!(", no raw file for {}", missing.join(", "))
            },
            if problems.is_empty() {
                crate::batch::tail(&out.text, 12)
            } else {
                format!(
                    "{}\n{}",
                    problems.join("\n"),
                    crate::batch::tail(&out.text, 6)
                )
            },
            log.display()
        );
    }
    let main = analyses
        .iter()
        .find(|(_, k)| k != "op")
        .or(analyses.first());
    if let Some((n, kind)) = main {
        let file = dir.join(format!("{n}.raw"));
        if kind == "noise" {
            // The test templates read ngspice's noise spectra: add them to the copy.
            let plots = raw::read(&file)?;
            let first = plots
                .into_iter()
                .next()
                .ok_or_else(|| eyre::eyre!("empty raw file of '{n}'"))?;
            std::fs::write(&raw, raw::write_real(&raw::with_noise_spectra(first)))
                .wrap_err("cannot write the raw file")?;
        } else {
            std::fs::copy(&file, &raw).wrap_err("cannot copy the raw file")?;
        }
    }
    let mut measures = std::collections::BTreeMap::new();
    for m in deck.lines().filter_map(|l| l.strip_prefix(MARK_MEASURE)) {
        let kind = meas::kind_of(m).unwrap_or_default();
        let Some((name, _)) = analyses.iter().find(|(_, k)| k == kind) else {
            bail!("measurement '{m}': the testbench has no {kind} analysis");
        };
        let plots = raw::read(&dir.join(format!("{name}.raw")))?;
        let plot = plots
            .first()
            .ok_or_else(|| eyre::eyre!("empty raw file of '{name}'"))?;
        if let (Some(value), Some(n)) = (
            meas::evaluate(m, plot).wrap_err_with(|| format!("measurement '{m}'"))?,
            meas::name_of(m),
        ) && value.is_finite()
        {
            measures.insert(n, value);
        }
    }
    Ok(Outcome {
        deck: deck_file,
        raw,
        log,
        measures,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn runs_a_fake_vacask_and_measures() {
        let dir = tempfile::tempdir().unwrap();
        let t: Vec<f64> = (0..=10).map(|i| i as f64).collect();
        let vout: Vec<f64> = t.iter().map(|t| t / 10.0).collect();
        let raw = raw::tests::binary(&["time", "out"], &[t, vout]);
        std::fs::write(dir.path().join("canned.raw"), raw).unwrap();
        let fake = crate::batch::testutil::fake(
            dir.path(),
            "vacask",
            "cp ../canned.raw tran1.raw; echo 'Running analysis tran1'",
        );
        let run = dir.path().join("run");
        let deck = format!(
            "ground 0\ncontrol\nendc\n{MARK_ANALYSIS}tran1 tran\n{MARK_MEASURE}tran half trig v(out) val=0.2 rise=1 targ v(out) val=0.7 rise=1\n"
        );
        let o = run_vacask(&fake, &deck, &run, &[]).unwrap();
        assert!((o.measures["half"] - 5.0).abs() < 1e-9, "{:?}", o.measures);
        assert!(o.raw.is_file() && o.deck.ends_with("deck.sim") && o.log.is_file());
        // An aborted analysis is an error although the exit status is 0.
        let bad = crate::batch::testutil::fake(
            dir.path(),
            "vacask2",
            "echo \"Sweep needs to specify values, mode, or step.\"; echo \"Analysis 'ac1' aborted.\"",
        );
        let err = run_vacask(
            &bad,
            &format!("{MARK_ANALYSIS}ac1 ac\n"),
            &dir.path().join("r2"),
            &[],
        )
        .unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains("aborted") && text.contains("no raw file for ac1"),
            "{text}"
        );
        // A deck the builder could not write is refused, with its message.
        let err = run_vacask(&fake, "// CDePT: this testbench cannot be written for VACASK\n// the technology 'x' has no VACASK model for 'nm'\n", &dir.path().join("r3"), &[]).unwrap_err();
        assert!(format!("{err}").contains("'nm'"), "{err}");
    }

    #[test]
    fn the_install_libraries_are_added_to_the_loader_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::create_dir_all(dir.path().join("lib")).unwrap();
        let env = tool_env_for(&dir.path().join("bin/vacask"));
        assert_eq!(env.len(), 1);
        assert!(
            env[0]
                .1
                .to_string_lossy()
                .starts_with(&format!("{}/lib:", dir.path().display()))
        );
        assert!(tool_env_for(Path::new("/nonexistent/bin/vacask")).is_empty());
    }
}
