//! Remote execution of run points (SYMBI-82): smoke tests against fake `ssh`, `sbatch`,
//! `squeue`, `sacct` programs, and the real ngspice behind them. Never a cluster.
//!
//! - hermetic: the `[jobs]` table of a project file, a run point through a fake ssh and a fake
//!   SLURM with a stand-in simulator;
//! - `real_*`: examples/ihp-inv/tb_inv through the fake ssh and the fake sbatch (which runs the
//!   batch script here with `sh`) with the real ngspice: the same numbers as a local run;
//! - `real_ssh_localhost`: the same through a real `ssh localhost`, when one logs in without a
//!   password (else skipped).

use std::path::{Path, PathBuf};

use libcdept::batch::Watch;
use libcdept::jobs::remote::{Kind, Remote};
use libcdept::sim::run::SimKind;
use libcdept::sim::{self, RunOptions, Simulator};
use libcdept::testutil::{hermetic_env, real_pdk};
use libcdept::{PROJECT_FILE, Project};

#[cfg(unix)]
fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A fake `ssh` (skips options and host, runs the command here) and a fake SLURM whose
/// `sbatch` runs the batch script synchronously with `sh`; `squeue` knows no job any more and
/// `sacct` says COMPLETED. Returns the toml of the two remotes `fakessh` and `fakeslurm`;
/// `program` is the simulator on the "remote".
#[cfg(unix)]
fn fake_remotes(dir: &Path, program: &str) -> String {
    let d = dir.display();
    fake(
        dir,
        "ssh",
        "while [ \"$1\" = -o ] || [ \"$1\" = -p ] || [ \"$1\" = -i ]; do shift 2; done\nshift\nexec sh -c \"$*\"",
    );
    fake(dir, "sbatch", "sh job.sbatch > /dev/null 2>&1\necho 31415");
    fake(dir, "squeue", "exit 0");
    fake(dir, "sacct", "echo 'COMPLETED|0:0'");
    fake(dir, "scancel", "exit 0");
    format!(
        "[jobs]\n[jobs.remote.fakessh]\nkind = \"ssh\"\nhost = \"fakehost\"\nssh = \"{d}/ssh\"\nprogram = \"{program}\"\nmax_jobs = 2\n\
         [jobs.remote.fakeslurm]\nkind = \"slurm\"\nprogram = \"{program}\"\nsbatch = \"{d}/sbatch\"\nsqueue = \"{d}/squeue\"\n\
         sacct = \"{d}/sacct\"\nscancel = \"{d}/scancel\"\npoll = \"20ms\"\nfile_wait = \"1s\"\npartition = \"fake\"\nmax_jobs = 3\n"
    )
}

#[cfg(unix)]
#[test]
fn the_jobs_table_of_a_project_file() {
    let _env = hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(PROJECT_FILE),
        "name = \"p\"\n[jobs]\ndefault = \"hpc\"\nbogus = 1\n\
         [jobs.remote.hpc]\nkind = \"slurm\"\nhost = \"login1\"\npartition = \"short\"\nmax_jobs = 16\n\
         [jobs.remote.box]\nkind = \"ssh\"\nhost = \"me@box\"\nworkdir_mode = \"rsync\"\nremote_dir = \"/scratch/cdept\"\n\
         [jobs.remote.broken]\nkind = \"ssh\"\n",
    )
    .unwrap();
    let project = Project::load(tmp.path()).unwrap();
    assert_eq!(
        project.jobs.remotes.keys().collect::<Vec<_>>(),
        ["box", "hpc"]
    );
    let hpc = project.jobs.get("hpc").unwrap();
    assert_eq!((hpc.kind, hpc.max_jobs), (Kind::Slurm, 16));
    assert_eq!(project.jobs.pick(None, "").unwrap().unwrap().name, "hpc");
    assert_eq!(project.jobs.pick(None, "box").unwrap().unwrap().name, "box");
    let w = project.warnings.join("\n");
    assert!(
        w.contains("bogus") && w.contains("jobs.remote.broken"),
        "{w}"
    );
}

/// A deck the stand-in ngspice understands: it answers with a measurement and a raw file.
#[cfg(unix)]
fn stand_in_ngspice(dir: &Path) -> PathBuf {
    fake(
        dir,
        "fake-ngspice",
        "[ \"$1 $2 $3 $4\" = '-b -o out.log deck.sp' ] || exit 2\n\
         [ -f deck.sp ] || exit 3\n\
         echo \"tpd = 1.5e-10 from $(basename $(pwd)) at $SIM_ENV\" > out.log\necho raw > out.raw",
    )
}

