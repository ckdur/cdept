//! Checking a schematic a user drew against what a tutorial asks for, however it was
//! drawn: parts (instances) with attribute values and the nets on their pins.

use std::collections::BTreeMap;

use serde::Deserialize;

/// What a tutorial asks of one schematic: `[[circuit]]` of a tutorial file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// The name a step asks for: `circuit:<id>`.
    pub id: String,
    /// The schematic, `library/cell/view`.
    pub view: String,
    #[serde(rename = "part")]
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// How the checklist calls it: "the PMOS".
    pub what: String,
    /// The symbol's file name without `.sym`: `sg13_lv_pmos`, `ipin`, `vsource`.
    pub symbol: String,
    /// Attribute values. Numbers are compared as SPICE numbers (`1.5u` = `1.5e-6`),
    /// other values without regard to case and spacing.
    #[serde(default)]
    pub with: BTreeMap<String, String>,
    /// The net of each pin. `?name`: any net, the same one wherever `?name` stands.
    /// `0|GND`: either of the two.
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    /// Two pins that may be the other way round (drain and source of a MOSFET).
    #[serde(default)]
    pub swap: Option<[String; 2]>,
}

/// An instance of the schematic as xschem reports it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Instance {
    pub name: String,
    pub symbol: String,
    pub attributes: BTreeMap<String, String>,
    /// Pin name and net.
    pub pins: BTreeMap<String, String>,
}

/// One line of the checklist.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub ok: bool,
    pub text: String,
}

/// What a SPICE number is worth: `1.5u`, `20uA`, `100f`, `10meg`, `1e-6`.
pub fn spice_number(text: &str) -> Option<f64> {
    let text = text.trim().to_ascii_lowercase();
    let digits = text
        .char_indices()
        .take_while(|&(at, c)| {
            c.is_ascii_digit()
                || c == '.'
                || (at == 0 && (c == '-' || c == '+'))
                || (c == 'e' && at > 0 && {
                    let rest = text[at + 1..].trim_start_matches(['-', '+']);
                    rest.starts_with(|c: char| c.is_ascii_digit())
                })
                || ((c == '-' || c == '+') && text[..at].ends_with('e'))
        })
        .count();
    let value: f64 = text[..digits].parse().ok()?;
    let rest = &text[digits..];
    if !rest.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let scale = if rest.starts_with("meg") {
        1e6
    } else {
        match rest.chars().next() {
            Some('t') => 1e12,
            Some('g') => 1e9,
            Some('k') => 1e3,
            Some('m') => 1e-3,
            Some('u') => 1e-6,
            Some('n') => 1e-9,
            Some('p') => 1e-12,
            Some('f') => 1e-15,
            // A unit without a prefix (`v`, `s`, `h`).
            _ => 1.0,
        }
    };
    Some(value * scale)
}

