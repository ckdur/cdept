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
    /// The project file.
    Project,
    /// The environment CDePT was started in.
    Environment,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Project => "project",
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
        let from_env = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let pick = |project: Option<String>, var: &str| match project {
            Some(value) => Some(Resolved {
                value,
                source: Source::Project,
            }),
            None => from_env(var).map(|value| Resolved {
                value,
                source: Source::Environment,
            }),
        };
        let pdk = Pdk {
            root: pick(pdk_root.clone(), "PDK_ROOT"),
            name: pick(technology.map(str::to_string), "PDK"),
        };
        if let (Some(project), Some(other)) = (technology, from_env("PDK"))
            && project != other
        {
            warnings.push(format!(
                "technology = \"{project}\" in the project file, but $PDK is \"{other}\" \
                 in the environment: using \"{project}\""
            ));
        }
        if let (Some(project), Some(other)) = (&pdk_root, from_env("PDK_ROOT"))
            && *project != other
        {
            tracing::debug!("pdk_root of the project ({project}) replaces $PDK_ROOT ({other})");
        }
        pdk
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

    /// `PDK_ROOT` and `PDK` as environment variables for a tool CDePT starts.
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
}
