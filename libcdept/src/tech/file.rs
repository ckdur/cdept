//! Reading and writing `tech.toml` and `tech.catalog.json`.
//!
//! Both files sit next to `cdept.toml`. `tech.toml` is small and meant to be read
//! (and, when a PDK needs it, corrected) by hand; the catalog holds the long lists and
//! can always be made again with `cdept tech refresh`.

use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};

use super::{CATALOG_FILE, CATALOG_SCHEMA, Catalog, TECH_FILE, TECH_SCHEMA, Tech, VERSION};

/// What `tech.toml` starts with. `{pdk}` is replaced by the PDK name.
const HEADER: &str = "\
# CDePT technology snapshot of the PDK \"{pdk}\": what CDePT read out of the PDK's
# files when this project was set up for it. It says where each tool's set-up is
# (KLayout technology and decks, xschemrc, magicrc, netgen set-up), which model files
# and corners there are, and which libraries the PDK brings.
#
# Paths starting with $PDK_ROOT follow the PDK folder of each machine (cdept.local.toml
# or the environment). The long lists (layers, PCells, symbols, cells) are next to
# this file in tech.catalog.json.
#
# Made by `cdept init --technology {pdk}`. This file is read as it is: edit it by hand
# (the files of a library, a corner, ...) and the edit applies. `cdept tech refresh`
# makes it again from the PDK and replaces the whole file (the old one is kept as
# tech.toml.bak). What cdept.toml sets wins over this file.
";

/// The snapshot file of the project in `dir`.
pub fn tech_file(dir: &Path) -> PathBuf {
    dir.join(TECH_FILE)
}

/// The catalog file of the project in `dir`.
pub fn catalog_file(dir: &Path) -> PathBuf {
    dir.join(CATALOG_FILE)
}

/// `tech.toml` as text, with its header.
pub fn tech_text(tech: &Tech) -> Result<String> {
    let mut tech = tech.clone();
    if tech.schema.is_empty() {
        tech.schema = TECH_SCHEMA.into();
    }
    if tech.version == 0 {
        tech.version = VERSION;
    }
    let body = toml::to_string(&tech).map_err(|e| eyre!("cannot write the snapshot: {e}"))?;
    Ok(format!(
        "{}\n{}",
        HEADER.replace("{pdk}", &tech.pdk.name),
        tidy(&body)
    ))
}

/// Lists longer than this many characters on one line are written one entry per line.
const LONG_LIST: usize = 88;

/// Make the TOML text of a snapshot pleasant to read. It stays the same data:
///
/// - a list too long for one line is written one entry per line;
/// - a list of small tables *inside* an entry of another list (the Liberty corners of
///   a library, the sites of a library) is written as rows, one `{ … }` per line,
///   instead of one `[[a.b.c]]` block per row.
///
/// Text that does not parse is returned as it is.
fn reshape(text: &str) -> String {
    use toml_edit::{Array, DocumentMut, Item, Table, Value};

    /// One entry per line.
    fn one_per_line(list: &mut Array) {
        for item in list.iter_mut() {
            item.decor_mut().set_prefix("\n    ");
            item.decor_mut().set_suffix("");
        }
        list.set_trailing_comma(true);
        list.set_trailing("\n");
    }
    fn in_value(value: &mut Value) {
        match value {
            Value::Array(list) => {
                for item in list.iter_mut() {
                    in_value(item);
                }
                if list.len() > 1 && list.to_string().trim().len() > LONG_LIST {
                    one_per_line(list);
                }
            }
            Value::InlineTable(table) => {
                for (_, item) in table.iter_mut() {
                    in_value(item);
                }
            }
            _ => {}
        }
    }
    /// Only plain values: such a table fits on a line.
    fn is_row(table: &Table) -> bool {
        table.iter().all(|(_, item)| match item {
            Item::Value(Value::Array(list)) => list.iter().all(|v| !v.is_inline_table()),
            Item::Value(Value::InlineTable(_)) => false,
            Item::Value(_) => true,
            _ => false,
        })
    }
    /// `in_list`: the table is an entry of a list of tables.
    fn in_table(table: &mut Table, in_list: bool) {
        for (mut key, item) in table.iter_mut() {
            match item {
                Item::Value(value) => in_value(value),
                Item::Table(table) => in_table(table, in_list),
                Item::ArrayOfTables(tables) => {
                    if in_list && !tables.is_empty() && tables.iter().all(is_row) {
                        let mut rows = Array::new();
                        for table in tables.iter() {
                            let mut row = table.clone().into_inline_table();
                            row.fmt();
                            rows.push(Value::InlineTable(row));
                        }
                        one_per_line(&mut rows);
                        rows.decor_mut().set_prefix(" ");
                        key.leaf_decor_mut().set_suffix(" ");
                        *item = Item::Value(Value::Array(rows));
                    } else {
                        for table in tables.iter_mut() {
                            in_table(table, true);
                        }
                    }
                }
                Item::None => {}
            }
        }
    }
    let Ok(mut doc) = text.parse::<DocumentMut>() else {
        return text.to_string();
    };
    in_table(doc.as_table_mut(), false);
    doc.to_string()
}