fn squeeze(text: &str) -> String {
    let text = text.trim().trim_matches('"');
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn same_value(wanted: &str, found: &str) -> bool {
    match (spice_number(wanted), spice_number(found)) {
        (Some(a), Some(b)) => (a - b).abs() <= a.abs().max(b.abs()) * 1e-9,
        _ => squeeze(wanted) == squeeze(found),
    }
}

fn stem(symbol: &str) -> &str {
    let file = symbol.rsplit('/').next().unwrap_or(symbol);
    file.strip_suffix(".sym").unwrap_or(file)
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

type Names = BTreeMap<String, String>;

/// How a net reads to a person: xschem's `#net3` is a net without a name.
fn shown(net: &str) -> String {
    if net.starts_with('#') || net.starts_with("net") {
        "a wire without a name".into()
    } else if net.is_empty() {
        "nothing".into()
    } else {
        net.to_string()
    }
}

/// The pins of `part` against `instance`, binding `?names` as they come.
fn pin_lines(part: &Part, instance: &Instance, swapped: bool, names: &mut Names) -> Vec<Line> {
    let mut lines = Vec::new();
    for (pin, wanted) in &part.pins {
        let actual_pin = match &part.swap {
            Some([a, b]) if swapped && pin == a => b,
            Some([a, b]) if swapped && pin == b => a,
            _ => pin,
        };
        let net = instance
            .pins
            .iter()
            .find(|(p, _)| same_name(p, actual_pin))
            .map(|(_, net)| net.as_str())
            .unwrap_or("");
        let what = &part.what;
        let line = if let Some(variable) = wanted.strip_prefix('?') {
            match names.get(variable) {
                Some(bound) => Line {
                    ok: same_name(bound, net),
                    text: format!("{what}: pin {pin} is on the {variable} net"),
                },
                None => {
                    // Two names must be two nets.
                    let taken = names.values().any(|n| same_name(n, net));
                    let ok = !net.is_empty() && !taken;
                    if ok {
                        names.insert(variable.to_string(), net.to_string());
                    }
                    Line {
                        ok,
                        text: format!("{what}: pin {pin} is on the {variable} net"),
                    }
                }
            }
        } else if wanted.split('|').any(|w| same_name(w, net)) {
            Line {
                ok: true,
                text: format!("{what}: pin {pin} on {net}"),
            }
        } else {
            let wanted = wanted.split('|').next().unwrap_or(wanted);
            Line {
                ok: false,
                text: format!(
                    "{what}: pin {pin} should be on {wanted}, is on {}",
                    shown(net)
                ),
            }
        };
        lines.push(line);
    }
    lines
}

fn value_lines(part: &Part, instance: &Instance) -> Vec<Line> {
    part.with
        .iter()
        .map(|(key, wanted)| {
            let found = instance.attributes.get(key).map_or("", String::as_str);
            let ok = same_value(wanted, found);
            let text = if ok {
                format!("{}: {key} = {wanted}", part.what)
            } else if found.is_empty() {
                format!("{}: set {key} = {wanted}", part.what)
            } else {
                format!("{}: {key} should be {wanted}, is {found}", part.what)
            };
            Line { ok, text }
        })
        .collect()
}

#[derive(Clone)]
struct Trial {
    score: usize,
    lines: Vec<Line>,
}

/// Steps the search may take: circuits of a tutorial are small, but many like devices
/// make many orders.
const BUDGET: usize = 20_000;

struct Search<'a> {
    spec: &'a Spec,
    instances: &'a [Instance],
    used: Vec<bool>,
    best: Option<Trial>,
    steps: usize,
}

impl Search<'_> {
    /// Items still to be had from part `from` on, at most.
    fn possible(&self, from: usize) -> usize {
        self.spec.parts[from..]
            .iter()
            .map(|p| 1 + p.with.len() + p.pins.len())
            .sum()
    }

    fn go(&mut self, at: usize, names: &Names, so_far: &Trial) {
        self.steps += 1;
        if let Some(best) = &self.best
            && (self.steps > BUDGET || so_far.score + self.possible(at) <= best.score)
        {
            return;
        }
        let Some(part) = self.spec.parts.get(at) else {
            self.best = Some(so_far.clone());
            return;
        };
        // The candidates, the best fitting first: the bound then cuts early.
        let mut candidates: Vec<(usize, Names, Vec<Line>)> = Vec::new();
        for (n, instance) in self.instances.iter().enumerate() {
            if self.used[n] || !same_name(stem(&instance.symbol), &part.symbol) {
                continue;
            }
            let orders: &[bool] = if part.swap.is_some() {
                &[false, true]
            } else {
                &[false]
            };
            for &swapped in orders {
                let mut bound = names.clone();
                let mut lines = value_lines(part, instance);
                lines.extend(pin_lines(part, instance, swapped, &mut bound));
                candidates.push((n, bound, lines));
            }
        }
        candidates
            .sort_by_key(|(_, _, lines)| std::cmp::Reverse(lines.iter().filter(|l| l.ok).count()));
        let placed = !candidates.is_empty();
        for (n, bound, lines) in candidates {
            let mut next = so_far.clone();
            next.score += 1 + lines.iter().filter(|l| l.ok).count();
            next.lines.push(Line {
                ok: true,
                text: format!("{} is placed ({})", part.what, part.symbol),
            });
            next.lines.extend(lines);
            self.used[n] = true;
            self.go(at + 1, &bound, &next);
            self.used[n] = false;
        }
        // Without this part: when there is no instance for it, or too few for all.
        if !placed || self.short_of(&part.symbol) {
            let mut next = so_far.clone();
            next.lines.push(Line {
                ok: false,
                text: format!("place {} ({})", part.what, part.symbol),
            });
            self.go(at + 1, names, &next);
        }
    }

    /// Fewer instances of `symbol` than the spec asks for.
    fn short_of(&self, symbol: &str) -> bool {
        let wanted = self
            .spec
            .parts
            .iter()
            .filter(|p| p.symbol == symbol)
            .count();
        let have = self
            .instances
            .iter()
            .filter(|i| same_name(stem(&i.symbol), symbol))
            .count();
        have < wanted
    }
}

