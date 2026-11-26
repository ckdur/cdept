//! `cdept wave`: open a raw file, or the raw of a run point, in UberWave (docs/howto/view-waveforms.md).

use std::path::{Path, PathBuf};

use clap::Args;
use eyre::{Result, eyre};
use libcdept::Project;
use libcdept::wave;

#[derive(Args)]
pub struct WaveArgs {
    /// A raw file (ngspice or Xyce), a Spectre PSF folder, a CSV table or a digital waveform
    /// (VCD, FST, GHW); or a library, followed by the cell
    /// of a testbench whose run point is opened.
    first: String,
    /// The cell, to open the raw file of a run point of its testbench.
    cell: Option<String>,
    /// The analysis to preselect (`tran`, `ac`, `dc`, `noise`, `op`; a PSF analysis name).
    /// Default: the transient one when the file has several.
    #[arg(long)]
    analysis: Option<String>,
    /// The testbench view. Default: the only view of the cell that has results.
    #[arg(long)]
    view: Option<String>,
    /// The run point (a folder of simulations/<lib>/<cell>/<view>/). Default: the only one.
    #[arg(long)]
    point: Option<String>,
    /// Open all the run points of the view in one viewer (tags = point names). Needs a
    /// viewer with IPC for more than the first; an older one opens the first and the other
    /// files are listed.
    #[arg(long)]
    family: bool,
    /// Open these run points (comma separated) in one viewer, as `--family` does for all.
    #[arg(long, value_delimiter = ',')]
    points: Vec<String>,
    /// Evaluate a calculator expression on the point (or on every point with `--family` /
    /// `--points`) and send waveform results to the viewer as traces (started when it is
    /// not running; needs a viewer with IPC). Scalar results are printed, as a table for
    /// several points. A complex waveform is sent as `<name> (mag dB)` and `<name> (phase)`.
    #[arg(long)]
    calc: Option<String>,
    /// The name of the trace sent by `--calc` (default: the expression); with several
    /// points `<name> @ <point>`.
    #[arg(long)]
    name: Option<String>,
    /// Signals to plot (comma separated, `v(out)` or just `out`; the tag of each file is
    /// added). Default: the nodes the testbench `save`s, else the ones its measures use,
    /// else the first four voltages that are not ground or a supply. For a CSV table or a
    /// digital file (VCD, FST, GHW) the columns or signals sent to UberWave as traces (a
    /// digital file without --show opens in Surfer or GTKWave).
    #[arg(long, value_delimiter = ',', conflicts_with = "no_show")]
    show: Vec<String>,
    /// With --calc: a complex (AC) result as a Bode plot, magnitude and phase in two new
    /// panes on a log frequency axis.
    #[arg(long, conflicts_with = "eye")]
    pub bode: bool,
    /// With --calc: the waveform result as an eye diagram, every PERIOD (from OFFSET, default
    /// the start) drawn on top of the others in a new pane: `--eye 1n` or `--eye 1n,250p`.
    #[arg(long, value_name = "PERIOD[,OFFSET]")]
    pub eye: Option<String>,
    /// With --calc: no markers where scalar results were measured (crossings, edges,
    /// extremes; built-in UberWave only).
    #[arg(long)]
    pub no_markers: bool,
    /// Plot nothing: the viewer opens as it is (File traces are added in the viewer).
    #[arg(long)]
    no_show: bool,
    /// Send calculator traces whole, however many points they have (by default a trace
    /// with more than `[wave] max_points` of cdept.toml, 200000, goes as a min/max
    /// envelope of half of them, named `... (decimated)`).
    #[arg(long)]
    no_decimate: bool,
    /// Which UberWave: `embedded` (built into this cdept: its window opens in this process
    /// and `cdept wave` waits until it is closed) or `process` (the separate
    /// `UberWaveViewer`, started and left running). Default: `embedded` when built in, else
    /// `process`; also `$CDEPT_UBERWAVE_BACKEND` and `[wave] backend` of cdept.toml.
    #[arg(long, value_parser = ["embedded", "process"])]
    backend: Option<String>,
    /// Save the viewer window as a PNG once everything is loaded and exit (embedded UberWave
    /// only; the window stays hidden and is 1280x720): for documentation and tests.
    #[arg(long, value_name = "PNG")]
    screenshot: Option<PathBuf>,
    /// Do not start the viewer: for a file, print its analyses and signal counts (CDePT's
    /// own reader); for a cell, the raw files of all its run points.
    #[arg(long)]
    list: bool,
}