/// One section of the snapshot (`klayout`, `models`, …) as it is written in
/// `tech.toml`; `None` for a name that is no section or a section with nothing in it.
pub fn section_text(tech: &Tech, name: &str) -> Result<Option<String>> {
    let whole = toml::to_string(tech).map_err(|e| eyre!("cannot write the snapshot: {e}"))?;
    let mut doc: toml_edit::DocumentMut = whole
        .parse()
        .map_err(|e| eyre!("cannot write the snapshot: {e}"))?;
    if !doc.get(name).is_some_and(|item| item.is_table()) {
        return Ok(None);
    }
    let others: Vec<String> = doc
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| key != name)
        .collect();
    for key in others {
        doc.remove(&key);
    }
    Ok(Some(tidy(&doc.to_string())))
}

/// A blank line before every table header, none doubled; long lists one entry per
/// line.
fn tidy(body: &str) -> String {
    let body = &reshape(body);
    let mut out = String::with_capacity(body.len() + 64);
    let mut blank = true;
    for line in body.lines() {
        if line.trim().is_empty() {
            if !blank {
                out.push('\n');
            }
            blank = true;
            continue;
        }
        if line.starts_with('[') && !blank {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
        blank = false;
    }
    out
}

/// Parse the text of a `tech.toml`. `warnings` gets a note when the file was written
/// by a newer CDePT.
pub fn parse_tech(text: &str, warnings: &mut Vec<String>) -> Result<Tech> {
    let tech: Tech = toml::from_str(text).map_err(|e| eyre!("{}", e.to_string().trim_end()))?;
    if !tech.schema.is_empty() && tech.schema != TECH_SCHEMA {
        return Err(eyre!(
            "schema is \"{}\", not \"{TECH_SCHEMA}\": not a CDePT technology snapshot",
            tech.schema
        ));
    }
    if tech.version > VERSION {
        warnings.push(format!(
            "{TECH_FILE} is version {} and this CDePT knows version {VERSION}: \
             what it does not know is ignored",
            tech.version
        ));
    }
    // `[models]` holds its model sets as tables next to its own keys, so a key there
    // that is neither is not an unknown field but a model set that does not read:
    // say that it was ignored.
    if let Ok(written) = text.parse::<toml::Table>()
        && let Some(models) = written.get("models").and_then(|m| m.as_table())
    {
        let kept = toml::Table::try_from(&tech.models).unwrap_or_default();
        for key in models.keys().filter(|key| !kept.contains_key(*key)) {
            warnings.push(format!(
                "{TECH_FILE}: [models] {key} is neither a setting nor a model set (a table \
                 with `simulator`, `dir`, …): ignored"
            ));
        }
    }
    Ok(tech)
}

/// The snapshot of the project in `dir`; `None` when it has none.
pub fn load_tech(dir: &Path) -> Result<Option<Tech>> {
    let mut warnings = Vec::new();
    let tech = load_tech_with(dir, &mut warnings)?;
    for warning in warnings {
        tracing::warn!("{warning}");
    }
    Ok(tech)
}

/// [`load_tech`], with the notes about the file collected instead of logged.
pub fn load_tech_with(dir: &Path, warnings: &mut Vec<String>) -> Result<Option<Tech>> {
    let file = tech_file(dir);
    if !file.is_file() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    parse_tech(&text, warnings)
        .map(Some)
        .map_err(|e| eyre!("{}: {e}", file.display()))
}

/// The catalog of the project in `dir`; `None` when it has none.
pub fn load_catalog(dir: &Path) -> Result<Option<Catalog>> {
    let file = catalog_file(dir);
    if !file.is_file() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    let catalog: Catalog =
        serde_json::from_str(&text).map_err(|e| eyre!("{}: {e}", file.display()))?;
    if !catalog.schema.is_empty() && catalog.schema != CATALOG_SCHEMA {
        return Err(eyre!(
            "{}: schema is \"{}\", not \"{CATALOG_SCHEMA}\"",
            file.display(),
            catalog.schema
        ));
    }
    if catalog.version > VERSION {
        tracing::warn!(
            "{} is version {} and this CDePT knows version {VERSION}",
            file.display(),
            catalog.version
        );
    }
    Ok(Some(catalog))
}

/// The catalog as text.
pub fn catalog_text(catalog: &Catalog) -> Result<String> {
    let mut catalog = catalog.clone();
    if catalog.schema.is_empty() {
        catalog.schema = CATALOG_SCHEMA.into();
    }
    if catalog.version == 0 {
        catalog.version = VERSION;
    }
    let mut text = serde_json::to_string_pretty(&catalog)
        .map_err(|e| eyre!("cannot write the catalog: {e}"))?;
    text.push('\n');
    Ok(text)
}

/// The copy of the previous `tech.toml` that [`save`] leaves next to it (one
/// generation, replaced by the next save).
pub const BACKUP_FILE: &str = "tech.toml.bak";

/// What a scan over an existing `tech.toml` does, for the person to read before
/// agreeing: the whole file is replaced.
pub fn replace_warning() -> String {
    format!(
        "{TECH_FILE} is replaced as a whole by what the PDK says now: everything in it \
         that was edited by hand (the files of a library, a corner, a device map, ...) is \
         lost. The old file is kept as {BACKUP_FILE} (one copy, replaced by the next \
         scan)."
    )
}

/// Does the project folder `dir` have a `tech.toml` a scan would replace?
pub fn exists(dir: &Path) -> bool {
    tech_file(dir).is_file()
}

/// Write `tech.toml` and `tech.catalog.json` into the project folder `dir`. An
/// existing `tech.toml` is first copied to [`BACKUP_FILE`].
pub fn save(dir: &Path, tech: &Tech, catalog: &Catalog) -> Result<()> {
    let file = tech_file(dir);
    if file.is_file() {
        let backup = dir.join(BACKUP_FILE);
        fs::copy(&file, &backup)
            .wrap_err_with(|| format!("cannot write {}", backup.display()))?;
    }
    write(&file, &tech_text(tech)?)?;
    write(&catalog_file(dir), &catalog_text(catalog)?)
}

/// Remove both files (a project that gives up its PDK). Missing files are fine.
pub fn remove(dir: &Path) -> Result<()> {
    for file in [tech_file(dir), catalog_file(dir)] {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).wrap_err_with(|| format!("cannot remove {}", file.display())),
        }
    }
    Ok(())
}

