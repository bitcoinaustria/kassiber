//! Localization from the shared i18next bundles (`assets/i18n/locales/{en,de}` at the repository
//! root), embedded at build time, so the web UI and this app share one source of truth.
//!
//! The subset of i18next the bundles use: namespaces (`"transactions:table.title"`, default
//! `common`), nested keys, `{{name}}` interpolation, and `_zero` / `_one` / `_other` plurals
//! picked by a `count` argument (English and German share the one/other rule). A key missing in
//! German falls back to English, as `fallbackLng: "en"` does; a key missing in both renders as
//! the key itself, so gaps stay visible. Every miss is recorded (`missing()`), and
//! `--i18n-report <file>` writes them out on exit.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
        }
    }
    /// The native label the language switcher shows (`SUPPORTED_LANGUAGES`).
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
        }
    }
    pub const ALL: [Lang; 2] = [Lang::En, Lang::De];
}

macro_rules! bundles {
    ($lang:literal: $($ns:literal),* $(,)?) => {
        &[$(($ns, include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/i18n/locales/", $lang, "/", $ns, ".json")))),*]
    };
}

const EN: &[(&str, &str)] = bundles!("en": "assistant", "chainAnalysis", "chrome", "common", "connections", "custodyGaps", "journals", "nav", "onboarding", "overview", "privacyMirror", "reports", "review", "search", "settings", "sourceFunds", "transactions");
const DE: &[(&str, &str)] = bundles!("de": "assistant", "chainAnalysis", "chrome", "common", "connections", "custodyGaps", "journals", "nav", "onboarding", "overview", "privacyMirror", "reports", "review", "search", "settings", "sourceFunds", "transactions");

type Table = HashMap<String, String>;

fn flatten(prefix: &str, v: &Value, out: &mut Table) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                flatten(
                    &if prefix.ends_with(':') {
                        format!("{prefix}{k}")
                    } else {
                        format!("{prefix}.{k}")
                    },
                    v,
                    out,
                );
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                flatten(&format!("{prefix}.{i}"), v, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
        }
        other => {
            out.insert(prefix.to_owned(), other.to_string());
        }
    }
}

fn load(bundles: &[(&str, &str)]) -> Table {
    let mut t = Table::new();
    for (ns, src) in bundles {
        let v: Value =
            serde_json::from_str(src).unwrap_or_else(|e| panic!("locale bundle {ns}: {e}"));
        flatten(&format!("{ns}:"), &v, &mut t);
    }
    t
}

fn table(lang: Lang) -> &'static Table {
    static EN_T: OnceLock<Table> = OnceLock::new();
    static DE_T: OnceLock<Table> = OnceLock::new();
    match lang {
        Lang::En => EN_T.get_or_init(|| load(EN)),
        Lang::De => DE_T.get_or_init(|| load(DE)),
    }
}

thread_local! {
    static LANG: Cell<Lang> = const { Cell::new(Lang::En) };
    static MISSING: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// The language views render in; set from the app's preference before each view.
pub fn set_lang(lang: Lang) {
    LANG.with(|l| l.set(lang));
}

pub fn lang() -> Lang {
    LANG.with(|l| l.get())
}

/// `ns:key` lookups that fell back to English (`de:` prefix) or to the key itself (`*:` prefix).
pub fn missing() -> Vec<String> {
    MISSING.with(|m| m.borrow().iter().cloned().collect())
}

fn qualify(key: &str) -> String {
    if key.contains(':') {
        key.to_owned()
    } else {
        format!("common:{key}")
    }
}

fn raw(key: &str, lang: Lang) -> Option<&'static str> {
    table(lang).get(key).map(String::as_str)
}

fn lookup_in(key: &str, lang: Lang) -> Option<&'static str> {
    if let Some(s) = raw(key, lang) {
        return Some(s);
    }
    if lang != Lang::En
        && let Some(s) = raw(key, Lang::En)
    {
        MISSING.with(|m| m.borrow_mut().insert(format!("{}:{key}", lang.code())));
        return Some(s);
    }
    None
}