impl WaveArgs {
    /// What was asked for with `--show` / `--no-show`; `None`: the default.
    fn show_choice(&self) -> Option<wave::show::Show> {
        if self.no_show {
            Some(wave::show::Show::Off)
        } else if !self.show.is_empty() {
            Some(wave::show::Show::Names(self.show.clone()))
        } else {
            None
        }
    }
}

fn show_request(
    args: &WaveArgs,
    project: &Project,
    (library, cell, view): (&str, &str, &str),
) -> wave::show::ShowRequest {
    wave::show::ShowRequest::for_testbench(
        project,
        library,
        cell,
        view,
        args.show_choice().unwrap_or_default(),
    )
}

pub fn run(args: WaveArgs, project: Option<&Project>) -> Result<()> {
    if let Some(b) = args
        .backend
        .as_deref()
        .and_then(wave::backend::Backend::parse)
    {
        wave::backend::set_override(Some(b));
    }
    if args.screenshot.is_some() {
        wave::embedded::configure(wave::embedded::Options {
            title: Some("UberWave (cdept)".into()),
            width: 1280,
            height: 720,
            headless: true,
            ..Default::default()
        });
    }
    match &args.cell {
        None if args.family || !args.points.is_empty() || args.calc.is_some() => Err(eyre!(
            "--family, --points and --calc work on the run points of a testbench: cdept wave <library> <cell> --view V ..."
        )),
        None => file(&args, Path::new(&args.first), project),
        Some(cell) => point(&args, cell, project),
    }
}

fn layout_of(args: &WaveArgs) -> Result<wave::calc::Layout> {
    if args.bode {
        return Ok(wave::calc::Layout::Bode);
    }
    let Some(eye) = &args.eye else {
        return Ok(wave::calc::Layout::Plain);
    };
    let mut parts = eye.split(',');
    let num = |t: Option<&str>| -> Result<Option<f64>> {
        match t.map(str::trim).filter(|t| !t.is_empty()) {
            None => Ok(None),
            Some(t) => libcdept::calc::parse_spice_number(t)
                .map(Some)
                .ok_or_else(|| eyre!("--eye: '{t}' is not a number")),
        }
    };
    let period = num(parts.next())?.ok_or_else(|| eyre!("--eye needs a period"))?;
    if !(period > 0.0 && period.is_finite()) {
        return Err(eyre!("--eye: the period must be positive"));
    }
    let offset = num(parts.next())?.unwrap_or(0.0);
    Ok(wave::calc::Layout::Eye { period, offset })
}

fn file(args: &WaveArgs, raw: &Path, project: Option<&Project>) -> Result<()> {
    if args.list {
        if !raw.is_file() {
            return Err(eyre!("{} is not a raw file", raw.display()));
        }
        for a in wave::analyses(raw)? {
            println!(
                "{}\t{}\t{} signals\t{} points",
                a.kind, a.name, a.signals, a.points
            );
        }
        return Ok(());
    }
    match wave::other::kind(raw) {
        wave::other::Kind::Digital if args.show.is_empty() => {
            let line = wave::other::open_digital(project, raw)?;
            let words: Vec<String> = line
                .iter()
                .map(|w| w.to_string_lossy().into_owned())
                .collect();
            println!("{}", words.join(" "));
            return Ok(());
        }
        wave::other::Kind::Digital | wave::other::Kind::Table => {
            let sent = wave::other::open_in_uberwave(project, raw, &args.show)?;
            for (name, n) in &sent.pushed {
                println!("sent '{name}' ({n} points)");
            }
            for note in &sent.notes {
                eprintln!("note: {note}");
            }
            return finish(args, true);
        }
        wave::other::Kind::Spice if !args.show.is_empty() => {
            return Err(eyre!(
                "--show works on a CSV table, a digital file or the run points of a testbench (cdept wave <library> <cell> --view V ...)"
            ));
        }
        wave::other::Kind::Spice => {}
    }
    let launched = wave::open_raw(project, raw, args.analysis.as_deref())?;
    println!("{}", launched.text());
    finish(args, launched.embedded)
}

