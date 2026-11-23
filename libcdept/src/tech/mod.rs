//! Technology snapshot: what CDePT reads out of a PDK and keeps in the project.
//!
//! When a project is set up for a PDK, CDePT scans the PDK's files once and writes
//! what it found next to `cdept.toml`:
//!
//! - `tech.toml` ([`Tech`], schema `cdept-tech`): small and hand-editable. Where each
//!   tool's set-up is (KLayout technology, layer properties, DRC and LVS decks,
//!   `xschemrc`, magicrc, netgen set-up), the model files and their corners, the
//!   libraries the PDK brings.
//! - `tech.catalog.json` ([`Catalog`], schema `cdept-tech-catalog`): the bulky lists
//!   behind it (layers, PCells and their parameters, symbols, the cells of the
//!   reference libraries). It can always be made again with a new scan.
//!
//! One module per section, each with a `scan(&ScanContext) -> Scanned<Section,
//! Catalog>`; [`scan`] runs them all. Shared types are in [`pdk`]. Reading and
//! writing the two files is in [`file`]; finding PDKs on the machine in [`discover`].
//! Design and file format: docs/TECH.md.

pub mod ciel;
pub mod devices;
pub mod discover;
pub mod file;
pub mod klayout;
pub mod librelane;
pub mod magic;
pub mod models;
pub mod netgen;
pub mod pdk;
pub mod refs;
pub mod report;
pub mod xschem;

use serde::{Deserialize, Serialize};

pub use pdk::{Family, PdkDir, PdkInfo, ScanContext, Scanned, TechLibrary, TechPath};

/// The snapshot file, next to `cdept.toml`.
pub const TECH_FILE: &str = "tech.toml";
/// The catalog file, next to `cdept.toml`.
pub const CATALOG_FILE: &str = "tech.catalog.json";
pub const TECH_SCHEMA: &str = "cdept-tech";
pub const CATALOG_SCHEMA: &str = "cdept-tech-catalog";
pub const VERSION: u32 = 1;

/// `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tech {
    pub schema: String,
    pub version: u32,
    pub pdk: PdkInfo,
    pub klayout: klayout::KlayoutTech,
    pub magic: magic::MagicTech,
    pub netgen: netgen::NetgenTech,
    pub xschem: xschem::XschemTech,
    pub models: models::ModelsTech,
    pub devices: devices::DevicesTech,
    pub refs: refs::RefsTech,
    /// What the scan could not find or had to guess.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// `tech.catalog.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Catalog {
    pub schema: String,
    pub version: u32,
    /// `$PDK` the catalog was made for.
    pub pdk: String,
    pub klayout: klayout::KlayoutCatalog,
    pub magic: magic::MagicCatalog,
    pub netgen: netgen::NetgenCatalog,
    pub xschem: xschem::XschemCatalog,
    pub models: models::ModelsCatalog,
    pub devices: devices::DevicesCatalog,
    pub refs: refs::RefsCatalog,
}

impl Tech {
    /// Every library the PDK brings: the xschem symbol libraries first, then the
    /// reference libraries (`libs.ref`). Names may repeat between the two; the caller
    /// decides what to do then.
    pub fn libraries(&self) -> Vec<TechLibrary> {
        let mut all = self.xschem.libraries.clone();
        all.extend(self.refs.libraries());
        all
    }

    /// The libraries a project of this PDK lists: those marked `enabled`, each name
    /// once. Of two libraries with one name, the one whose folder holds the other's
    /// is listed (the xschem symbols of the standard cells are a folder inside their
    /// reference library, which has them as views). When neither holds the other (an
    /// xschem symbol library and a `libs.ref` library elsewhere) the first is listed,
    /// which is the xschem one, and the other is in the second list with the one
    /// that took its place: a library made of several folders is not in the model yet.
    pub fn project_libraries(&self) -> (Vec<TechLibrary>, Vec<(TechLibrary, TechLibrary)>) {
        use std::path::Path;
        let mut listed: Vec<TechLibrary> = Vec::new();
        let mut left_out = Vec::new();
        for entry in self.libraries() {
            if !entry.enabled || entry.name.is_empty() {
                continue;
            }
            let Some(first) = listed.iter_mut().find(|e| e.name == entry.name) else {
                listed.push(entry);
                continue;
            };
            let (old, new) = (
                Path::new(first.path.as_str()),
                Path::new(entry.path.as_str()),
            );
            if new.starts_with(old) {
                // Inside the one listed already.
            } else if old.starts_with(new) {
                *first = entry;
            } else {
                left_out.push((first.clone(), entry));
            }
        }
        (listed, left_out)
    }
}

