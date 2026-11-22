//! Technology (PDK) description, and which PDK a project uses.
//!
//! A project names its PDK with `technology = "ihp-sg13g2"` (the value of `$PDK`) and,
//! optionally, where PDKs are installed with `pdk_root = "/opt/ext/OpenPDKs"` (the value
//! of `$PDK_ROOT`). Whatever the project file leaves out comes from the environment
//! CDePT was started in. The result is a [`Pdk`]; it is what `$PDK_ROOT` and `$PDK` mean
//! in library paths and `[xschem] rc`, and what every tool CDePT starts gets in its
//! environment (see `Project::tool_env`).

use std::ffi::OsString;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Technology {
    pub name: String,
    pub file: PathBuf,
}

/// Where a value of [`Pdk`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The user's own file next to the project file (`cdept.local.toml`).
    Local,
    /// The project file.
    Project,
    /// The technology snapshot (`tech.toml`): the PDK it was made for, when the
    /// project file does not name one.
    Snapshot,
    /// The environment CDePT was started in.
    Environment,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Local => "cdept.local.toml",
            Source::Project => "project",
            Source::Snapshot => "tech.toml",
            Source::Environment => "environment",
        }
    }
}

/// A value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub value: String,
    pub source: Source,
}

/// The PDK a project works with: `$PDK_ROOT` and `$PDK`, each possibly unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pdk {
    pub root: Option<Resolved>,
    pub name: Option<Resolved>,
    /// The PDKs the environment offers: `$PDK` may list several, separated by colons
    /// (`sky130A:gf180mcu:ihp-sg13g2` in the asic-all image). The project chooses one
    /// with `technology`; with several on offer and none chosen, [`Pdk::name`] is unset.
    pub available: Vec<String>,
}