/// The checklist of `spec` for a schematic with these instances: the assignment of
/// instances to parts that gets the most right.
pub fn check(spec: &Spec, instances: &[Instance]) -> Vec<Line> {
    let mut search = Search {
        spec,
        instances,
        used: vec![false; instances.len()],
        best: None,
        steps: 0,
    };
    let empty = Trial {
        score: 0,
        lines: Vec::new(),
    };
    search.go(0, &Names::new(), &empty);
    search.best.map(|t| t.lines).unwrap_or_default()
}

pub fn passes(lines: &[Line]) -> bool {
    !lines.is_empty() && lines.iter().all(|l| l.ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(name: &str, symbol: &str, attributes: &str, pins: &str) -> Instance {
        let map = |text: &str| {
            text.split_whitespace()
                .map(|item| {
                    let (k, v) = item.split_once('=').unwrap();
                    (k.to_string(), v.to_string())
                })
                .collect()
        };
        Instance {
            name: name.into(),
            symbol: symbol.into(),
            attributes: map(attributes),
            pins: map(pins),
        }
    }

    fn spec() -> Spec {
        toml::from_str(
            "id='inv'\nview='work/inv/schematic'\n\
             [[part]]\nwhat='the PMOS'\nsymbol='sg13_lv_pmos'\nwith={w='1.5u'}\n\
             pins={G='A', D='Y', S='VDD'}\nswap=['D','S']\n\
             [[part]]\nwhat='the input pin'\nsymbol='ipin'\nwith={lab='A'}\n",
        )
        .unwrap()
    }

    #[test]
    fn spice_numbers() {
        assert!(same_value("1.5u", "1.5e-6"));
        assert!(same_value("0.13u", "130n"));
        assert!(same_value("10meg", "1e7"));
        assert!(same_value("20uA", "20u"));
        assert!(!same_value("1.5u", "1u"));
        assert!(!same_value("1m", "1meg"));
        assert!(same_value("PULSE(0 1.2 0)", "pulse(0  1.2 0)"));
        assert_eq!(spice_number("vdd"), None);
        assert_eq!(spice_number("{cl}"), None);
    }

    #[test]
    fn a_right_circuit_passes_however_source_and_drain_are() {
        let instances = [
            instance("p1", "devices/ipin.sym", "lab=A", "p=A"),
            instance(
                "M7",
                "sg13g2_pr/sg13_lv_pmos.sym",
                "w=1500n",
                "G=A D=VDD S=Y B=VDD",
            ),
        ];
        assert!(passes(&check(&spec(), &instances)));
    }

    #[test]
    fn what_is_wrong_is_said() {
        let instances = [instance(
            "M1",
            "sg13g2_pr/sg13_lv_pmos.sym",
            "w=1u",
            "G=#net2 D=Y S=VDD",
        )];
        let lines = check(&spec(), &instances);
        let wrong: Vec<&str> = lines
            .iter()
            .filter(|l| !l.ok)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            wrong,
            [
                "the PMOS: w should be 1.5u, is 1u",
                "the PMOS: pin G should be on A, is on a wire without a name",
                "place the input pin (ipin)",
            ]
        );
    }

    #[test]
    fn named_nets_bind_across_parts() {
        let spec: Spec = toml::from_str(
            "id='pair'\nview='a/b/c'\n\
             [[part]]\nwhat='left'\nsymbol='n'\npins={S='?tail', D='?mirror'}\n\
             [[part]]\nwhat='right'\nsymbol='n'\npins={S='?tail', D='out'}\n",
        )
        .unwrap();
        let good = [
            instance("a", "n.sym", "", "S=#t D=out"),
            instance("b", "n.sym", "", "S=#t D=#m"),
        ];
        assert!(passes(&check(&spec, &good)));
        let split = [
            instance("a", "n.sym", "", "S=#t D=out"),
            instance("b", "n.sym", "", "S=#u D=#m"),
        ];
        assert!(!passes(&check(&spec, &split)));
        // Source and drain of the left one shorted: two names, one net.
        let short = [
            instance("a", "n.sym", "", "S=#t D=out"),
            instance("b", "n.sym", "", "S=#t D=#t"),
        ];
        assert!(!passes(&check(&spec, &short)));
    }
}
