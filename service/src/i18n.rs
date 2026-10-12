//! Translations for the KWin script.
//!
//! KWin's JavaScript has no i18n functions, so the script asks the service
//! for its strings. They come from the same gettext catalog that the script's
//! settings page uses, shipped inside the script package as
//! `contents/locale/<lang>/LC_MESSAGES/muteonminimize.mo`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

const DOMAIN: &str = "muteonminimize";
const PACKAGE: &str = "kwin/scripts/muteonminimize";

/// The session's translations as a JSON object mapping English text to
/// translated text; `{}` when there is no catalog for the language.
pub fn translations_json() -> String {
    let catalog = find_catalog(&languages())
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| parse_mo(&bytes))
        .unwrap_or_default();
    to_json(&catalog)
}

/// Candidate catalog names in order of preference, gettext style: every
/// entry of `LANGUAGE`, then the locale from `LC_ALL`, `LC_MESSAGES` or
/// `LANG`; each as given (`ru_RU`) and as a bare language (`ru`).
fn languages() -> Vec<String> {
    let var = |name| env::var(name).ok().filter(|v| !v.is_empty());
    let locale = var("LC_ALL")
        .or_else(|| var("LC_MESSAGES"))
        .or_else(|| var("LANG"));
    languages_from(var("LANGUAGE").as_deref(), locale.as_deref())
}

fn languages_from(language: Option<&str>, locale: Option<&str>) -> Vec<String> {
    // gettext ignores LANGUAGE when the locale is C.
    if locale.is_some_and(is_c_locale) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let entries = language
        .into_iter()
        .flat_map(|l| l.split(':'))
        .chain(locale);
    for entry in entries {
        // ru_RU.UTF-8@modifier -> ru_RU
        let name = entry.split(['.', '@']).next().unwrap_or_default();
        if name.is_empty() || is_c_locale(name) {
            continue;
        }
        let bare = name.split('_').next().unwrap_or(name);
        for candidate in [name, bare] {
            if !out.iter().any(|known| known == candidate) {
                out.push(candidate.to_owned());
            }
        }
    }
    out
}

fn is_c_locale(locale: &str) -> bool {
    matches!(locale.split('.').next(), Some("C" | "POSIX"))
}

/// The script package can be installed per user or system-wide; look where
/// KPackage looks, user data first.
fn data_dirs() -> Vec<PathBuf> {
    let home_data = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    home_data
        .into_iter()
        .chain(system.split(':').map(PathBuf::from))
        .collect()
}

fn find_catalog(languages: &[String]) -> Option<PathBuf> {
    let dirs = data_dirs();
    languages.iter().find_map(|lang| {
        dirs.iter()
            .map(|dir| {
                dir.join(PACKAGE)
                    .join("contents/locale")
                    .join(lang)
                    .join("LC_MESSAGES")
                    .join(format!("{DOMAIN}.mo"))
            })
            .find(|path| path.is_file())
    })
}

/// Reads a GNU gettext `.mo` file. Only plain messages are kept: the header,
/// messages with a context, and untranslated entries are skipped; for plural
/// messages the singular forms are used.
fn parse_mo(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    let word = |offset: usize, big_endian: bool| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    };
    let big_endian = match word(0, false)? {
        0x9504_12de => false,
        0xde12_0495 => true,
        _ => return None,
    };
    let count = word(8, big_endian)? as usize;
    let originals = word(12, big_endian)? as usize;
    let translations = word(16, big_endian)? as usize;
    let string = |table: usize, index: usize| -> Option<&str> {
        let entry = table.checked_add(index.checked_mul(8)?)?;
        let len = word(entry, big_endian)? as usize;
        let start = word(entry + 4, big_endian)? as usize;
        std::str::from_utf8(bytes.get(start..start.checked_add(len)?)?).ok()
    };

    let mut catalog = BTreeMap::new();
    for index in 0..count {
        let original = string(originals, index)?;
        let translated = string(translations, index)?;
        let original = original.split('\0').next().unwrap_or_default();
        let translated = translated.split('\0').next().unwrap_or_default();
        if original.is_empty() || original.contains('\u{4}') || translated.is_empty() {
            continue;
        }
        catalog.insert(original.to_owned(), translated.to_owned());
    }
    Some(catalog)
}

fn to_json(catalog: &BTreeMap<String, String>) -> String {
    let pairs: Vec<String> = catalog
        .iter()
        .map(|(key, value)| format!("{}:{}", json_string(key), json_string(value)))
        .collect();
    format!("{{{}}}", pairs.join(","))
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a little-endian .mo file from (original, translation) pairs,
    /// which must be sorted by original like msgfmt does.
    fn build_mo(entries: &[(&str, &str)]) -> Vec<u8> {
        let n = entries.len();
        let originals = 28;
        let translations = originals + n * 8;
        let mut data = translations + n * 8;
        let mut header = Vec::new();
        let mut tables = (Vec::new(), Vec::new());
        let mut strings = Vec::new();
        for (table, pick) in [(&mut tables.0, 0), (&mut tables.1, 1)] {
            for entry in entries {
                let text = if pick == 0 { entry.0 } else { entry.1 };
                table.extend((text.len() as u32).to_le_bytes());
                table.extend((data as u32).to_le_bytes());
                strings.extend(text.as_bytes());
                strings.push(0);
                data += text.len() + 1;
            }
        }
        for value in [
            0x9504_12de_u32,
            0,
            n as u32,
            originals as u32,
            translations as u32,
            0,
            0,
        ] {
            header.extend(value.to_le_bytes());
        }
        [header, tables.0, tables.1, strings].concat()
    }

    #[test]
    fn parses_mo() {
        let mo = build_mo(&[
            ("", "Content-Type: text/plain; charset=UTF-8\n"),
            ("%1: muted", "%1: заглушено"),
            ("Apple\0Apples", "Яблоко\0Яблока"),
            ("ctx\u{4}Menu", "Меню"),
            ("Untranslated", ""),
        ]);
        let catalog = parse_mo(&mo).unwrap();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog["%1: muted"], "%1: заглушено");
        assert_eq!(catalog["Apple"], "Яблоко");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_mo(b"not a catalog").is_none());
        let mut truncated = build_mo(&[("a", "b")]);
        truncated.truncate(40);
        assert!(parse_mo(&truncated).is_none());
    }

    #[test]
    fn picks_languages() {
        assert_eq!(languages_from(None, Some("ru_RU.UTF-8")), ["ru_RU", "ru"]);
        assert_eq!(
            languages_from(Some("uk:de_DE"), Some("ru_RU.UTF-8")),
            ["uk", "de_DE", "de", "ru_RU", "ru"]
        );
        assert_eq!(languages_from(None, Some("sr_RS@latin")), ["sr_RS", "sr"]);
        assert!(languages_from(Some("ru"), Some("C.UTF-8")).is_empty());
        assert!(languages_from(None, None).is_empty());
    }

    #[test]
    fn escapes_json() {
        let catalog = BTreeMap::from([("a \"b\"\n".to_owned(), "в\\г\u{1}".to_owned())]);
        assert_eq!(to_json(&catalog), r#"{"a \"b\"\n":"в\\г\u0001"}"#);
        assert_eq!(to_json(&BTreeMap::new()), "{}");
    }
}
