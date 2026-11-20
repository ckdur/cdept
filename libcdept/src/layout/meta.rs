//! The sidecar of a layout file: `inv.gds` → `inv.gds.json` (docs/KLAYOUT.md §3.2).
//!
//! Written only by CDePT (Rust). Optional: a layout without one has its ghost cells
//! searched by name.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};
use serde::{Deserialize, Serialize};

/// Value of [`Meta::format`].
pub const FORMAT: &str = "cdept-layout";
/// Current [`Meta::version`]. Version 2 added the link with a schematic
/// ([`Meta::source`], [`Meta::instances`], docs/LINKING.md); a sidecar without a link is
/// still written as version 1.
pub const VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// Always [`FORMAT`].
    pub format: String,
    pub version: u32,
    /// What this file is: library name (as in `cdept.toml`), cell, view.
    pub library: String,
    pub cell: String,
    pub view: String,
    /// Name of the top cell inside the file.
    pub top: String,
    /// Key: structure name used in this file for a cell of another view.
    #[serde(default)]
    pub references: BTreeMap<String, MetaRef>,
    /// The schematic this layout is linked with (version 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// What each schematic instance was made into at the last sync, by instance name
    /// (version 2). The layout instance carries the name as a property (docs/LINKING.md).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub instances: BTreeMap<String, Linked>,
}

/// A schematic view: `library/cell/view`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub library: String,
    pub cell: String,
    pub view: String,
}

impl Source {
    pub fn label(&self) -> String {
        format!("{}/{}/{}", self.library, self.cell, self.view)
    }
}

/// One schematic instance as placed in the layout at the last sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Linked {
    /// The symbol, as the schematic refers to it.
    pub symbol: String,
    /// `pcell` (a PCell of a KLayout library) or `cell` (a layout view of the project).
    pub kind: String,
    /// KLayout library name (PCells) or CDePT library name (cells).
    pub library: String,
    pub cell: String,
    /// PCell parameters as set (PCells only).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaRef {
    pub library: String,
    pub cell: String,
    pub view: String,
    /// `[left, bottom, right, top]` in µm, in the referenced cell's coordinates, as last
    /// seen. Used for the "cannot be found" marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
}

impl Meta {
    /// A sidecar for a file that is `library/cell/view`, with top structure `top` and no
    /// references.
    pub fn new(library: &str, cell: &str, view: &str, top: &str) -> Meta {
        Meta {
            format: FORMAT.to_string(),
            version: 1,
            library: library.to_string(),
            cell: cell.to_string(),
            view: view.to_string(),
            top: top.to_string(),
            references: BTreeMap::new(),
            source: None,
            instances: BTreeMap::new(),
        }
    }

    /// True when the sidecar links the layout with a schematic.
    pub fn is_linked(&self) -> bool {
        self.source.is_some() || !self.instances.is_empty()
    }

    /// `library/cell/view`.
    pub fn label(&self) -> String {
        format!("{}/{}/{}", self.library, self.cell, self.view)
    }

    /// Check the fields; the error is a sentence for the user.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("format is '{}', expected '{FORMAT}'", self.format));
        }
        if self.version == 0 || self.version > VERSION {
            return Err(format!(
                "version {} is not supported (this CDePT reads up to {VERSION})",
                self.version
            ));
        }
        for (what, value) in [
            ("library", &self.library),
            ("cell", &self.cell),
            ("view", &self.view),
            ("top", &self.top),
        ] {
            if value.is_empty() {
                return Err(format!("'{what}' is empty"));
            }
        }
        for (name, r) in &self.references {
            if name.is_empty() {
                return Err("a reference has an empty name".to_string());
            }
            for (what, value) in [
                ("library", &r.library),
                ("cell", &r.cell),
                ("view", &r.view),
            ] {
                if value.is_empty() {
                    return Err(format!("reference '{name}': '{what}' is empty"));
                }
            }
            if let Some(b) = r.bbox
                && (b.iter().any(|v| !v.is_finite()) || b[0] > b[2] || b[1] > b[3])
            {
                return Err(format!(
                    "reference '{name}': bbox is not [left, bottom, right, top]"
                ));
            }
        }
        Ok(())
    }

    /// Read and validate a sidecar file. Errors name the file.
    pub fn read(sidecar: &Path) -> Result<Meta> {
        let text = fs::read_to_string(sidecar)
            .wrap_err_with(|| format!("cannot read {}", sidecar.display()))?;
        let meta: Meta = serde_json::from_str(&text)
            .map_err(|e| eyre!("{} is not a layout sidecar: {e}", sidecar.display()))?;
        meta.validate()
            .map_err(|e| eyre!("{}: {e}", sidecar.display()))?;
        Ok(meta)
    }

    /// The sidecar of `layout_file`; `None` when there is none.
    pub fn read_for(layout_file: &Path) -> Result<Option<Meta>> {
        let path = sidecar_path(layout_file);
        if !path.is_file() {
            return Ok(None);
        }
        Meta::read(&path).map(Some)
    }

    /// Write as pretty JSON with a trailing newline, through a temporary file.
    pub fn write(&self, sidecar: &Path) -> Result<()> {
        self.validate()
            .map_err(|e| eyre!("not writing {}: {e}", sidecar.display()))?;
        // The oldest version that can hold it, so files without a link stay readable
        // by an older CDePT.
        let mut meta = self.clone();
        meta.version = if meta.is_linked() { 2 } else { 1 };
        let mut text = serde_json::to_string_pretty(&meta)?;
        text.push('\n');
        super::write_atomic(sidecar, text.as_bytes())
    }

    /// Write the sidecar of `layout_file`.
    pub fn write_for(&self, layout_file: &Path) -> Result<()> {
        self.write(&sidecar_path(layout_file))
    }
}