/// With the embedded UberWave the window belongs to this process: save the screenshot
/// that was asked for and leave, or stay until the window is closed.
fn finish(args: &WaveArgs, embedded: bool) -> Result<()> {
    if !embedded {
        if args.screenshot.is_some() {
            return Err(eyre!(
                "--screenshot needs the embedded UberWave (this cdept uses the UberWaveViewer process: see --backend)"
            ));
        }
        return Ok(());
    }
    if let Some(png) = &args.screenshot {
        let png = std::path::absolute(png)?;
        let viewer =
            wave::embedded::running().ok_or_else(|| eyre!("the viewer window is not open"))?;
        let saved = viewer.screenshot(&png);
        wave::embedded::close();
        saved?;
        println!("screenshot {}", png.display());
        return Ok(());
    }
    eprintln!("UberWave is open in this process: close its window to end cdept wave");
    wave::embedded::wait_closed();
    Ok(())
}

fn point(args: &WaveArgs, cell: &str, project: Option<&Project>) -> Result<()> {
    let project = project.ok_or_else(|| eyre!("no project (run inside one or use --project)"))?;
    let library = &args.first;
    let view = wave::pick_view(project, library, cell, args.view.as_deref())?;
    let all = wave::points(project, library, cell, &view);
    if args.list {
        for p in &all {
            println!("{}\t{}", p.point, p.raw.display());
        }
        return Ok(());
    }
    let several = args.family || !args.points.is_empty();
    if several && args.point.is_some() {
        return Err(eyre!(
            "--point names one run point: use --points a,b,c or --family for several"
        ));
    }
    let chosen: Vec<wave::PointRaw> = if several {
        wave::select_points(project, library, cell, &view, &args.points)?
    } else {
        vec![wave::pick_point(
            project,
            library,
            cell,
            &view,
            args.point.as_deref(),
        )?]
    };
    if let Some(expr) = &args.calc {
        return calc(args, project, (library, cell, &view), &chosen, expr);
    }
    let show = show_request(args, project, (library, cell, &view));
    let opened = if several {
        wave::open_family(
            project,
            library,
            cell,
            &view,
            &chosen,
            args.analysis.as_deref(),
            &show,
        )?
    } else {
        wave::open_point_raw(
            project,
            library,
            cell,
            &view,
            &chosen[0],
            args.analysis.as_deref(),
            &show,
        )?
    };
    println!("{}", opened.text());
    for note in &opened.notes {
        eprintln!("note: {note}");
    }
    if let Some(note) = opened.old_viewer_note() {
        eprintln!("note: {note}");
    }
    finish(args, opened.embedded)
}

fn calc(
    args: &WaveArgs,
    project: &Project,
    (library, cell, view): (&str, &str, &str),
    chosen: &[wave::PointRaw],
    expr: &str,
) -> Result<()> {
    let req = wave::calc::Request {
        expr: expr.to_string(),
        name: args.name.clone(),
        plot: args.analysis.clone(),
        show: args.show_choice(),
        max_points: args.no_decimate.then_some(0),
        markers: !args.no_markers,
        layout: layout_of(args)?,
        extra: vec![],
    };
    let sent = wave::calc::send(project, library, cell, view, chosen, &req);
    let table = sent.table();
    if chosen.len() == 1 && sent.outcomes[0].result.is_ok() && !table.is_empty() {
        // One scalar: just the value.
        println!(
            "{}",
            table.split_once('\t').map_or(table.as_str(), |(_, v)| v)
        );
    } else if !table.is_empty() {
        println!("{table}");
    }
    for note in &sent.notes {
        eprintln!("note: {note}");
    }
    for (name, n) in &sent.pushed {
        println!("sent trace '{name}' ({n} points)");
    }
    if let Some(v) = &sent.viewer
        && !sent.pushed.is_empty()
    {
        eprintln!("viewer: {v}");
    }
    let failed: Vec<String> = sent
        .outcomes
        .iter()
        .filter_map(|o| o.result.as_ref().err().map(|e| format!("{}: {e}", o.point)))
        .collect();
    if let Some(problem) = &sent.problem {
        return Err(eyre!("the traces were not sent: {problem}"));
    }
    if !failed.is_empty() {
        return Err(eyre!("{}", failed.join("\n")));
    }
    finish(args, sent.embedded)
}
