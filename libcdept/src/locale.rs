//! The character set of the helper processes.
//!
//! xschem, Magic and KLayout read file names in the character set of the locale. With
//! `LANG` unset or `C` (usual in containers and services) a project under a folder with
//! non-ASCII letters does not load: Magic's helper never gets ready, xschem reports only
//! "netlisting errors". CDePT therefore starts its helpers with a UTF-8 character set.

use std::ffi::OsString;
use std::process::Command;

const UTF8: &str = "C.UTF-8";

/// Variables to set (`Some`) or remove (`None`) so that the child's `LC_CTYPE` is UTF-8.
/// Nothing when it already is. The other locale categories keep their effect.
fn utf8_changes(get: impl Fn(&str) -> Option<OsString>) -> Vec<(&'static str, Option<OsString>)> {
    let set = |name: &str| get(name).filter(|value| !value.is_empty());
    let all = set("LC_ALL");
    let effective = all
        .clone()
        .or_else(|| set("LC_CTYPE"))
        .or_else(|| set("LANG"));
    let is_utf8 = effective.is_some_and(|value| {
        let value = value.to_string_lossy().to_ascii_lowercase();
        value.contains("utf-8") || value.contains("utf8")
    });
    if is_utf8 {
        return Vec::new();
    }
    let mut changes = Vec::new();
    if let Some(all) = all {
        // LC_ALL overrides every category: hand its value to LANG, drop what it hid.
        changes.push(("LC_ALL", None));
        changes.push(("LANG", Some(all)));
        for name in [
            "LC_NUMERIC",
            "LC_COLLATE",
            "LC_TIME",
            "LC_MONETARY",
            "LC_MESSAGES",
        ] {
            changes.push((name, None));
        }
    }
    changes.push(("LC_CTYPE", Some(UTF8.into())));
    changes
}

/// Gives `command` a UTF-8 character set. `env` are the variables the caller adds to the
/// inherited ones; call this after setting them.
pub(crate) fn ensure_utf8(command: &mut Command, env: &[(OsString, OsString)]) {
    let changes = utf8_changes(|name| {
        env.iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var_os(name))
    });
    for (name, value) in changes {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(env: &[(&str, &str)]) -> Vec<(&'static str, Option<String>)> {
        utf8_changes(|name| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
        .into_iter()
        .map(|(name, value)| (name, value.map(|v| v.into_string().unwrap())))
        .collect()
    }

    fn ctype() -> (&'static str, Option<String>) {
        ("LC_CTYPE", Some(UTF8.to_string()))
    }

    #[test]
    fn a_utf8_locale_is_left_alone() {
        assert!(changes(&[("LANG", "en_US.UTF-8")]).is_empty());
        assert!(changes(&[("LANG", "C"), ("LC_CTYPE", "ja_JP.utf8")]).is_empty());
        assert!(changes(&[("LANG", "C"), ("LC_ALL", "C.UTF-8")]).is_empty());
    }

    #[test]
    fn no_locale_or_c_gets_a_utf8_character_set() {
        assert_eq!(changes(&[]), [ctype()]);
        assert_eq!(changes(&[("LANG", "C")]), [ctype()]);
        // An empty variable counts as unset.
        assert_eq!(changes(&[("LC_ALL", ""), ("LANG", "POSIX")]), [ctype()]);
    }

    #[test]
    fn lc_all_does_not_expose_other_categories() {
        let got = changes(&[("LC_ALL", "C"), ("LANG", "de_DE.UTF-8")]);
        assert!(got.contains(&("LC_ALL", None)));
        assert!(got.contains(&("LANG", Some("C".to_string()))));
        assert!(got.contains(&("LC_NUMERIC", None)));
        assert_eq!(got.last(), Some(&ctype()));
    }
}
