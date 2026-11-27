//! `cdept sim new`: a testbench view from a template, with a starter schematic that
//! instantiates the DUT and the sources the template wants.
//!
//! The schematic is written as xschem text (no engine needed): the DUT symbol at the
//! origin with a labelled wire on each pin, a supply source, an input source whose value
//! is the template's `stimulus`, and a load capacitor on the output. Pins are matched to
//! the template's nets by name; supply pins (`vdd`, `vcc`, ...) go to the supply net,
//! ground pins (`vss`, `gnd`, ...) to `0`, any other pin gets a net of its own name.

use std::collections::BTreeMap;
use std::path::PathBuf;

use eyre::{Result, WrapErr, bail, eyre};

use super::template::{Template, starter_setup};
use crate::Project;
use crate::entry::Format;

/// A pin of a symbol: name, direction (`in`, `out`, `inout`) and position.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub name: String,
    pub dir: String,
    pub x: f64,
    pub y: f64,
}

/// The pins (`B 5 …` boxes with a `name=`) of an xschem symbol.
pub fn symbol_pins(text: &str) -> Vec<Pin> {
    let mut pins = vec![];
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("B 5 ") else {
            continue;
        };
        let Some((coords, props)) = rest.split_once('{') else {
            continue;
        };
        let n: Vec<f64> = coords
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        if n.len() != 4 {
            continue;
        }
        let prop = |key: &str| {
            props
                .trim_end_matches('}')
                .split_whitespace()
                .find_map(|t| t.strip_prefix(&format!("{key}=")))
                .unwrap_or("")
                .to_string()
        };
        let name = prop("name");
        if !name.is_empty() {
            pins.push(Pin {
                name,
                dir: prop("dir"),
                x: (n[0] + n[2]) / 2.0,
                y: (n[1] + n[3]) / 2.0,
            });
        }
    }
    pins
}

fn is_supply(name: &str) -> bool {
    let n = name.to_lowercase();
    ["vdd", "vcc", "avdd", "dvdd", "vpwr", "vdda", "vddd"]
        .iter()
        .any(|p| n == *p || n.starts_with(p))
}

fn is_ground(name: &str) -> bool {
    let n = name.to_lowercase();
    ["vss", "gnd", "agnd", "dgnd", "vgnd", "vssa", "vssd"]
        .iter()
        .any(|p| n == *p || n.starts_with(p))
}

/// Escape braces for an xschem property value (`{vdd}` becomes `\{vdd\}`).
fn xschem_value(v: &str) -> String {
    v.replace('{', "\\{").replace('}', "\\}")
}

/// What the starter schematic is built from.
pub struct StarterSpec<'a> {
    /// `library/cell/view/file.sym` as the schematic references it.
    pub symbol_ref: &'a str,
    pub pins: &'a [Pin],
    pub in_net: &'a str,
    pub out_net: &'a str,
    pub supply_net: &'a str,
    pub supply_source: &'a str,
    pub input_source: &'a str,
    /// The input source's value (SPICE), `{var}` for design variables.
    pub stimulus: &'a str,
}

/// The pin that is the input (named like the net, else the first `in` pin) and the one
/// that is the output (likewise).
fn io_pins<'a>(s: &StarterSpec<'a>) -> (Option<&'a str>, Option<&'a str>) {
    let plain = |p: &&Pin| !is_supply(&p.name) && !is_ground(&p.name);
    let pick = |net: &str, dir: &str| -> Option<&'a str> {
        s.pins
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(net))
            .or_else(|| s.pins.iter().filter(plain).find(|p| p.dir == dir))
            .map(|p| p.name.as_str())
    };
    (pick(s.in_net, "in"), pick(s.out_net, "out"))
}

/// The net a pin of the DUT is wired to.
fn pin_net(s: &StarterSpec, pin: &str) -> String {
    let (i, o) = io_pins(s);
    if is_supply(pin) {
        s.supply_net.to_string()
    } else if is_ground(pin) {
        "0".to_string()
    } else if Some(pin) == i {
        s.in_net.to_string()
    } else if Some(pin) == o {
        s.out_net.to_string()
    } else {
        pin.to_string()
    }
}