/// The sidecar path of a layout file: the file name with `.json` appended.
pub fn sidecar_path(layout_file: &Path) -> PathBuf {
    let mut name = layout_file.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

/// True for a file name that is a sidecar of a layout file (`*.gds.json`,
/// `*.gds.gz.json`, `*.oas.json`, …), which the library scanner must skip.
pub fn is_sidecar_name(name: &str) -> bool {
    name.strip_suffix(".json")
        .and_then(crate::Format::split_name)
        .is_some_and(|(_, format)| crate::Format::tool(format) == Some("KLayout"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Meta {
        let mut m = Meta::new("analog", "buf", "layout", "buf");
        m.references.insert(
            "inv".into(),
            MetaRef {
                library: "analog".into(),
                cell: "inv".into(),
                view: "layout".into(),
                bbox: Some([0.0, 0.0, 1.2, 3.4]),
            },
        );
        m
    }

    #[test]
    fn sidecar_names() {
        assert!(is_sidecar_name("inv.gds.json"));
        assert!(is_sidecar_name("inv.gds.gz.json"));
        assert!(is_sidecar_name("inv.oas.json"));
        assert!(!is_sidecar_name("inv.json"));
        assert!(!is_sidecar_name("inv.gds"));
        assert!(!is_sidecar_name("setup.json"));
        assert_eq!(
            sidecar_path(Path::new("/a/inv.gds")),
            Path::new("/a/inv.gds.json")
        );
    }

    #[test]
    fn write_read_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let gds = tmp.path().join("buf.gds");
        sample().write_for(&gds).unwrap();
        let text = fs::read_to_string(sidecar_path(&gds)).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"format\": \"cdept-layout\""));
        assert_eq!(Meta::read_for(&gds).unwrap(), Some(sample()));
        assert_eq!(Meta::read_for(&tmp.path().join("x.gds")).unwrap(), None);
        // No temporary file is left.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_references_and_bbox_are_optional() {
        let text = r#"{"format":"cdept-layout","version":1,"library":"a","cell":"c",
            "view":"layout","top":"c"}"#;
        let m: Meta = serde_json::from_str(text).unwrap();
        assert!(m.references.is_empty());
        m.validate().unwrap();
    }

    #[test]
    fn validation_messages() {
        let mut m = sample();
        m.format = "other".into();
        assert!(m.validate().unwrap_err().contains("format"));
        let mut m = sample();
        m.version = 3;
        assert!(m.validate().unwrap_err().contains("version 3"));
        let mut m = sample();
        m.cell.clear();
        assert!(m.validate().unwrap_err().contains("'cell' is empty"));
        let mut m = sample();
        m.references.get_mut("inv").unwrap().bbox = Some([2.0, 0.0, 1.0, 1.0]);
        assert!(m.validate().unwrap_err().contains("inv"));
    }

    #[test]
    fn a_link_makes_version_2_and_reads_back() {
        let tmp = tempfile::tempdir().unwrap();
        let gds = tmp.path().join("inv.gds");
        let mut m = sample();
        m.write_for(&gds).unwrap();
        assert!(
            fs::read_to_string(sidecar_path(&gds))
                .unwrap()
                .contains("\"version\": 1")
        );
        m.source = Some(Source {
            library: "analog".into(),
            cell: "buf".into(),
            view: "schematic".into(),
        });
        m.instances.insert(
            "M1".into(),
            Linked {
                symbol: "sg13g2_pr/sg13_lv_nmos.sym".into(),
                kind: "pcell".into(),
                library: "SG13_dev".into(),
                cell: "nmos".into(),
                params: BTreeMap::from([("w".to_string(), "1u".to_string())]),
            },
        );
        m.write_for(&gds).unwrap();
        let text = fs::read_to_string(sidecar_path(&gds)).unwrap();
        assert!(text.contains("\"version\": 2"), "{text}");
        let back = Meta::read_for(&gds).unwrap().unwrap();
        assert_eq!(back.version, 2);
        assert_eq!(back.instances, m.instances);
        assert_eq!(back.source, m.source);
    }

    #[test]
    fn read_errors_name_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.gds.json");
        fs::write(&file, "{ nope").unwrap();
        let e = Meta::read(&file).unwrap_err().to_string();
        assert!(e.contains("a.gds.json"), "{e}");
        fs::write(
            &file,
            r#"{"format":"x","version":1,"library":"a","cell":"c","view":"v","top":"t"}"#,
        )
        .unwrap();
        assert!(
            Meta::read(&file)
                .unwrap_err()
                .to_string()
                .contains("format")
        );
    }
}