#[cfg(unix)]
#[test]
fn a_run_point_through_the_fake_ssh_and_the_fake_slurm() {
    let _env = hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let sim = stand_in_ngspice(tmp.path());
    let toml = fake_remotes(tmp.path(), &sim.to_string_lossy());
    let root: toml::Table = toml::from_str(&toml).unwrap();
    let mut warnings = vec![];
    let jobs = libcdept::jobs::remote::JobsSettings::load(&root, &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    for name in ["fakessh", "fakeslurm"] {
        let remote: &Remote = jobs.get(name).unwrap();
        let dir = tmp.path().join(format!("run-{name}"));
        let env = vec![("SIM_ENV".into(), "from-project".into())];
        let out = sim::remote::run(
            remote,
            SimKind::Ngspice,
            ".end\n",
            &dir,
            &env,
            &Watch::default(),
            &|_| {},
        )
        .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(out.measures["tpd"], 1.5e-10, "{name}");
        assert!(out.raw.is_file() && out.deck.is_file(), "{name}");
        let log = std::fs::read_to_string(&out.log).unwrap();
        assert!(
            log.contains(&format!("run-{name} at from-project")),
            "{name}: {log}"
        );
        // The point is the folder of a local run: the deck and the result files.
        assert_eq!(
            std::fs::read_to_string(dir.join("deck.sp")).unwrap(),
            ".end\n"
        );
    }
    // SLURM left its script and exit status in the folder.
    let dir = tmp.path().join("run-fakeslurm");
    assert!(
        std::fs::read_to_string(dir.join("job.sbatch"))
            .unwrap()
            .contains("#SBATCH --partition=fake")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("exit_status"))
            .unwrap()
            .trim(),
        "0"
    );

    // A simulator that fails: its status and log come back as for a local run.
    let broken = fake(
        tmp.path(),
        "broken",
        "echo 'boom: no such model' > out.log; exit 1",
    );
    let mut remote = jobs.get("fakeslurm").unwrap().clone();
    remote.program = broken.to_string_lossy().into_owned();
    let err = sim::remote::run(
        &remote,
        SimKind::Ngspice,
        ".end\n",
        &tmp.path().join("run-bad"),
        &[],
        &Watch::default(),
        &|_| {},
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("boom: no such model"),
        "{err:#}"
    );
}

#[cfg(unix)]
#[test]
fn a_remote_run_of_xyce_reads_the_files_back() {
    let _env = hermetic_env();
    let tmp = tempfile::tempdir().unwrap();
    let xyce = fake(
        tmp.path(),
        "fake-xyce",
        "[ \"$1 $2 $3 $4 $5\" = '-r out.raw -l out.log deck.cir' ] || exit 2\n\
         echo raw > out.raw\necho 'GAIN = 2.5' > out.log\necho 'GAIN = 2.5' > deck.cir.mt0",
    );
    let toml = fake_remotes(tmp.path(), &xyce.to_string_lossy());
    let root: toml::Table = toml::from_str(&toml).unwrap();
    let jobs = libcdept::jobs::remote::JobsSettings::load(&root, &mut vec![]);
    for name in ["fakessh", "fakeslurm"] {
        let out = sim::remote::run(
            jobs.get(name).unwrap(),
            SimKind::Xyce,
            ".end\n",
            &tmp.path().join(format!("x-{name}")),
            &[],
            &Watch::default(),
            &|_| {},
        )
        .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(out.measures["gain"], 2.5, "{name}");
    }
}

