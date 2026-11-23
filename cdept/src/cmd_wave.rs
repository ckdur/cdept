//! `cdept wave`: open a raw file, or the raw of a run point, in UberWave (docs/howto/view-waveforms.md).

use std::path::Path;

use clap::Args;
use eyre::{Result, eyre};
use libcdept::Project;
use libcdept::wave;

#[derive(Args)]
pub struct WaveArgs {
    /// A raw file (ngspice or Xyce) or a Spectre PSF folder; or a library, followed by the cell
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
    /// Do not start the viewer: for a file, print its analyses and signal counts (CDePT's
    /// own reader); for a cell, the raw files of all its run points.
    #[arg(long)]
    list: bool,
}

pub fn run(args: WaveArgs, project: Option<&Project>) -> Result<()> {
    match &args.cell {
        None if args.family || !args.points.is_empty() || args.calc.is_some() => Err(eyre!(
            "--family, --points and --calc work on the run points of a testbench: cdept wave <library> <cell> --view V ..."
        )),
        None => file(&args, Path::new(&args.first), project),
        Some(cell) => point(&args, cell, project),
    }
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
    let launched = wave::open_raw(project, raw, args.analysis.as_deref())?;
    println!("{}", launched.text());
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
    let opened = if several {
        wave::open_family(
            project,
            library,
            cell,
            &view,
            &chosen,
            args.analysis.as_deref(),
        )?
    } else {
        wave::open_point_raw(
            project,
            library,
            cell,
            &view,
            &chosen[0],
            args.analysis.as_deref(),
        )?
    };
    println!("{}", opened.text());
    if let Some(note) = opened.old_viewer_note() {
        eprintln!("note: {note}");
    }
    Ok(())
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
    Ok(())
}
