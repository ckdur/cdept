//! `cdept calc`: evaluate a calculator expression on a SPICE raw file (docs/CALC.md).

use std::path::PathBuf;

use clap::Args;
use eyre::{Result, bail};
use libcdept::calc::{self, Env, Value, funcs};
use libcdept::raw::RawFile;

#[derive(Args)]
pub struct CalcArgs {
    /// The expression, e.g. `ugbw(v("out") / v("in"))` or `delay(v("in"), v("out"), 0.6)`.
    expr: Option<String>,
    /// The raw file (ngspice or Xyce, binary or ascii; a run's `out.raw`).
    #[arg(long)]
    raw: Option<PathBuf>,
    /// Look only in this plot: its number, its kind (tran, ac, dc, noise, op) or part of
    /// its name.
    #[arg(long)]
    plot: Option<String>,
    /// A name for the expression, `name=value` (SPICE suffixes work: `vdd=1.2`,
    /// `cload=10f`). May be repeated.
    #[arg(long = "var")]
    vars: Vec<String>,
    /// List the plots and vectors of the raw file and stop.
    #[arg(long)]
    list: bool,
    /// List the functions and stop.
    #[arg(long)]
    functions: bool,
    /// For a waveform result, print every point (x y), not the summary.
    #[arg(long)]
    dump: bool,
}

pub fn run(args: CalcArgs) -> Result<()> {
    if args.functions {
        for f in funcs::all() {
            let params: Vec<String> = f
                .params
                .iter()
                .map(|p| {
                    p.strip_prefix('?')
                        .map_or(p.to_string(), |o| format!("[{o}]"))
                })
                .collect();
            println!("{}({})", f.names.join(" | "), params.join(", "));
            println!("    {}", f.doc);
        }
        return Ok(());
    }
    let raw = match &args.raw {
        Some(file) => Some(RawFile::open(file)?),
        None => None,
    };
    if args.list {
        let Some(raw) = &raw else {
            bail!("--list needs --raw FILE");
        };
        for (i, p) in raw.plots().iter().enumerate() {
            println!(
                "plot {i}: {} [{}], {} points{}, {} vectors{}",
                p.name,
                p.kind(),
                p.points,
                if p.declared_points != p.points {
                    format!(" (of {} announced)", p.declared_points)
                } else {
                    String::new()
                },
                p.vars.len(),
                if p.complex { ", complex" } else { "" }
            );
            for v in &p.vars {
                println!("    {:<24} {}", v.name, v.unit);
            }
        }
        return Ok(());
    }
    let Some(expr) = &args.expr else {
        bail!("give an expression (or --list / --functions)");
    };
    let mut env = match &raw {
        Some(r) => Env::new(r),
        None => Env::empty(),
    };
    if let Some(p) = &args.plot {
        env = env.with_plot(p.clone());
    }
    for v in &args.vars {
        let Some((k, val)) = v.split_once('=') else {
            bail!("--var wants name=value, got '{v}'");
        };
        env.set_text(k.trim(), val.trim());
    }
    match calc::evaluate(expr, &env) {
        Ok(Value::Wave(w)) if args.dump => {
            for i in 0..w.len() {
                let y = w.y.at(i);
                if w.y.is_complex() {
                    println!("{:e} {:e} {:e}", w.x[i], y.re, y.im);
                } else {
                    println!("{:e} {:e}", w.x[i], y.re);
                }
            }
        }
        Ok(v) => println!("{}", v.display()),
        Err(e) => {
            bail!("{}", e.render(expr));
        }
    }
    Ok(())
}
