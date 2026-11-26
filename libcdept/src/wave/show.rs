//! What the viewer plots after files were opened over IPC. A file opened by the command
//! line, `open` or `import` plots nothing until `show` is sent, so CDePT sends it: for
//! each file (tag = point name) the signals of [`Show`], all in one pane.
//!
//! The default signals of a point are, in this order of preference: the nodes the
//! testbench `save`s (when the raw file has them), the signals its measures and outputs
//! use, the first four voltages that are not an axis, ground or a supply.

use std::time::{Duration, Instant};

use super::backend::ViewerOps;
use super::ipc::{PaneSel, Signal};
use super::session::ViewFile;
use crate::Project;

/// How long to wait for a freshly started viewer to have loaded the files.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// The most signals the default shows per point.
const MAX_DEFAULT: usize = 8;
/// The most voltages the fallback shows per point.
const MAX_FALLBACK: usize = 4;

/// Which signals to plot.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Show {
    /// The default described in the module documentation.
    #[default]
    Default,
    /// These signals of every file (`v(out)`, or just `out`), without the `tag:`.
    Names(Vec<String>),
    /// Nothing: the viewer stays as it opens.
    Off,
}

/// What to show and what the testbench says about its signals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShowRequest {
    pub show: Show,
    /// `save` of the testbench's setup.
    pub save: Vec<String>,
    /// Signals its measures and outputs use.
    pub measured: Vec<String>,
}

impl ShowRequest {
    /// Show nothing.
    pub fn off() -> ShowRequest {
        ShowRequest {
            show: Show::Off,
            ..Default::default()
        }
    }

    /// `show` for the testbench view `library/cell/view`, with the hints its `setup.toml`
    /// gives (none when it cannot be read: the fallback still plots something).
    pub fn for_testbench(
        project: &Project,
        library: &str,
        cell: &str,
        view: &str,
        show: Show,
    ) -> ShowRequest {
        let mut req = ShowRequest {
            show,
            ..Default::default()
        };
        let file = project
            .library(library)
            .and_then(|l| l.cell(cell))
            .and_then(|c| c.entry(view))
            .map(|e| e.path.join(crate::sim::setup::SETUP_FILE));
        let setup = file.filter(|f| f.is_file()).and_then(|f| {
            crate::sim::Setup::load_with(
                &f,
                &crate::sim::template::TemplateSet::for_project(project),
            )
            .ok()
        });
        if let Some(setup) = setup {
            req.save = setup.save.clone();
            let mut seen = Vec::new();
            for text in setup
                .measures
                .iter()
                .chain(setup.outputs.iter().map(|o| &o.expr))
            {
                for s in signals_in(text) {
                    if !seen.contains(&s) {
                        seen.push(s);
                    }
                }
            }
            req.measured = seen;
        }
        req
    }

    /// Anything is to be shown.
    pub fn enabled(&self) -> bool {
        self.show != Show::Off
    }
}