/// The xschem text of the starter testbench schematic.
pub fn schematic_text(s: &StarterSpec) -> String {
    let mut t =
        String::from("v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {}\nV {}\nS {}\nE {}\n");
    t.push_str(&format!("C {{{}}} 0 0 0 0 {{name=x1}}\n", s.symbol_ref));
    let (mut cx, mut cy) = (0.0, 0.0);
    for p in s.pins {
        cx += p.x;
        cy += p.y;
    }
    let n = s.pins.len().max(1) as f64;
    let (cx, cy) = (cx / n, cy / n);
    for (i, p) in s.pins.iter().enumerate() {
        let net = pin_net(s, &p.name);
        // The wire leaves the pin away from the symbol's middle.
        let (dx, dy) = if (p.x - cx).abs() >= (p.y - cy).abs() {
            (if p.x >= cx { 60.0 } else { -60.0 }, 0.0)
        } else {
            (0.0, if p.y >= cy { 60.0 } else { -60.0 })
        };
        t.push_str(&format!(
            "N {} {} {} {} {{lab={net}}}\n",
            p.x,
            p.y,
            p.x + dx,
            p.y + dy
        ));
        if net == "0" {
            t.push_str(&format!(
                "C {{gnd.sym}} {} {} 0 0 {{name=lg{i} lab=0}}\n",
                p.x + dx,
                p.y + dy
            ));
        } else if net.eq_ignore_ascii_case(s.supply_net) {
            t.push_str(&format!(
                "C {{vdd.sym}} {} {} 0 0 {{name=lv{i} lab={net}}}\n",
                p.x + dx,
                p.y + dy
            ));
        } else {
            t.push_str(&format!(
                "C {{lab_wire.sym}} {} {} 0 0 {{name=ln{i} lab={net}}}\n",
                p.x + dx,
                p.y + dy
            ));
        }
    }
    // Sources and the load, left of the DUT, in the layout of cells/tb_inv.
    let (mut x, y) = (-400.0, 30.0 + 200.0);
    t.push_str(&format!(
        "C {{vsource.sym}} {x} {y} 0 0 {{name={} value=\"\\{{vdd\\}}\" savecurrent=false}}\n",
        s.supply_source
    ));
    t.push_str(&format!(
        "C {{vdd.sym}} {x} {} 0 0 {{name=lsv lab={}}}\n",
        y - 30.0,
        s.supply_net
    ));
    t.push_str(&format!(
        "C {{gnd.sym}} {x} {} 0 0 {{name=lsg lab=0}}\n",
        y + 30.0
    ));
    x += 120.0;
    t.push_str(&format!(
        "C {{vsource.sym}} {x} {y} 0 0 {{name={} value=\"{}\" savecurrent=false}}\n",
        s.input_source,
        xschem_value(s.stimulus)
    ));
    t.push_str(&format!(
        "C {{lab_wire.sym}} {x} {} 0 0 {{name=lsi lab={}}}\n",
        y - 30.0,
        s.in_net
    ));
    t.push_str(&format!(
        "C {{gnd.sym}} {x} {} 0 0 {{name=lig lab=0}}\n",
        y + 30.0
    ));
    x += 120.0;
    t.push_str(&format!(
        "C {{capa.sym}} {x} {y} 0 0 {{name=CL value=10f m=1}}\n"
    ));
    t.push_str(&format!(
        "C {{lab_wire.sym}} {x} {} 0 0 {{name=lso lab={}}}\n",
        y - 30.0,
        s.out_net
    ));
    t.push_str(&format!(
        "C {{gnd.sym}} {x} {} 0 0 {{name=lcg lab=0}}\n",
        y + 30.0
    ));
    t
}

/// What [`create_testbench`] made.
#[derive(Debug, Clone, Default)]
pub struct Created {
    pub setup: PathBuf,
    /// The starter schematic, when one was written.
    pub schematic: Option<PathBuf>,
    pub notes: Vec<String>,
}

