//! The bundles themselves: both languages ship the same namespaces with the same keys, each
//! object declares a key once, and every namespace on disk is embedded. Ported from the web
//! app's `locales.test.ts` and `i18n.test.ts`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::*;

fn locales_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/i18n/locales"
    ))
}

/// The `*.json` namespaces of one locale directory, sorted.
fn namespaces_on_disk(lang: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(locales_dir().join(lang))
        .unwrap_or_else(|e| panic!("{lang}: {e}"))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".json").map(str::to_owned))
        .collect();
    names.sort();
    names
}

fn embedded(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::En => EN,
        Lang::De => DE,
    }
}

/// Dotted leaf paths of a bundle, sorted; arrays are leaves (`leafKeys` in `i18n.test.ts`).
fn leaf_keys(prefix: &str, v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                leaf_keys(&path, v, out);
            }
        }
        _ => out.push(prefix.to_owned()),
    }
}

/// Every object path that declares the same key more than once, e.g. `privacy.evidence`.
/// A JSON parser keeps one of the values and silently drops the other, so this reads the raw
/// text: one `seen` set per open object, and a string followed by `:` is a key.
fn duplicate_key_paths(json: &str) -> Vec<String> {
    let bytes = json.as_bytes();
    let mut duplicates: Vec<String> = Vec::new();
    let mut stack: Vec<(String, BTreeSet<String>)> = Vec::new();
    let mut current: Option<String> = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i + 1;
                i = start;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                assert!(i < bytes.len(), "unterminated string in bundle");
                let value = json[start..i].to_owned();
                i += 1;
                let mut probe = i;
                while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
                    probe += 1;
                }
                if bytes.get(probe) == Some(&b':') {
                    if let Some((path, seen)) = stack.last_mut()
                        && !seen.insert(value.clone())
                    {
                        let dup = if path.is_empty() {
                            value.clone()
                        } else {
                            format!("{path}.{value}")
                        };
                        if !duplicates.contains(&dup) {
                            duplicates.push(dup);
                        }
                    }
                    current = Some(value);
                }
                continue;
            }
            b'{' => {
                let parent = stack.last().map(|(p, _)| p.clone()).unwrap_or_default();
                let path = match current.take() {
                    Some(key) if parent.is_empty() => key,
                    Some(key) => format!("{parent}.{key}"),
                    None => parent,
                };
                stack.push((path, BTreeSet::new()));
            }
            b'}' => {
                stack.pop();
                current = None;
            }
            _ => {}
        }
        i += 1;
    }
    duplicates
}

#[test]
fn every_language_ships_the_same_namespaces_and_embeds_them_all() {
    let mut dirs: Vec<String> = std::fs::read_dir(locales_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    let mut codes: Vec<String> = Lang::ALL.iter().map(|l| l.code().to_owned()).collect();
    codes.sort();
    assert_eq!(
        dirs, codes,
        "a bundle directory for every language, and no other"
    );
    let reference = namespaces_on_disk(Lang::En.code());
    assert!(reference.len() > 10, "{reference:?}");
    for lang in Lang::ALL {
        assert_eq!(
            namespaces_on_disk(lang.code()),
            reference,
            "{} namespaces must match en",
            lang.code()
        );
        let mut built: Vec<String> = embedded(lang)
            .iter()
            .map(|(ns, _)| (*ns).to_owned())
            .collect();
        built.sort();
        assert_eq!(
            built,
            reference,
            "{}: every namespace on disk is embedded",
            lang.code()
        );
    }
}

#[test]
fn every_language_keeps_the_english_keys() {
    // Parity, not non-emptiness: no language drifts from another (no half-translated
    // namespace). Plural forms (`_one`, `_other`, `_zero`) are keys like any other: English
    // and German share the one/other rule.
    let keys = |lang: Lang| -> Vec<(String, Vec<String>)> {
        embedded(lang)
            .iter()
            .map(|(ns, src)| {
                let v: Value = serde_json::from_str(src)
                    .unwrap_or_else(|e| panic!("{}/{ns}: {e}", lang.code()));
                let mut out = Vec::new();
                leaf_keys("", &v, &mut out);
                out.sort();
                ((*ns).to_owned(), out)
            })
            .collect()
    };
    let reference = keys(Lang::En);
    for lang in Lang::ALL {
        for ((ns, want), (got_ns, got)) in reference.iter().zip(keys(lang)) {
            assert_eq!(*ns, got_ns);
            let missing: Vec<&String> = want.iter().filter(|k| !got.contains(k)).collect();
            let extra: Vec<&String> = got.iter().filter(|k| !want.contains(k)).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{}/{ns} keys must match en/{ns}: missing {missing:?}, extra {extra:?}",
                lang.code()
            );
        }
    }
}

#[test]
fn every_object_declares_each_key_once() {
    let mut offenders = Vec::new();
    for lang in Lang::ALL {
        for ns in namespaces_on_disk(lang.code()) {
            let path = locales_dir().join(lang.code()).join(format!("{ns}.json"));
            let dups = duplicate_key_paths(&std::fs::read_to_string(&path).unwrap());
            if !dups.is_empty() {
                offenders.push(format!("{}/{ns}: {dups:?}", lang.code()));
            }
        }
    }
    assert!(offenders.is_empty(), "duplicate keys: {offenders:#?}");
}

#[test]
fn the_duplicate_scan_finds_a_repeated_key() {
    // Pins the checker itself: a false negative would make the guard above pass vacuously.
    assert_eq!(
        duplicate_key_paths(r#"{"a":{"b":{"c":1},"b":{"d":2}},"e":3}"#),
        ["a.b"]
    );
    assert!(duplicate_key_paths(r#"{"a":{"b":1},"c":{"b":2}}"#).is_empty());
    // A repeated key inside a string value is not a key.
    assert!(duplicate_key_paths(r#"{"a":"\"b\": 1","b":2}"#).is_empty());
    assert_eq!(duplicate_key_paths(r#"{"x": 1, "x" : 2}"#), ["x"]);
}

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