/// The signals an expression or a `.meas` line names: `v(out)`, `v("out")`, `i(vdd)`
/// as `v(out)` / `i(vdd)` (lower case). A call with other arguments (`v("out", plot=..)`)
/// gives its first one.
pub fn signals_in(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let b = lower.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if before_ok && (b[i] == b'v' || b[i] == b'i') && b[i + 1] == b'(' {
            let rest = &lower[i + 2..];
            if let Some(end) = rest.find(')') {
                let first = rest[..end]
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .trim();
                if !first.is_empty() && !first.contains('=') {
                    let sig = format!("{}({first})", b[i] as char);
                    if !out.contains(&sig) {
                        out.push(sig);
                    }
                }
                i += 2 + end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A signal name in one spelling: lower case, `v(out)` as `out`, `i(vdd)` and `vdd#branch`
/// as `i:vdd` (ngspice and Xyce write the same signal differently).
fn norm(name: &str) -> String {
    let n = name.trim().to_lowercase();
    if let Some(inner) = n.strip_prefix("v(").and_then(|s| s.strip_suffix(')')) {
        return inner.trim().to_string();
    }
    if let Some(inner) = n.strip_prefix("i(").and_then(|s| s.strip_suffix(')')) {
        return format!("i:{}", inner.trim());
    }
    if let Some(inner) = n.strip_suffix("#branch") {
        return format!("i:{inner}");
    }
    n
}

fn is_axis(s: &Signal) -> bool {
    let n = s.name.to_lowercase();
    matches!(n.as_str(), "time" | "frequency" | "freq")
        || n.contains("sweep")
        || s.unit == "s"
        || s.unit == "Hz"
}

fn is_ground_or_supply(name: &str) -> bool {
    let base = norm(name);
    let last = base.rsplit(['.', ':', '/']).next().unwrap_or(&base);
    if last == "0" {
        return true;
    }
    const SUPPLIES: [&str; 14] = [
        "gnd", "vss", "vdd", "vcc", "vee", "avdd", "avss", "dvdd", "dvss", "vpwr", "vgnd", "agnd",
        "dgnd", "vnb",
    ];
    SUPPLIES.iter().any(|p| {
        last.strip_prefix(p)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphabetic()))
    })
}

fn is_voltage(s: &Signal) -> bool {
    if norm(&s.name).starts_with("i:") {
        return false;
    }
    let u = s.unit.to_lowercase();
    u.is_empty() || u == "v" || u == "voltage"
}

/// The signals to plot for one file: `save` that the file has, else the measured ones,
/// else the first voltages. Names as the viewer lists them.
pub fn pick_default(signals: &[Signal], req: &ShowRequest) -> Vec<String> {
    for wanted in [&req.save, &req.measured] {
        let hits: Vec<String> = wanted
            .iter()
            .filter_map(|w| {
                let key = norm(w);
                signals
                    .iter()
                    .find(|s| !is_axis(s) && norm(&s.name) == key)
                    .map(|s| s.name.clone())
            })
            .take(MAX_DEFAULT)
            .collect();
        if !hits.is_empty() {
            return dedup(hits);
        }
    }
    let usable: Vec<&Signal> = signals
        .iter()
        .filter(|s| !is_axis(s) && !is_ground_or_supply(&s.name))
        .collect();
    let volts = dedup(
        usable
            .iter()
            .filter(|s| is_voltage(s))
            .map(|s| s.name.clone())
            .collect(),
    );
    if !volts.is_empty() {
        return volts.into_iter().take(MAX_FALLBACK).collect();
    }
    dedup(usable.iter().map(|s| s.name.clone()).collect())
        .into_iter()
        .take(MAX_FALLBACK)
        .collect()
}

/// The signals of a file that `names` (`v(out)` or `out`) denote; the ones it does not
/// have are returned apart.
pub fn pick_named(signals: &[Signal], names: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut found, mut missing) = (Vec::new(), Vec::new());
    for n in names {
        let key = norm(n);
        match signals.iter().find(|s| norm(&s.name) == key) {
            Some(s) => found.push(s.name.clone()),
            None => missing.push(n.clone()),
        }
    }
    (dedup(found), missing)
}

fn dedup(mut v: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    v.retain(|n| {
        let fresh = !seen.contains(n);
        seen.push(n.clone());
        fresh
    });
    v
}

/// What [`apply`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// `tag:name` of what was shown.
    pub shown: Vec<String>,
    /// Things the user should know (a name a file does not have, a refusal).
    pub notes: Vec<String>,
}