// ---- the real ngspice, behind the fakes ----

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// tb_inv's `tt_27` point: its `tpd` measure and the raw file's size, run with `remote`.
#[cfg(unix)]
fn tb_inv(extra_toml: impl Fn(&Path, &str) -> String, remote: Option<&str>) -> Option<(f64, u64)> {
    let ngspice = Simulator::find()?;
    if libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: the xschem helper is not built");
        return None;
    }
    let pdk = real_pdk("ihp-sg13g2")?;
    let dir = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv"),
        dir.path(),
    );
    let fakes = tempfile::tempdir().unwrap();
    let program = ngspice.program.to_string_lossy().into_owned();
    let mut toml = std::fs::read_to_string(dir.path().join(PROJECT_FILE)).unwrap();
    toml.push('\n');
    toml.push_str(&extra_toml(fakes.path(), &program));
    std::fs::write(dir.path().join(PROJECT_FILE), toml).unwrap();
    std::fs::write(
        dir.path().join("cdept.local.toml"),
        format!("pdk_root = \"{}\"\n", pdk.root.display()),
    )
    .unwrap();
    let project = Project::load(dir.path()).unwrap();
    let tb = sim::open(&project, "cells", "tb_inv", "testbench").unwrap();
    let options = RunOptions {
        remote: remote.map(str::to_string),
        no_store: true,
        ..Default::default()
    };
    // The local program is not used for a remote run.
    let simulator = if remote.is_some() {
        Simulator::for_remote(&tb.setup).unwrap()
    } else {
        ngspice
    };
    let results =
        sim::run_testbench_with(&project, &tb, &simulator, &["tt".to_string()], &options).unwrap();
    let r = results.first().expect("a result");
    let out = r.outcome.as_ref().unwrap_or_else(|e| panic!("{e:#}"));
    if remote.is_some() {
        let folder = out.raw.parent().unwrap();
        let log = std::fs::read_to_string(folder.join("job.log")).unwrap_or_default();
        assert!(
            log.contains("submitted")
                || log.contains("ssh fakehost")
                || log.contains("ssh localhost"),
            "the job log says where it ran:\n{log}"
        );
    }
    Some((out.measures["tpd"], out.raw.metadata().unwrap().len()))
}

#[cfg(unix)]
#[test]
fn real_tb_inv_through_fake_ssh_and_fake_slurm_matches_a_local_run() {
    let Some(local) = tb_inv(|_, _| String::new(), None) else {
        return;
    };
    assert!(local.1 > 1000);
    for name in ["fakessh", "fakeslurm"] {
        let remote = tb_inv(fake_remotes, Some(name)).expect("the same tools as the local run");
        assert_eq!(remote.0, local.0, "{name}: tpd");
        assert_eq!(remote.1, local.1, "{name}: raw file size");
    }
}

#[cfg(unix)]
#[test]
fn real_a_remote_refuses_the_in_process_backend() {
    let Some(ngspice) = Simulator::find() else {
        return;
    };
    if libcdept::xschem::Engine::find_helper().is_none() {
        return;
    }
    let Some(pdk) = real_pdk("ihp-sg13g2") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv"),
        dir.path(),
    );
    let mut toml = std::fs::read_to_string(dir.path().join(PROJECT_FILE)).unwrap();
    toml.push_str("\n[jobs.remote.box]\nkind = \"ssh\"\nhost = \"box\"\n");
    std::fs::write(dir.path().join(PROJECT_FILE), toml).unwrap();
    std::fs::write(
        dir.path().join("cdept.local.toml"),
        format!("pdk_root = \"{}\"\n", pdk.root.display()),
    )
    .unwrap();
    let project = Project::load(dir.path()).unwrap();
    let mut tb = sim::open(&project, "cells", "tb_inv", "testbench").unwrap();
    tb.setup.simulator_backend = "inproc".into();
    let options = RunOptions {
        remote: Some("box".into()),
        no_store: true,
        ..Default::default()
    };
    let err = match sim::run_testbench_with(&project, &tb, &ngspice, &[], &options) {
        Ok(_) => panic!("the in-process backend ran on a remote"),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        err.contains("in-process backend cannot run on the remote 'box'"),
        "{err}"
    );
    let options = RunOptions {
        remote: Some("nowhere".into()),
        ..options
    };
    let err = match sim::run_testbench_with(&project, &tb, &ngspice, &[], &options) {
        Ok(_) => panic!("an unknown remote ran"),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        err.contains("no remote 'nowhere'") && err.contains("box"),
        "{err}"
    );
}

/// A real `ssh localhost`, when one logs in without a password and the project is on a
/// folder both sides see (it is: the same machine). Skipped otherwise.
#[cfg(unix)]
#[test]
fn real_ssh_localhost() {
    let up = std::process::Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "localhost",
            "true",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !up {
        eprintln!("skipped: `ssh -o BatchMode=yes localhost true` does not work");
        return;
    }
    let Some(local) = tb_inv(|_, _| String::new(), None) else {
        return;
    };
    let over_ssh = tb_inv(
        |_, program| {
            format!("[jobs.remote.local]\nkind = \"ssh\"\nhost = \"localhost\"\nprogram = \"{program}\"\n")
        },
        Some("local"),
    )
    .unwrap();
    assert_eq!(over_ssh.0, local.0);
}
