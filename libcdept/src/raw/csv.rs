//! CSV (and TSV, `;`-separated, whitespace-separated) waveform tables: the first column
//! is x (time, frequency, a swept value), every other column a signal.
//!
//! ```text
//! time,v(out),i(vdd)
//! 0,0,1e-6
//! 1e-9,0.1,1.2e-6
//! ```
//!
//! The first line that is not a comment (`#`, `%`, `*`, `;`) names the columns when any
//! of its fields is not a number; otherwise the columns are `x`, `col1`, `col2`, ...
//! Numbers may carry SPICE suffixes (`1n`, `2.5k`). An empty cell, `nan` or text is a gap
//! (NaN). The x column must be finite and must not decrease. Quoted fields (`"a,b"`)
//! are taken as they are.

use eyre::{Result, bail};

use super::{Layout, Plot, Var};
use crate::calc::parse_spice_number;

/// The most columns a table may have (a guard against a binary file read as text).
const MAX_COLUMNS: usize = 100_000;

fn delimiter(line: &str) -> Option<char> {
    if line.contains('\t') {
        Some('\t')
    } else if line.contains(',') {
        Some(',')
    } else if line.contains(';') {
        Some(';')
    } else {
        None // whitespace
    }
}

fn fields(line: &str, delim: Option<char>) -> Vec<String> {
    let Some(d) = delim else {
        return line.split_whitespace().map(String::from).collect();
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c == d && !quoted {
            out.push(std::mem::take(&mut cur).trim().to_string());
        } else {
            cur.push(c);
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().or_else(|| parse_spice_number(t))
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty()
        || t.starts_with('#')
        || t.starts_with('%')
        || t.starts_with('*')
        || t.starts_with("//")
}

/// The plot name a table gets from its x column: `time` -> a transient, `freq...` -> AC.
fn plot_name(x: &str) -> &'static str {
    let x = x.to_lowercase();
    if x == "t" || x.contains("time") {
        "Transient Analysis (CSV)"
    } else if x.starts_with("f") && x.contains("freq") || x == "f" || x == "hz" {
        "AC Analysis (CSV)"
    } else {
        "DC transfer characteristic (CSV)"
    }
}

/// Parse a table. Returns one plot with the values in the ascii layout of raw files.
pub(super) fn parse(data: &[u8]) -> Result<Vec<Plot>> {
    let text = match std::str::from_utf8(data) {
        Ok(t) => t,
        Err(e) => {
            return Err(eyre::eyre!(
                "not a text table: invalid UTF-8 at byte {}",
                e.valid_up_to()
            ));
        }
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().enumerate().filter(|(_, l)| !is_comment(l));
    let Some((first_no, first)) = lines.next() else {
        bail!("the table is empty");
    };
    let delim = delimiter(first);
    let head = fields(first, delim);
    if head.len() < 2 {
        bail!(
            "line {}: a table needs at least two columns (x and a signal)",
            first_no + 1
        );
    }
    if head.len() > MAX_COLUMNS {
        bail!(
            "line {}: {} columns (at most {MAX_COLUMNS})",
            first_no + 1,
            head.len()
        );
    }
    let named = head
        .iter()
        .any(|f| number(f).is_none() && !f.eq_ignore_ascii_case("nan"));
    let names: Vec<String> = if named {
        let mut seen = std::collections::HashSet::new();
        head.iter()
            .enumerate()
            .map(|(i, n)| {
                let base = if n.is_empty() {
                    format!("col{i}")
                } else {
                    n.clone()
                };
                let mut name = base.clone();
                let mut k = 2;
                while !seen.insert(name.to_lowercase()) {
                    name = format!("{base}#{k}");
                    k += 1;
                }
                name
            })
            .collect()
    } else {
        std::iter::once("x".to_string())
            .chain((1..head.len()).map(|i| format!("col{i}")))
            .collect()
    };
    let nv = names.len();
    let mut rows: Vec<f64> = Vec::new();
    let mut points = 0usize;
    let mut last_x = f64::NEG_INFINITY;
    let mut push_row = |no: usize, cells: Vec<String>| -> Result<()> {
        if cells.len() > nv {
            bail!(
                "line {}: {} fields, the header has {nv}",
                no + 1,
                cells.len()
            );
        }
        let x = cells.first().and_then(|c| number(c));
        let Some(x) = x.filter(|x| x.is_finite()) else {
            bail!(
                "line {}: the x value '{}' is not a finite number",
                no + 1,
                cells.first().map_or("", String::as_str)
            );
        };
        if x < last_x {
            bail!(
                "line {}: x goes back from {last_x} to {x} (the x column must not decrease)",
                no + 1
            );
        }
        last_x = x;
        for i in 0..nv {
            let v = cells.get(i).and_then(|c| number(c)).unwrap_or(f64::NAN);
            // infinities are gaps too: no drawing or calculation can use them
            rows.push(if v.is_finite() { v } else { f64::NAN });
            rows.push(0.0);
        }
        points += 1;
        Ok(())
    };
    if !named {
        push_row(first_no, head)?;
    }
    for (no, line) in lines {
        push_row(no, fields(line, delim))?;
    }
    if points == 0 {
        bail!("the table has a header but no rows");
    }
    let vars = names
        .iter()
        .enumerate()
        .map(|(i, n)| Var {
            name: n.clone(),
            unit: if i == 0 {
                n.to_lowercase()
            } else {
                String::new()
            },
        })
        .collect();
    Ok(vec![Plot {
        title: "CSV table".into(),
        date: String::new(),
        name: plot_name(&names[0]).into(),
        complex: false,
        vars,
        points,
        declared_points: points,
        layout: Layout::Ascii(rows),
    }])
}

#[cfg(test)]
mod tests {
    use super::super::RawFile;
    use super::*;

    fn table(text: &str) -> Result<RawFile> {
        RawFile::from_table(text.as_bytes().to_vec())
    }

    #[test]
    fn reads_tables_of_every_kind() {
        let raw =
            table("# made by hand\ntime,v(out),i(vdd)\n0,0,1u\n1n,0.5,\n2n,1,3e-6\n").unwrap();
        let p = &raw.plots()[0];
        assert_eq!(p.kind(), "tran");
        assert_eq!(p.points, 3);
        let w = raw.vector(0, "v(out)").unwrap();
        assert_eq!(*w.x, vec![0.0, 1e-9, 2e-9]);
        assert_eq!(w.y.re(), vec![0.0, 0.5, 1.0]);
        let i = raw.vector(0, "I(VDD)").unwrap().y.re();
        assert!(i[1].is_nan() && (i[2] - 3e-6).abs() < 1e-18);
        // tabs, semicolons, spaces, no header, quotes, a BOM
        assert_eq!(
            table("f\tgain\n1\t2\n10\t3\n").unwrap().plots()[0].kind(),
            "ac"
        );
        assert_eq!(
            table("x;y\n1;2\n").unwrap().vector(0, "y").unwrap().y.re(),
            vec![2.0]
        );
        let raw = table("1 2 3\n2 4 6\n").unwrap();
        assert_eq!(raw.plots()[0].vars[2].name, "col2");
        assert_eq!(
            table("\u{feff}\"a,b\",c\n1,2\n").unwrap().plots()[0].vars[0].name,
            "a,b"
        );
        // duplicate names get #2
        assert_eq!(
            table("t,v,v\n0,1,2\n").unwrap().plots()[0].vars[2].name,
            "v#2"
        );
    }

    #[test]
    fn refuses_or_survives_bad_tables() {
        for bad in [
            "",
            "# only a comment\n",
            "time\n0\n1\n",
            "time,v\n",
            "time,v\n0,1\n2,3,4\n",
            "time,v\n1,0\n0,1\n",
            "time,v\nnan,1\n",
            "time,v\ninf,1\n",
            "time,v\nabc,1\n",
        ] {
            assert!(table(bad).is_err(), "{bad:?}");
        }
        assert!(RawFile::from_table(vec![0xff, 0xfe, 0x00, 0x41]).is_err());
        // NaN, infinities and text in a signal column are gaps
        let raw = table("t,v\n0,nan\n1,inf\n2,-inf\n3,x\n4,5\n").unwrap();
        let y = raw.vector(0, "v").unwrap().y.re();
        assert!(y[..4].iter().all(|v| v.is_nan()) && y[4] == 5.0);
        // a short row is padded with gaps; equal x values are allowed
        let raw = table("t,a,b\n0,1\n0,2,3\n").unwrap();
        assert!(raw.vector(0, "b").unwrap().y.re()[0].is_nan());
        // unicode names
        let raw = table("zeit,µV→out,電圧\n0,1,2\n").unwrap();
        assert_eq!(raw.vector(0, "µv→OUT").unwrap().y.re(), vec![1.0]);
        // noise never panics
        let mut seed = 7u64;
        for _ in 0..300 {
            let len = (seed % 300) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    b"0123456789,;\t \n.-+enaxE\"#k"[(seed >> 40) as usize % 26]
                })
                .collect();
            let _ = RawFile::from_table(bytes);
        }
    }
}