fn param_or(bound: &super::template::Bound, name: &str, default: &str) -> String {
    bound
        .get(name)
        .cloned()
        .flatten()
        .unwrap_or_else(|| default.to_string())
}

/// Create the testbench view `library/cell/view` from `template`. With `dut`
/// (`library/cell`) the cell also gets a starter schematic that instantiates the DUT
/// (its symbol must exist), unless it has a schematic already: then that one is used.
pub fn create_testbench(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    template: &Template,
    args: &BTreeMap<String, String>,
    dut: Option<&str>,
) -> Result<Created> {
    create_testbench_using(project, library, cell, view, template, args, dut, None)
}

/// [`create_testbench`] for a testbench that netlists another cell's schematic:
/// `schematic` (`library/cell`, `library/cell/view`) goes into `setup.toml`. It
/// excludes `dut`, which writes a schematic of the cell's own.
#[allow(clippy::too_many_arguments)]
pub fn create_testbench_using(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    template: &Template,
    args: &BTreeMap<String, String>,
    dut: Option<&str>,
    schematic: Option<&str>,
) -> Result<Created> {
    if dut.is_some() && schematic.is_some() {
        bail!("a device under test and another cell's schematic exclude each other");
    }
    crate::library::validate_name("cell", cell)?;
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    if lib.read_only {
        bail!("library '{library}' is read-only");
    }
    if lib.layout != crate::library::LibraryLayout::Cells {
        bail!("library '{library}' is not a cells library (views live in <cell>/<view>/)");
    }
    if !view
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        || view.is_empty()
    {
        bail!("'{view}' is not a valid view name");
    }
    let view_dir = lib.path.join(cell).join(view);
    if view_dir.exists() {
        bail!("{library}/{cell} already has a view '{view}'");
    }
    let bound = template.bind(args)?;
    let mut created = Created::default();
    let has_schematic = lib.cell(cell).is_some_and(|c| {
        c.entries
            .iter()
            .any(|e| e.format == Some(Format::XschemSchematic))
    });
    if let Some(dut) = dut {
        let (dl, dc) = dut
            .split_once('/')
            .ok_or_else(|| eyre!("--dut is library/cell, not '{dut}'"))?;
        if has_schematic {
            created.notes.push(format!(
                "{library}/{cell} already has a schematic; --dut ignored"
            ));
        } else {
            let dcell = project
                .library(dl)
                .and_then(|l| l.cell(dc))
                .ok_or_else(|| eyre!("no cell {dut} for the DUT"))?;
            let sym = dcell
                .entries
                .iter()
                .find(|e| e.format == Some(Format::XschemSymbol))
                .ok_or_else(|| eyre!("{dut} has no xschem symbol to instantiate"))?;
            let file = sym
                .main_file
                .clone()
                .ok_or_else(|| eyre!("{dut}: the symbol view has no file"))?;
            let text = std::fs::read_to_string(&file)
                .wrap_err_with(|| format!("cannot read {}", file.display()))?;
            let pins = symbol_pins(&text);
            if pins.is_empty() {
                bail!("{dut}: the symbol {} has no pins", file.display());
            }
            let symbol_ref = format!(
                "{dl}/{dc}/{}/{}",
                sym.name,
                file.file_name().and_then(|n| n.to_str()).unwrap_or("")
            );
            let in_net = param_or(&bound, "in", "in");
            let out_net = param_or(&bound, "out", "out");
            let supply_source = param_or(&bound, "supply", "Vdd");
            let input_source = param_or(&bound, "src", "Vin");
            let stimulus = if template.stimulus.is_empty() {
                "dc 0"
            } else {
                template.stimulus.as_str()
            };
            let spec = StarterSpec {
                symbol_ref: &symbol_ref,
                pins: &pins,
                in_net: &in_net,
                out_net: &out_net,
                supply_net: "VDD",
                supply_source: &supply_source,
                input_source: &input_source,
                stimulus,
            };
            let dir = lib.path.join(cell).join("schematic");
            std::fs::create_dir_all(&dir)?;
            let sch = dir.join(format!("{cell}.sch"));
            if sch.exists() {
                bail!("{} exists already", sch.display());
            }
            std::fs::write(&sch, schematic_text(&spec))?;
            let others: Vec<String> = pins
                .iter()
                .filter(|p| {
                    let n = pin_net(&spec, &p.name);
                    n.eq_ignore_ascii_case(&p.name)
                        && n != in_net
                        && n != out_net
                        && !is_supply(&n)
                        && !is_ground(&n)
                })
                .map(|p| p.name.clone())
                .collect();
            if !others.is_empty() {
                created.notes.push(format!(
                    "pins {} of the DUT are neither input, output nor supply: they got nets of their own name; edit the schematic to drive them",
                    others.join(", ")
                ));
            }
            let (i, o) = io_pins(&spec);
            created.notes.push(format!(
                "DUT pins: input {}, output {}, supply on {}",
                i.unwrap_or("(none found)"),
                o.unwrap_or("(none found)"),
                pins.iter()
                    .filter(|p| is_supply(&p.name))
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            created.schematic = Some(sch);
        }
    } else if !has_schematic && schematic.is_none() {
        created.notes.push(format!(
            "{library}/{cell} has no xschem schematic: draw one, or name another with `schematic = \"lib/cell\"` in setup.toml (or use --dut)"
        ));
    }
    let mut text = starter_setup(template, args, schematic.unwrap_or(""))?;
    let mut vars = template.starter_variables.clone();
    if !vars.contains_key("vdd") {
        vars.insert("vdd".into(), "1.2".into());
    }
    // Variables the template sets itself (vdd) come from its parameters already.
    if let Some(toml::Value::Table(t)) = template.file.setup.get("variables") {
        for k in t.keys() {
            vars.remove(k);
        }
    }
    if !vars.is_empty() {
        text.push_str("\n# design variables the schematic's sources use as {name}\n[variables]\n");
        for (k, v) in &vars {
            text.push_str(&format!("{k} = {}\n", toml::Value::String(v.clone())));
        }
    }
    std::fs::create_dir_all(&view_dir)?;
    let setup = view_dir.join(super::setup::SETUP_FILE);
    std::fs::write(&setup, &text)?;
    // The result must load.
    super::Setup::parse_with(&text, &super::template::TemplateSet::for_project(project))
        .wrap_err("the new setup.toml does not load")?;
    created.setup = setup;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYM: &str = "v {xschem version=3.4.5 file_version=1.2\n}\nK {type=subcircuit}\nB 5 -42.5 -2.5 -37.5 2.5 {name=A dir=in}\nB 5 37.5 -2.5 42.5 2.5 {name=Y dir=out}\nB 5 -2.5 -42.5 2.5 -37.5 {name=VDD dir=inout}\nB 5 -2.5 37.5 2.5 42.5 {name=VSS dir=inout}\n";

    #[test]
    fn reads_pins() {
        let pins = symbol_pins(SYM);
        assert_eq!(pins.len(), 4);
        assert_eq!(
            (
                pins[0].name.as_str(),
                pins[0].dir.as_str(),
                pins[0].x,
                pins[0].y
            ),
            ("A", "in", -40.0, 0.0)
        );
        assert_eq!(pins[2].y, -40.0);
    }

    #[test]
    fn starter_schematic_labels_the_pins() {
        let pins = symbol_pins(SYM);
        let t = schematic_text(&StarterSpec {
            symbol_ref: "cells/inv/symbol/inv.sym",
            pins: &pins,
            in_net: "A",
            out_net: "Y",
            supply_net: "VDD",
            supply_source: "Vdd",
            input_source: "Vin",
            stimulus: "pulse(0 {vdd} 200p 50p 50p 400p 1n)",
        });
        assert!(
            t.contains("C {cells/inv/symbol/inv.sym} 0 0 0 0 {name=x1}"),
            "{t}"
        );
        assert!(
            t.contains("{lab=VDD}")
                && t.contains("{lab=0}")
                && t.contains("{lab=A}")
                && t.contains("{lab=Y}"),
            "{t}"
        );
        assert!(t.contains("value=\"pulse(0 \\{vdd\\} 200p"), "{t}");
    }
}