/// The names in a `$PDK` value: one, or several separated by colons.
pub fn pdk_names(value: &str) -> Vec<String> {
    value
        .split(':')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

impl Pdk {
    /// The project's `pdk_root` (already expanded) and `technology` first, then the
    /// environment's `$PDK_ROOT` and `$PDK`. When the project and the environment name
    /// different PDKs, the project wins and `warnings` gets a note.
    pub fn resolve(
        pdk_root: Option<String>,
        technology: Option<&str>,
        env: fn(&str) -> Option<String>,
        warnings: &mut Vec<String>,
    ) -> Pdk {
        let root = pdk_root.map(|value| Resolved {
            value,
            source: Source::Project,
        });
        Self::resolve_from(root, technology, &env, warnings)
    }

    /// [`Pdk::resolve`] with the PDK folder's source given (the project file or the
    /// user's local file) and any lookup for the environment.
    pub fn resolve_from(
        pdk_root: Option<Resolved>,
        technology: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
        warnings: &mut Vec<String>,
    ) -> Pdk {
        let from_env = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let available = from_env("PDK").map(|v| pdk_names(&v)).unwrap_or_default();
        let name = match technology {
            Some(value) => Some(Resolved {
                value: value.to_string(),
                source: Source::Project,
            }),
            None => match available.as_slice() {
                [one] => Some(Resolved {
                    value: one.clone(),
                    source: Source::Environment,
                }),
                [] => None,
                several => {
                    warnings.push(format!(
                        "$PDK offers several PDKs ({}): set `technology` in the project \
                         file to choose one",
                        several.join(", ")
                    ));
                    None
                }
            },
        };
        if let Some(project) = technology
            && !available.is_empty()
            && !available.iter().any(|name| name == project)
        {
            warnings.push(format!(
                "technology = \"{project}\" in the project file, but $PDK is \"{}\" \
                 in the environment: using \"{project}\"",
                available.join(":")
            ));
        }
        if let (Some(project), Some(other)) = (&pdk_root, from_env("PDK_ROOT"))
            && project.value != other
        {
            tracing::debug!(
                "pdk_root of the {} ({}) replaces $PDK_ROOT ({other})",
                project.source.name(),
                project.value
            );
        }
        let root = pdk_root.or_else(|| {
            from_env("PDK_ROOT").map(|value| Resolved {
                value,
                source: Source::Environment,
            })
        });
        Pdk {
            root,
            name,
            available,
        }
    }

    /// The value of `$PDK_ROOT` or `$PDK` for this project.
    pub fn var(&self, name: &str) -> Option<&str> {
        match name {
            "PDK_ROOT" => self.root.as_ref(),
            "PDK" => self.name.as_ref(),
            _ => None,
        }
        .map(|r| r.value.as_str())
    }

    /// `<$PDK_ROOT>/<$PDK>` as a PDK folder, when both are known.
    pub fn dir(&self) -> Option<crate::tech::PdkDir> {
        Some(crate::tech::PdkDir::new(
            self.var("PDK_ROOT")?,
            self.var("PDK")?,
        ))
    }

    /// `PDK_ROOT` and `PDK` as environment variables for a tool CDePT starts. `PDK` is
    /// always one name: a list in the environment is replaced by the PDK chosen.
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        [("PDK_ROOT", &self.root), ("PDK", &self.name)]
            .into_iter()
            .filter_map(|(name, r)| Some((name.into(), r.as_ref()?.value.clone().into())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "PDK_ROOT" => Some("/env/pdks".into()),
            "PDK" => Some("sky130A".into()),
            _ => None,
        }
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn project_beats_environment() {
        let mut warnings = Vec::new();
        let pdk = Pdk::resolve(Some("/p".into()), Some("ihp-sg13g2"), env, &mut warnings);
        assert_eq!(pdk.var("PDK_ROOT"), Some("/p"));
        assert_eq!(pdk.var("PDK"), Some("ihp-sg13g2"));
        assert_eq!(pdk.root.as_ref().unwrap().source, Source::Project);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("sky130A"));
    }

    #[test]
    fn environment_fills_the_gaps() {
        let mut warnings = Vec::new();
        let pdk = Pdk::resolve(None, None, env, &mut warnings);
        assert_eq!(pdk.var("PDK_ROOT"), Some("/env/pdks"));
        assert_eq!(pdk.name.as_ref().unwrap().source, Source::Environment);
        assert!(warnings.is_empty());
        // Same name in both: no warning.
        Pdk::resolve(None, Some("sky130A"), env, &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn unset_stays_unset() {
        let mut warnings = Vec::new();
        let pdk = Pdk::resolve(None, None, none, &mut warnings);
        assert_eq!(pdk, Pdk::default());
        assert!(pdk.env().is_empty());
        assert_eq!(pdk.var("HOME"), None);
        let pdk = Pdk::resolve(Some("/r".into()), None, none, &mut warnings);
        assert_eq!(pdk.env(), [("PDK_ROOT".into(), "/r".into())]);
    }

    fn several(name: &str) -> Option<String> {
        match name {
            "PDK_ROOT" => Some("/env/pdks".into()),
            "PDK" => Some("sky130A:gf180mcuD: ihp-sg13g2".into()),
            _ => None,
        }
    }

    #[test]
    fn several_pdks_in_the_environment_are_an_offer() {
        // None chosen: no PDK, one warning naming them.
        let mut warnings = Vec::new();
        let pdk = Pdk::resolve(None, None, several, &mut warnings);
        assert_eq!(pdk.name, None);
        assert_eq!(pdk.available, ["sky130A", "gf180mcuD", "ihp-sg13g2"]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(!pdk.env().iter().any(|(name, _)| name == "PDK"));
        assert!(pdk.dir().is_none());
        // One of them chosen by the project: no warning, tools get the one name.
        let mut warnings = Vec::new();
        let pdk = Pdk::resolve(None, Some("ihp-sg13g2"), several, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(pdk.env().contains(&("PDK".into(), "ihp-sg13g2".into())));
        assert_eq!(
            pdk.dir().unwrap().dir(),
            std::path::Path::new("/env/pdks/ihp-sg13g2")
        );
        // Another one: the project wins, with a warning.
        let pdk = Pdk::resolve(None, Some("xh018"), several, &mut warnings);
        assert_eq!(pdk.var("PDK"), Some("xh018"));
        assert_eq!(warnings.len(), 1);
        // The local file as the source of the folder.
        let root = Resolved {
            value: "/mine".into(),
            source: Source::Local,
        };
        let pdk = Pdk::resolve_from(Some(root), None, &none, &mut Vec::new());
        assert_eq!(pdk.root.unwrap().source.name(), "cdept.local.toml");
    }
}
