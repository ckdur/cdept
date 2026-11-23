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
    /// Do not start the viewer: for a file, print its analyses and signal counts (CDePT's
    /// own reader); for a cell, the raw files of all its run points.
    #[arg(long)]
    list: bool,
}

pub fn run(args: WaveArgs, project: Option<&Project>) -> Result<()> {
    match &args.cell {
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
            println!("{}\t{}\t{} signals\t{} points", a.kind, a.name, a.signals, a.points);
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
    let chosen = wave::pick_point(project, library, cell, &view, args.point.as_deref())?;
    let launched = wave::open_raw(Some(project), &chosen.raw, args.analysis.as_deref())?;
    println!("{}", launched.text());
    if all.len() > 1 {
        eprintln!(
            "note: UberWave opens one file at a time; the other {} point(s) are File > Import (`cdept wave --list {library} {cell} --view {view}` prints their files)",
            all.len() - 1
        );
    }
    Ok(())
}