/// Write through a temporary file so a failure cannot truncate the old one.
fn write(file: &Path, text: &str) -> Result<()> {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = file.with_file_name(name);
    fs::write(&tmp, text).wrap_err_with(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, file).wrap_err_with(|| format!("cannot write {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tech::{PdkDir, TechPath};

    fn sample() -> (Tech, Catalog) {
        let pdk = PdkDir::new("/opt/pdks", "ihp-sg13g2");
        let mut tech = Tech {
            schema: TECH_SCHEMA.into(),
            version: VERSION,
            ..Tech::default()
        };
        tech.pdk.name = pdk.name.clone();
        tech.pdk.family = pdk.family;
        tech.klayout.tech = Some("sg13g2".into());
        tech.klayout.dbu = Some(0.001);
        tech.klayout.lyt = Some(pdk.portable(&pdk.tech("klayout").join("tech/sg13g2.lyt")));
        tech.xschem.rc = Some(TechPath(
            "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc".into(),
        ));
        tech.notes.push("no netgen set-up found".into());
        let mut catalog = Catalog {
            schema: CATALOG_SCHEMA.into(),
            version: VERSION,
            pdk: pdk.name.clone(),
            ..Catalog::default()
        };
        catalog.klayout.layers.push(crate::tech::klayout::Layer {
            name: "Metal1".into(),
            layer: 8,
            datatype: 0,
            ..Default::default()
        });
        (tech, catalog)
    }

    #[test]
    fn both_files_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load_tech(tmp.path()).unwrap(), None);
        assert_eq!(load_catalog(tmp.path()).unwrap(), None);
        let (tech, catalog) = sample();
        save(tmp.path(), &tech, &catalog).unwrap();
        let text = fs::read_to_string(tech_file(tmp.path())).unwrap();
        assert!(text.starts_with("# CDePT technology snapshot of the PDK \"ihp-sg13g2\""));
        assert!(text.contains("schema = \"cdept-tech\""), "{text}");
        assert!(text.contains("\n\n[klayout]\n"), "{text}");
        assert!(!text.contains("\n\n\n"), "{text}");
        // One section, as it is in the file.
        let section = section_text(&tech, "klayout").unwrap().unwrap();
        assert!(
            section.starts_with("[klayout]\ntech = \"sg13g2\"\n"),
            "{section}"
        );
        assert!(text.contains(&section), "{section}");
        assert_eq!(section_text(&tech, "nope").unwrap(), None);
        assert_eq!(section_text(&tech, "version").unwrap(), None);
        // A long list is written one entry per line, a short one on one.
        let mut long = tech.clone();
        long.pdk.tools = (0..12).map(|n| format!("a-tool-with-a-name-{n}")).collect();
        long.notes.push("b".into());
        let text = tech_text(&long).unwrap();
        assert!(
            text.contains("tools = [\n    \"a-tool-with-a-name-0\",\n"),
            "{text}"
        );
        assert!(
            text.contains("    \"a-tool-with-a-name-11\",\n]\n"),
            "{text}"
        );
        assert!(
            text.contains("notes = [\"no netgen set-up found\", \"b\"]"),
            "{text}"
        );
        assert_eq!(parse_tech(&text, &mut Vec::new()).unwrap(), long);
        assert_eq!(load_tech(tmp.path()).unwrap(), Some(tech));
        assert_eq!(load_catalog(tmp.path()).unwrap(), Some(catalog));
        remove(tmp.path()).unwrap();
        remove(tmp.path()).unwrap();
        assert_eq!(load_tech(tmp.path()).unwrap(), None);
    }

    #[test]
    fn rows_inside_a_list_entry_are_written_one_per_line() {
        let text = "\
[[lib]]
name = \"io\"

[[lib.corner]]
name = \"fast\"
voltage = 1.32

[[lib.corner]]
name = \"slow\"
voltage = 1.08

[[lib.deep]]
name = \"x\"

[lib.deep.inner]
a = 1

[[lib]]
name = \"sc\"
";
        let out = reshape(text);
        assert!(
            out.contains(
                "corner = [\n    { name = \"fast\", voltage = 1.32 },\n    \
                 { name = \"slow\", voltage = 1.08 },\n]\n"
            ),
            "{out}"
        );
        // Entries with tables of their own, and first-level lists, stay blocks.
        assert!(
            out.contains("[[lib.deep]]") && out.contains("[lib.deep.inner]"),
            "{out}"
        );
        assert_eq!(out.matches("[[lib]]").count(), 2, "{out}");
        // The same data.
        let (a, b): (toml::Table, toml::Table) =
            (toml::from_str(text).unwrap(), toml::from_str(&out).unwrap());
        assert_eq!(a, b);
        assert_eq!(reshape("not = [toml"), "not = [toml");
    }

    #[test]
    fn a_newer_version_loads_with_a_warning_and_another_schema_does_not() {
        let mut warnings = Vec::new();
        let tech = parse_tech(
            "schema = \"cdept-tech\"\nversion = 99\nfuture = 1\n[pdk]\nname = \"x\"\n\
             [hologram]\nfile = \"a\"\n",
            &mut warnings,
        )
        .unwrap();
        assert_eq!(tech.pdk.name, "x");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        // A key in [models] that is no model set is ignored, with a note.
        let mut warnings = Vec::new();
        let tech = parse_tech(
            "[models]\ndefault = \"ngspice\"\nmine = 3\nnote = \"x\"\n\
             [models.ngspice]\nsimulator = \"ngspice\"\ndir = \"$PDK_ROOT/x/models\"\n\
             [models.broken]\nsimulator = 7\n",
            &mut warnings,
        )
        .unwrap();
        assert_eq!(tech.models.default.as_deref(), Some("ngspice"));
        assert_eq!(tech.models.sets.keys().collect::<Vec<_>>(), ["ngspice"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("[models] mine")));
        assert!(warnings.iter().any(|w| w.contains("[models] broken")));
        assert!(parse_tech("schema = \"other\"\n", &mut Vec::new()).is_err());
        assert!(parse_tech("version = [\n", &mut Vec::new()).is_err());
    }
}