fn lookup(key: &str) -> Option<&'static str> {
    lookup_in(key, lang())
}

/// Plural key for `count` (i18next v4 suffixes; English and German: 1 → one, else other;
/// `_zero` wins for 0 when the bundle has it).
fn plural_key_in(key: &str, count: f64, lang: Lang) -> Option<&'static str> {
    if count == 0.0
        && let Some(s) = lookup_in(&format!("{key}_zero"), lang)
    {
        return Some(s);
    }
    let suffix = if count == 1.0 { "_one" } else { "_other" };
    lookup_in(&format!("{key}{suffix}"), lang)
}

fn plural_key(key: &str, count: f64) -> Option<&'static str> {
    plural_key_in(key, count, lang())
}

fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        // `{{name, format}}` keeps the name; formats are applied by callers before passing.
        let name = after[..end].split(',').next().unwrap_or("").trim();
        match args.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => {
                out.push_str("{{");
                out.push_str(&after[..end]);
                out.push_str("}}");
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Translates `key` in a specific language (`"ns:path"`, or `"path"` in `common`).
pub fn t_in(key: &str, lang: Lang) -> String {
    tr_in(key, lang, &[])
}

/// Translates with interpolation arguments in a specific language.
pub fn tr_in(key: &str, lang: Lang, args: &[(&str, &str)]) -> String {
    let key = qualify(key);
    let count = args
        .iter()
        .find(|(k, _)| *k == "count")
        .and_then(|(_, v)| v.replace([',', '.', '\u{a0}', ' '], "").parse::<f64>().ok());
    let found = count
        .and_then(|n| plural_key_in(&key, n, lang))
        .or_else(|| lookup_in(&key, lang));
    match found {
        Some(s) => interpolate(s, args),
        None => {
            MISSING.with(|m| m.borrow_mut().insert(format!("*:{key}")));
            key
        }
    }
}

/// Translates `key` (`"ns:path"`, or `"path"` in `common`).
pub fn t(key: &str) -> String {
    tr(key, &[])
}

/// Translates with interpolation arguments. A `count` argument that parses as a number picks
/// the plural form.
pub fn tr(key: &str, args: &[(&str, &str)]) -> String {
    tr_in(key, lang(), args)
}

/// Translates with a count (formatted with the language's grouping) and other arguments.
pub fn tc(key: &str, count: usize, args: &[(&str, &str)]) -> String {
    let n = crate::format::count_in(count, lang());
    let mut all: Vec<(&str, &str)> = vec![("count", n.as_str())];
    all.extend_from_slice(args);
    let key_q = qualify(key);
    let found = plural_key(&key_q, count as f64).or_else(|| lookup(&key_q));
    match found {
        Some(s) => interpolate(s, &all),
        None => {
            MISSING.with(|m| m.borrow_mut().insert(format!("*:{key_q}")));
            key_q
        }
    }
}

/// Whether a key exists in the current language or English.
pub fn has(key: &str) -> bool {
    let key = qualify(key);
    raw(&key, lang()).is_some() || raw(&key, Lang::En).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_load_and_plurals_interpolate() {
        set_lang(Lang::En);
        assert!(table(Lang::En).len() > 7000, "{}", table(Lang::En).len());
        assert!(table(Lang::De).len() > 7000);
        assert_eq!(
            interpolate("{{a}} and {{b}} {{c}}", &[("a", "1"), ("b", "2")]),
            "1 and 2 {{c}}"
        );
        assert_eq!(t("nav:book.overview"), "Overview");
        assert_eq!(tc("nav:badge.review", 3, &[]), "3 left to review");
        assert_eq!(t("no:such.key"), "no:such.key");
        set_lang(Lang::De);
        assert_eq!(
            t("nav:book.transactions"),
            table(Lang::De)["nav:book.transactions"]
        );
        assert_ne!(t("nav:book.transactions"), "Transactions");
        set_lang(Lang::En);
    }
}
