//! `cdept ngspice ...`: the in-process simulator (docs/NGSPICE.md).

use std::path::PathBuf;

use clap::Subcommand;
use eyre::{Result, WrapErr, bail};
use libcdept::ngspice::{self, Event, Session};

#[derive(Subcommand)]
pub enum NgspiceCommand {
    /// Say whether libngspice can be loaded and where it is looked for.
    Info,
    /// Run a deck in-process on ngspice's background thread and print the vectors
    /// as they arrive. A `.spiceinit` next to the deck is applied first; the deck's
    /// `.control` commands run, the analysis commands (`run`, `tran`, `ac`, `dc`, `op`)
    /// starting the background run.
    Live {
        deck: PathBuf,
        /// Print one progress line for every N data points.
        #[arg(long, default_value_t = 100)]
        every: usize,
        /// Halt the run after this many data points.
        #[arg(long)]
        halt_after: Option<usize>,
    },
}

pub fn run(command: NgspiceCommand) -> Result<()> {
    match command {
        NgspiceCommand::Info => {
            println!("candidates: {:?}", ngspice::candidates());
            println!("available: {}", ngspice::available());
            Ok(())
        }
        NgspiceCommand::Live {
            deck,
            every,
            halt_after,
        } => live(deck, every.max(1), halt_after),
    }
}

fn is_analysis(cmd: &str) -> bool {
    let word = cmd
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        word.as_str(),
        "run" | "tran" | "ac" | "dc" | "op" | "noise" | "sens" | "tf" | "pz" | "disto"
    )
}

fn follow(
    session: &Session,
    every: usize,
    halt_after: Option<usize>,
    points: &mut usize,
    halted: &mut bool,
) {
    session.wait_background(|ev| match ev {
        Event::Stdout(l) | Event::Stderr(l) => println!("{l}"),
        Event::InitData { plot, vectors } => {
            println!("[{plot}] {} vectors: {}", vectors.len(), vectors.join(" "))
        }
        Event::Data { index, values, .. } => {
            *points += 1;
            if (*points).is_multiple_of(every) {
                let first = values
                    .iter()
                    .next()
                    .map(|(n, v)| format!("{n}={v:.4e}"))
                    .unwrap_or_default();
                println!("point {index} ({} values) {first}", values.len());
            }
            if let Some(n) = halt_after
                && *points >= n
                && !*halted
            {
                *halted = true;
                let _ = session.halt();
            }
        }
        Event::BackgroundRunning(r) => {
            println!("background {}", if *r { "running" } else { "finished" })
        }
        Event::Exit { .. } | Event::Status(_) => {}
    });
}

fn live(deck: PathBuf, every: usize, halt_after: Option<usize>) -> Result<()> {
    let text = std::fs::read_to_string(&deck)
        .wrap_err_with(|| format!("cannot read {}", deck.display()))?;
    let dir = deck
        .canonicalize()?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    if !ngspice::available() {
        bail!("libngspice not found; set ${}", ngspice::LIB_ENV);
    }
    let session = Session::open()?;
    std::env::set_current_dir(&dir)?;
    if let Ok(init) = std::fs::read_to_string(dir.join(".spiceinit")) {
        for line in init
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('*'))
        {
            let _ = session.command(line);
        }
    }
    session.load_deck(&ngspice::strip_control_blocks(&text))?;
    let (mut points, mut halted, mut ran, mut inside, mut has_control) =
        (0usize, false, false, false, false);
    for line in text.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if lower.starts_with(".control") {
            inside = true;
            has_control = true;
        } else if lower.starts_with(".endc") {
            inside = false;
        } else if inside && !t.is_empty() && !t.starts_with('*') {
            if is_analysis(t) {
                if !ran {
                    // Any command can run on the background thread with a `bg_` prefix.
                    session.command(&format!("bg_{t}"))?;
                    follow(&session, every, halt_after, &mut points, &mut halted);
                    ran = true;
                }
            } else if let Err(e) = session.command(t) {
                println!("warning: {e:#}");
            }
        }
    }
    if !has_control {
        session.run_background()?;
        follow(&session, every, halt_after, &mut points, &mut halted);
    }
    println!(
        "{points} data points{}",
        if halted { " (halted)" } else { "" }
    );
    for plot in session.plot_names() {
        let names = session.vectors(&plot);
        println!("plot {plot}: {} vectors", names.len());
        for n in names.iter().take(8) {
            if let Ok(v) = session.vector(&plot, n) {
                println!("  {n}: {} values", v.len());
            }
        }
    }
    Ok(())
}