/// The signals of one file, waiting for the viewer to have loaded it (an empty list
/// until then). `None` when `deadline` came first.
fn wait_signals(client: &mut dyn ViewerOps, tag: &str, deadline: Instant) -> Option<Vec<Signal>> {
    loop {
        if let Ok((s, _)) = client.list_signals(Some(tag), None)
            && !s.is_empty()
        {
            return Some(s);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Send `show` for every file: first into a new pane, the others into the pane the
/// viewer answered with. A viewer that has not loaded a file within `wait` is left alone
/// for it. Failures are notes, never errors: the files are open anyway.
pub fn apply(
    client: &mut dyn ViewerOps,
    files: &[ViewFile],
    req: &ShowRequest,
    wait: Duration,
) -> Applied {
    let mut done = Applied::default();
    if !req.enabled() {
        return done;
    }
    let deadline = Instant::now() + wait;
    let mut pane = PaneSel::New;
    for f in files {
        let Some(signals) = wait_signals(client, &f.tag, deadline) else {
            done.notes.push(format!(
                "'{}' was not loaded in the viewer within {:.0} s: nothing shown for it",
                f.tag,
                wait.as_secs_f64()
            ));
            continue;
        };
        let names = match &req.show {
            Show::Names(n) => {
                let (found, missing) = pick_named(&signals, n);
                if !missing.is_empty() {
                    done.notes
                        .push(format!("'{}' has no signal {}", f.tag, missing.join(", ")));
                }
                found
            }
            _ => pick_default(&signals, req),
        };
        if names.is_empty() {
            continue;
        }
        let qualified: Vec<String> = names.iter().map(|n| format!("{}:{n}", f.tag)).collect();
        match client.show(&qualified, pane) {
            Ok(p) => {
                if let Some(p) = p {
                    pane = PaneSel::Index(p.min(u32::MAX as u64) as u32);
                }
                done.shown.extend(qualified);
            }
            Err(e) => done.notes.push(format!("show for '{}': {e:#}", f.tag)),
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(name: &str, unit: &str) -> Signal {
        Signal {
            name: name.into(),
            file: "t".into(),
            unit: unit.into(),
            complex: false,
        }
    }

    fn ring() -> Vec<Signal> {
        vec![
            sig("time", "s"),
            sig("v(vdd)", "V"),
            sig("v(in)", "V"),
            sig("v(out)", "V"),
            sig("v(mid)", "V"),
            sig("v(a)", "V"),
            sig("v(b)", "V"),
            sig("i(vdd)", "A"),
        ]
    }

    fn req(save: &[&str], measured: &[&str]) -> ShowRequest {
        ShowRequest {
            show: Show::Default,
            save: save.iter().map(|s| s.to_string()).collect(),
            measured: measured.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn save_then_measured_then_the_first_voltages() {
        let s = ring();
        assert_eq!(
            pick_default(&s, &req(&["v(in)", "v(out)", "v(nope)"], &["v(mid)"])),
            ["v(in)", "v(out)"]
        );
        assert_eq!(
            pick_default(&s, &req(&["v(nope)", "v(*)"], &["v(mid)", "i(vdd)"])),
            ["v(mid)", "i(vdd)"]
        );
        // Nothing known: no axis, no supply, no current, at most four.
        assert_eq!(
            pick_default(&s, &req(&[], &[])),
            ["v(in)", "v(out)", "v(mid)", "v(a)"]
        );
        // Another spelling (Xyce) of the same names.
        let xyce = vec![sig("TIME", "s"), sig("OUT", "V"), sig("VDD#branch", "A")];
        assert_eq!(pick_default(&xyce, &req(&["v(out)"], &[])), ["OUT"]);
        assert_eq!(pick_default(&xyce, &req(&["i(vdd)"], &[])), ["VDD#branch"]);
        // Only a supply and a current: the currents are better than nothing.
        let poor = vec![sig("v(0)", "V"), sig("v(vdd)", "V"), sig("i(v1)", "A")];
        assert_eq!(pick_default(&poor, &req(&[], &[])), ["i(v1)"]);
        assert!(pick_default(&[], &req(&[], &[])).is_empty());
    }

    #[test]
    fn named_signals_and_the_missing() {
        let (found, missing) = pick_named(
            &ring(),
            &["out".into(), "V(IN)".into(), "i(vdd)".into(), "zz".into()],
        );
        assert_eq!(found, ["v(out)", "v(in)", "i(vdd)"]);
        assert_eq!(missing, ["zz"]);
    }

    #[test]
    fn signals_of_measures_and_expressions() {
        assert_eq!(
            signals_in("tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1"),
            ["v(in)", "v(out)"]
        );
        assert_eq!(
            signals_in(
                "db20(value_at(v(\"OUT\", analysis=\"ac\") / v('in'), 1k)) + i(vdd) + dv(x)"
            ),
            ["v(out)", "v(in)", "i(vdd)"]
        );
        assert!(signals_in("max(1, 2)").is_empty());
    }

    #[test]
    fn supplies_and_ground_are_recognised() {
        for n in [
            "v(0)",
            "v(vdd)",
            "v(x1.vss)",
            "v(VDD3)",
            "gnd",
            "v(vdd_core)",
        ] {
            assert!(is_ground_or_supply(n), "{n}");
        }
        for n in ["v(out)", "v(vddx)", "v(vref)", "v(in)", "v(gate)"] {
            assert!(!is_ground_or_supply(n), "{n}");
        }
    }
}
