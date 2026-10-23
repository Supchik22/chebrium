//! Translations of the texts of the shell and of every Chebrium program. Every text passes through `tr` on its way to the screen (labels when
//! they are drawn, widths when they are measured); a text that has no entry in the language's catalogue is shown
//! as it is, which is what happens with file names and anything typed by the user.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::RwLock;

static UK: &str = include_str!("../assets/i18n/uk.toml");

struct Lang {
    code: &'static str,
    table: HashMap<String, String>,
}

static CURRENT: RwLock<Option<Lang>> = RwLock::new(None);

/// The languages the shell can speak: (code, name shown in the settings).
pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("uk", "Українська")];

/// Choose the language: a code from `LANGUAGES`, or "auto" to follow the environment (`LC_ALL`, `LC_MESSAGES`, `LANG`).
pub fn set_language(choice: &str) {
    let code = match choice {
        "auto" | "" => detect(),
        c => c,
    };
    let lang = match code {
        "uk" => Some(Lang { code: "uk", table: toml::from_str(UK).unwrap_or_default() }),
        _ => None,
    };
    if let Ok(mut g) = CURRENT.write() {
        *g = lang;
    }
}

fn detect() -> &'static str {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return if v.starts_with("uk") { "uk" } else { "en" };
            }
        }
    }
    "en"
}

/// The code of the language in use ("en" when there is no translation).
pub fn language() -> &'static str {
    CURRENT.read().ok().and_then(|g| g.as_ref().map(|l| l.code)).unwrap_or("en")
}

/// `pattern` in the current language with each `{}` replaced by the next of `args` (`trf("Items: {}", &[n.to_string()])`).
/// The arguments are data and are not translated.
pub fn trf(pattern: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut rest = tr(pattern).into_owned();
    let mut it = args.iter();
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        out.push_str(it.next().map(String::as_str).unwrap_or(""));
        rest = rest[i + 2..].to_string();
    }
    out.push_str(&rest);
    out
}

/// `text` in the current language.
pub fn tr(text: &str) -> Cow<'_, str> {
    if let Ok(g) = CURRENT.read() {
        if let Some(t) = g.as_ref().and_then(|l| l.table.get(text)) {
            return Cow::Owned(t.clone());
        }
    }
    Cow::Borrowed(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_parses_and_translates() {
        set_language("uk");
        assert_eq!(tr("Settings"), "Налаштування");
        assert_eq!(tr("some-file-name.txt"), "some-file-name.txt");
        set_language("en");
        assert_eq!(tr("Settings"), "Settings");
    }

    #[test]
    fn a_pattern_takes_its_numbers_in_either_language() {
        set_language("uk");
        assert_eq!(trf("Items: {}", &["12".into()]), "Елементів: 12");
        set_language("en");
        assert_eq!(trf("Items: {}", &["12".into()]), "Items: 12");
        assert_eq!(trf("{} / {}", &["3".into(), "9".into()]), "3 / 9");
    }

    #[test]
    fn every_entry_has_a_translation() {
        let table: HashMap<String, String> = toml::from_str(UK).expect("uk.toml parses");
        assert!(table.len() > 200);
        assert!(table.values().all(|v| !v.trim().is_empty()));
    }
}