/// The sections a scan reports, in the order they are put together (`devices` last:
/// it joins what the others found).
pub const SCAN_STEPS: [&str; 8] = [
    "pdk", "klayout", "magic", "netgen", "xschem", "models", "refs", "devices",
];

/// Scan a PDK: every section, the independent ones side by side.
pub fn scan(ctx: &ScanContext) -> (Tech, Catalog) {
    scan_with(ctx, &|_, _| {})
}

/// [`scan`], saying when each section starts (`false`) and ends (`true`), by its name
/// in [`SCAN_STEPS`]. `report` is called from the threads the sections run on.
pub fn scan_with(
    ctx: &ScanContext,
    report: &(dyn Fn(&'static str, bool) + Sync),
) -> (Tech, Catalog) {
    // Run one section, reporting its start and end.
    fn run<T>(
        name: &'static str,
        report: &(dyn Fn(&'static str, bool) + Sync),
        work: impl FnOnce() -> T,
    ) -> T {
        report(name, false);
        let value = work();
        report(name, true);
        value
    }
    let (pdk, klayout, magic, netgen, xschem, models, refs) = std::thread::scope(|s| {
        let pdk = s.spawn(|| run("pdk", report, || discover::info(&ctx.pdk)));
        let klayout = s.spawn(|| run("klayout", report, || klayout::scan(ctx)));
        let magic = s.spawn(|| run("magic", report, || magic::scan(ctx)));
        let netgen = s.spawn(|| run("netgen", report, || netgen::scan(ctx)));
        let xschem = s.spawn(|| run("xschem", report, || xschem::scan(ctx)));
        let models = s.spawn(|| run("models", report, || models::scan(ctx)));
        let refs = s.spawn(|| run("refs", report, || refs::scan(ctx)));
        (
            pdk.join().expect("pdk info"),
            klayout.join().expect("klayout scan"),
            magic.join().expect("magic scan"),
            netgen.join().expect("netgen scan"),
            xschem.join().expect("xschem scan"),
            models.join().expect("models scan"),
            refs.join().expect("refs scan"),
        )
    });

    let mut tech = Tech {
        schema: TECH_SCHEMA.into(),
        version: VERSION,
        pdk,
        ..Tech::default()
    };
    let mut catalog = Catalog {
        schema: CATALOG_SCHEMA.into(),
        version: VERSION,
        pdk: ctx.pdk.name.clone(),
        ..Catalog::default()
    };
    macro_rules! take {
        ($field:ident, $scanned:expr) => {{
            let scanned = $scanned;
            tech.$field = scanned.section;
            catalog.$field = scanned.catalog;
            tech.notes.extend(scanned.notes);
        }};
    }
    take!(klayout, klayout);
    take!(magic, magic);
    take!(netgen, netgen);
    take!(xschem, xschem);
    take!(models, models);
    take!(refs, refs);
    // The devices join what the others found (symbols, models, PCells).
    take!(
        devices,
        run("devices", report, || devices::scan(ctx, &tech, &catalog))
    );
    for (listed, left_out) in tech.project_libraries().1 {
        tech.notes.push(format!(
            "libraries: the PDK has two libraries named '{}'; a project lists {} and not \
             {} (a library cannot have several folders yet)",
            listed.name, listed.path, left_out.path
        ));
    }
    (tech, catalog)
}
