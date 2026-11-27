//! What a tutorial wants to know about the schematic xschem shows: its instances with
//! their attributes and the nets on their pins, asked now and then while a tutorial
//! checks a circuit.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use libcdept::link::schematic::attributes;
use libcdept::xschem::{Status, tcl};

use super::SchematicView;
use crate::guide::Instance;

const EVERY: Duration = Duration::from_millis(700);

/// `{schematic {{name symbol attributes {pin net …} x y} …}}`. The place of an
/// instance is the middle of its box in frame pixels.
const SCRIPT: &str = r#"
set cdept_list {}
set cdept_n [xschem get instances]
set cdept_z [xschem get zoom]
for {set i 0} {$i < $cdept_n} {incr i} {
  set cdept_name [xschem getprop instance $i name]
  set cdept_map [xschem instance_nodemap $cdept_name]
  set cdept_x -1
  set cdept_y -1
  if {[regexp {Instance: (\S+) (\S+) (\S+) (\S+)} [xschem instance_bbox $cdept_name] -> x1 y1 x2 y2]} {
    set cdept_x [expr {(($x1 + $x2) / 2.0 + [xschem get xorigin]) / $cdept_z}]
    set cdept_y [expr {(($y1 + $y2) / 2.0 + [xschem get yorigin]) / $cdept_z}]
  }
  lappend cdept_list [list $cdept_name [xschem getprop instance $cdept_name cell::name] \
      [xschem getprop instance $cdept_name] [lrange $cdept_map 1 end] $cdept_x $cdept_y]
}
list [file normalize [xschem get schname]] $cdept_list
"#;

/// The answer to the last probe.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    /// The schematic xschem showed (it may have descended into another).
    pub file: PathBuf,
    pub instances: Vec<Instance>,
    /// Name and place in frame pixels.
    pub places: Vec<(String, f32, f32)>,
}

#[derive(Default)]
pub(super) struct Probing {
    asked: Option<u64>,
    at: Option<Instant>,
    last: Option<Probe>,
}

fn read(text: &str) -> Result<Probe, String> {
    let list = tcl::parse_list;
    let [file, instances] =
        <[String; 2]>::try_from(list(text)?).map_err(|_| "not a probe".to_string())?;
    let mut probe = Probe {
        file: file.into(),
        ..Probe::default()
    };
    for item in list(&instances)? {
        let [name, symbol, props, pins, x, y] =
            <[String; 6]>::try_from(list(&item)?).map_err(|_| "not an instance".to_string())?;
        if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
            probe.places.push((name.clone(), x, y));
        }
        probe.instances.push(Instance {
            name,
            symbol,
            attributes: attributes(&props),
            pins: list(&pins)?
                .chunks(2)
                .filter(|p| p.len() == 2)
                .map(|p| (p[0].clone(), p[1].clone()))
                .collect(),
        });
    }
    Ok(probe)
}

impl SchematicView {
    /// Ask xschem again when it is time, and give the latest answer. Not while xschem
    /// waits for a dialog or a mouse button is down.
    pub(crate) fn probe(&mut self) -> Option<&Probe> {
        if let Some(id) = self.probing.asked
            && let Some(reply) = self.take_reply(id)
        {
            self.probing.asked = None;
            match reply.and_then(|text| read(&text)) {
                Ok(probe) => self.probing.last = Some(probe),
                Err(error) => tracing::debug!(target: "cdept::guide", "probe: {error}"),
            }
        }
        let due = self.probing.at.is_none_or(|at| at.elapsed() >= EVERY);
        let quiet = self.session.status() == Status::Running
            && !self.mirror.has_modal()
            && self.buttons == 0;
        if self.probing.asked.is_none() && due && quiet {
            self.probing.asked = Some(self.session.tcl(SCRIPT));
            self.probing.at = Some(Instant::now());
        }
        self.probing.last.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_is_read() {
        let text =
            "/p/inv.sch {{M1 sg13g2_pr/sg13_lv_nmos.sym {name=M1\nw=0.74u} {D Y G A} 10.5 20}}";
        let probe = read(text).unwrap();
        assert_eq!(probe.instances[0].attributes["w"], "0.74u");
        assert_eq!(probe.instances[0].pins["G"], "A");
        assert_eq!(probe.places, [("M1".to_string(), 10.5, 20.0)]);
        assert!(read("x").is_err());
    }
}
