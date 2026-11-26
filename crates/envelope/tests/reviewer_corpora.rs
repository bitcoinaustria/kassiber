//! The reviewers' redaction corpora through the secret floor, as the desktop's
//! log floor applies it (`crates/kassiber-desktop/src/secret_floor.rs`
//! compiles this crate's `redaction` source).
//!
//! Each corpus is frozen from one of the original Python generators (plan 22)
//! and locked test data: `tests/fixtures/secret_redaction/<name>-<seed>-<count>.jsonl.gz`,
//! with its seed, count and output digest in
//! `tests/fixtures/secret_redaction_<name>_generator.json`. Every case goes
//! through [`redact_secret_text`]; no planted secret may survive in any
//! decoded view of the output, and the preserved text must. A pinned seed's
//! outputs hash (one line each) to the digest Python produced. Ported from the
//! removed web app's `secretRedaction{Third,Fourth,Fifth,Sixth,Independent}.test.ts`.
//!
//! Strings are compared as UTF-16 code units, as those suites did.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use flate2::read::GzDecoder;
use kassiber_envelope::redaction::redact_secret_text;
use serde_json::Value;

#[path = "support/sha256.rs"]
mod sha256;
use sha256::Sha256;

type Units = Vec<u16>;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn spec(name: &str) -> Value {
    let path = root().join(format!(
        "tests/fixtures/secret_redaction_{name}_generator.json"
    ));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn number(value: &Value) -> u64 {
    value.as_u64().expect("a count or seed")
}

fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

fn strings(value: &Value) -> Vec<&str> {
    value.as_array().unwrap().iter().map(text).collect()
}

/// `[[text, kind], …]`.
fn pairs(value: &Value) -> Vec<(&str, &str)> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| (text(&pair[0]), text(&pair[1])))
        .collect()
}

/// Every case of one corpus with its floored output. Leaks are collected (the
/// first few are reported); returns the output digest.
fn floor_corpus(
    name: &str,
    seed: u64,
    count: u64,
    mut leaks: impl FnMut(&Value, &str) -> Vec<String>,
) -> String {
    let path = root().join(format!(
        "tests/fixtures/secret_redaction/{name}-{seed}-{count}.jsonl.gz"
    ));
    let file = File::open(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}: run scripts/fetch-test-data.sh",
            path.display()
        )
    });
    let mut digest = Sha256::new();
    let (mut cases, mut failures, mut failed) = (0u64, Vec::new(), 0usize);
    for line in BufReader::new(GzDecoder::new(file)).lines() {
        let case: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let output = redact_secret_text(text(&case["input"]));
        let found = leaks(&case, &output);
        if !found.is_empty() {
            failed += 1;
            if failures.len() < 5 {
                let index = if case["i"].is_null() {
                    &case["index"]
                } else {
                    &case["i"]
                };
                failures.push(format!(
                    "{name} case {seed}/{index}: {}: {:?} => {output:?}",
                    found.join(", "),
                    text(&case["input"])
                ));
            }
        }
        digest.update(output.as_bytes());
        digest.update(b"\n");
        cases += 1;
    }
    assert!(
        failures.is_empty(),
        "{failed} failing cases, the first:\n{}",
        failures.join("\n")
    );
    assert_eq!(cases, count, "{name}-{seed}: case count");
    digest.hex()
}

fn units(text: &str) -> Units {
    text.encode_utf16().collect()
}

fn is_hex(unit: u16) -> bool {
    char::from_u32(unit.into()).is_some_and(|c| c.is_ascii_hexdigit())
}

/// `view.replaceAll("\\", "")`.
fn without_backslashes(view: &[u16]) -> Units {
    view.iter()
        .copied()
        .filter(|&u| u != u16::from(b'\\'))
        .collect()
}

/// One layer of `/%([a-f0-9]{2})/gi` to `String.fromCharCode`.
fn percent_decoded(view: &[u16]) -> Units {
    let mut out = Vec::with_capacity(view.len());
    let mut i = 0;
    while i < view.len() {
        if view[i] == u16::from(b'%')
            && i + 2 < view.len()
            && is_hex(view[i + 1])
            && is_hex(view[i + 2])
        {
            let digits = String::from_utf16(&view[i + 1..i + 3]).unwrap();
            out.push(u16::from_str_radix(&digits, 16).unwrap());
            i += 3;
        } else {
            out.push(view[i]);
            i += 1;
        }
    }
    out
}

const ENTITIES: &[(&str, &str)] = &[
    ("quot", "\""),
    ("apos", "'"),
    ("lt", "<"),
    ("gt", ">"),
    ("amp", "&"),
    ("colon", ":"),
    ("sol", "/"),
    ("commat", "@"),
    ("equals", "="),
];

/// The replacement and end of an entity at `at` (`&`), as
/// `/&(?:#([0-9]{1,7})|#[xX]([0-9a-fA-F]{1,6})|([A-Za-z]+));/g` matches.
fn entity_at(view: &[u16], at: usize) -> Option<(Units, usize)> {
    let unit = |j: usize| view.get(j).and_then(|&u| char::from_u32(u.into()));
    let run = |from: usize, max: usize, ok: fn(char) -> bool| {
        let mut j = from;
        while j - from < max && unit(j).is_some_and(ok) {
            j += 1;
        }
        j
    };
    let code_point = |code: u32, end: usize| {
        // `String.fromCodePoint`, a lone surrogate included.
        let units = if code <= 0x10ffff {
            match char::from_u32(code) {
                Some(c) => c.encode_utf16(&mut [0; 2]).to_vec(),
                None => vec![code as u16],
            }
        } else {
            view[at..=end].to_vec()
        };
        Some((units, end + 1))
    };
    let text_of = |from: usize, to: usize| String::from_utf16(&view[from..to]).unwrap();
    if unit(at + 1) == Some('#') {
        let end = run(at + 2, 7, |c| c.is_ascii_digit());
        if end > at + 2 && unit(end) == Some(';') {
            return code_point(text_of(at + 2, end).parse().unwrap(), end);
        }
        if matches!(unit(at + 2), Some('x' | 'X')) {
            let end = run(at + 3, 6, |c| c.is_ascii_hexdigit());
            if end > at + 3 && unit(end) == Some(';') {
                return code_point(u32::from_str_radix(&text_of(at + 3, end), 16).unwrap(), end);
            }
        }
        return None;
    }
    let end = run(at + 1, usize::MAX, |c| c.is_ascii_alphabetic());
    if end > at + 1 && unit(end) == Some(';') {
        let name = text_of(at + 1, end);
        let replacement = ENTITIES
            .iter()
            .find(|(entity, _)| *entity == name)
            .map_or_else(|| view[at..=end].to_vec(), |(_, value)| units(value));
        return Some((replacement, end + 1));
    }
    None
}

fn entities_decoded(view: &[u16]) -> Units {
    let mut out = Vec::with_capacity(view.len());
    let mut i = 0;
    while i < view.len() {
        if view[i] == u16::from(b'&') {
            if let Some((replacement, end)) = entity_at(view, i) {
                out.extend(replacement);
                i = end;
                continue;
            }
        }
        out.push(view[i]);
        i += 1;
    }
    out
}

/// The output, without backslashes, each with up to three percent-decoding
/// layers, and all of those entity-decoded (the fourth to sixth generators).
fn decoded_views(output: &str) -> Vec<Units> {
    let plain = units(output);
    let bare = without_backslashes(&plain);
    let mut views = vec![plain.clone(), bare.clone()];
    for start in [plain, bare] {
        let mut view = start;
        for _ in 0..3 {
            view = percent_decoded(&view);
            views.push(view.clone());
        }
    }
    let entities: Vec<Units> = views.iter().map(|v| entities_decoded(v)).collect();
    views.extend(entities);
    views
}

fn window(units: &[u16]) -> u64 {
    units
        .iter()
        .fold(0u64, |key, &unit| (key << 16) | u64::from(unit))
}

/// The four-unit fragments of `secret`.
fn fragments(secret: &[u16]) -> HashSet<u64> {
    secret.windows(4).map(window).collect()
}

fn any_view_holds(views: &[Units], fragments: &HashSet<u64>) -> bool {
    !fragments.is_empty()
        && views
            .iter()
            .any(|view| view.windows(4).any(|w| fragments.contains(&window(w))))
}

fn contains(haystack: &[u16], needle: &[u16]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The fragments of each planted secret that no other planted secret and no
/// surrounding input carry, found in any decoded view.
fn leaked_secrets<'a>(
    input: &str,
    secrets: impl Iterator<Item = &'a str>,
    planted: &[&str],
    views: &[Units],
) -> Vec<String> {
    let others: Vec<(&str, Units)> = planted.iter().map(|s| (*s, units(s))).collect();
    let mut found = Vec::new();
    for secret in secrets {
        let blanked = units(&input.replace(secret, "\0"));
        let own = units(secret);
        let fragments: HashSet<u64> = own
            .windows(4)
            .filter(|fragment| {
                !others
                    .iter()
                    .any(|(other, units)| *other != secret && contains(units, fragment))
                    && !contains(&blanked, fragment)
            })
            .map(window)
            .collect();
        if any_view_holds(views, &fragments) {
            found.push(secret.to_owned());
        }
    }
    found
}

/// Occurrences of the digits `number` in `text` not bordered by a `border` character.
fn bordered_matches(text: &str, number: &str, border: fn(char) -> bool) -> usize {
    assert!(number.bytes().all(|b| b.is_ascii_digit()), "{number}");
    text.match_indices(number)
        .filter(|(at, _)| {
            !text[..*at].chars().next_back().is_some_and(border)
                && !text[at + number.len()..].chars().next().is_some_and(border)
        })
        .count()
}

fn sentinel_leaked(sentinel: &str, views: &[Units]) -> bool {
    any_view_holds(views, &fragments(&units(sentinel)))
}

#[test]
fn third_generator_corpus_floors_as_python() {
    let spec = spec("third");
    let digest = floor_corpus(
        "third",
        number(&spec["seed"]),
        number(&spec["count"]),
        |case, output| {
            let mut found = Vec::new();
            // The output, without backslashes, then up to three percent-decoding layers.
            let mut views = vec![units(output), without_backslashes(&units(output))];
            for _ in 0..3 {
                views.push(percent_decoded(views.last().unwrap()));
            }
            if text(&case["kind"]).contains("pctlow") {
                // `view.toUpperCase()`: a lower-case escape decoded upper-case.
                for view in &mut views {
                    *view = units(&String::from_utf16_lossy(view).to_uppercase());
                }
            }
            if sentinel_leaked(text(&case["sentinel"]), &views) {
                found.push("secret".to_owned());
            }
            let keep = text(&case["keep"]);
            if !output.contains(keep) {
                found.push(format!("lost {keep}"));
            }
            found
        },
    );
    assert_eq!(digest, text(&spec["output_sha256"]));
}

fn fourth_leaks(case: &Value, output: &str) -> Vec<String> {
    let mut found = Vec::new();
    if sentinel_leaked(text(&case["sentinel"]), &decoded_views(output)) {
        found.push("secret".to_owned());
    }
    for keep in strings(&case["keep"]) {
        if !output.contains(keep) {
            found.push(format!("lost {keep}"));
        }
    }
    found
}

#[test]
fn fourth_generator_corpus_floors_as_python() {
    let spec = spec("fourth");
    let digest = floor_corpus(
        "fourth",
        number(&spec["seed"]),
        number(&spec["count"]),
        fourth_leaks,
    );
    assert_eq!(digest, text(&spec["output_sha256"]));
}

#[test]
fn fourth_generator_extra_seeds_leak_nothing() {
    let spec = spec("fourth");
    let seeds = spec["extra_seeds"].as_array().unwrap();
    assert_eq!(seeds.len(), 4);
    for seed in seeds {
        floor_corpus(
            "fourth",
            number(seed),
            number(&spec["extra_count"]),
            fourth_leaks,
        );
    }
}

/// Numeric names inside the contract; `twofa` alone and the count-shaped
/// `token_ttl`, `session_token_max` and `pin_len` are metadata or out of scope.
const FIFTH_NUMERIC: &[&str] = &[
    "pin",
    "otp",
    "wallet_pin",
    "device_pin",
    "user_pin",
    "login_otp",
    "PIN",
    "OTP",
    "pin_code",
    "otp_code",
    "totp",
    "totp_code",
    "mfa_code",
    "2fa_code",
    "verification_code",
    "pincode",
    "token_num",
    "api_key_num",
];

fn fifth_leaks(case: &Value, output: &str) -> Vec<String> {
    let input = text(&case["input"]);
    let secrets = pairs(&case["secrets"]);
    let planted: Vec<&str> = secrets.iter().map(|(secret, _)| *secret).collect();
    // Zero-width noise inside a key and prose-only provider errors are out of scope.
    let in_scope = secrets
        .iter()
        .filter(|(_, kind)| {
            !(kind.contains("~noise")
                || (kind.ends_with("-oos") && !kind.starts_with("path_token")))
        })
        .map(|(secret, _)| *secret);
    let mut found = leaked_secrets(input, in_scope, &planted, &decoded_views(output));
    for (number, kind) in pairs(&case["numeric"]) {
        let name = kind.split_once(':').map_or("", |(_, name)| name);
        if !kind.contains("~noise")
            && FIFTH_NUMERIC.contains(&name)
            && bordered_matches(output, number, |c| c.is_ascii_hexdigit()) > 0
        {
            found.push(number.to_owned());
        }
    }
    found
}

#[test]
fn fifth_generator_corpus_floors_as_python() {
    let spec = spec("fifth");
    let digest = floor_corpus(
        "fifth",
        number(&spec["seed"]),
        number(&spec["count"]),
        fifth_leaks,
    );
    assert_eq!(digest, text(&spec["output_sha256"]));
}

#[test]
fn fifth_generator_extra_seeds_leak_nothing() {
    let spec = spec("fifth");
    let seeds = spec["extra_seeds"].as_array().unwrap();
    assert_eq!(seeds.len(), 3);
    for seed in seeds {
        floor_corpus(
            "fifth",
            number(seed),
            number(&spec["extra_count"]),
            fifth_leaks,
        );
    }
}

fn sixth_leaks(case: &Value, output: &str) -> Vec<String> {
    let input = text(&case["input"]);
    let secrets = pairs(&case["secrets"]);
    let planted: Vec<&str> = secrets.iter().map(|(secret, _)| *secret).collect();
    // Every class is inside the contract.
    let mut found = leaked_secrets(
        input,
        planted.iter().copied(),
        &planted,
        &decoded_views(output),
    );
    for (number, _) in pairs(&case["numeric"]) {
        // A code that also appears as readable data (a token count) is ambiguous.
        let digit = |c: char| c.is_ascii_digit();
        if bordered_matches(input, number, digit) == 1
            && bordered_matches(output, number, digit) > 0
        {
            found.push(number.to_owned());
        }
    }
    found
}

#[test]
fn sixth_generator_corpus_floors_as_python() {
    let spec = spec("sixth");
    let digest = floor_corpus(
        "sixth",
        number(&spec["seed"]),
        number(&spec["count"]),
        sixth_leaks,
    );
    assert_eq!(digest, text(&spec["output_sha256"]));
}

#[test]
fn sixth_generator_extra_seeds_leak_nothing() {
    let spec = spec("sixth");
    let seeds = spec["extra_seeds"].as_array().unwrap();
    assert_eq!(seeds.len(), 2);
    for seed in seeds {
        floor_corpus(
            "sixth",
            number(seed),
            number(&spec["extra_count"]),
            sixth_leaks,
        );
    }
}

#[test]
fn independent_generator_corpus_floors_as_python() {
    let spec = spec("independent");
    let digest = floor_corpus(
        "independent",
        number(&spec["seed"]),
        number(&spec["count"]),
        |case, output| {
            let mut found = Vec::new();
            // The raw output only: this generator plants no encoded secrets.
            if sentinel_leaked(text(&case["sentinel"]), &[units(output)]) {
                found.push("secret".to_owned());
            }
            for preserved in strings(&case["preserved"]) {
                if !output.contains(preserved) {
                    found.push(format!("lost {preserved}"));
                }
            }
            found
        },
    );
    assert_eq!(digest, text(&spec["output_sha256"]));
}

#[test]
fn decoded_views_follow_the_web_suites() {
    let decode = |s: &str| String::from_utf16(&entities_decoded(&units(s))).unwrap();
    assert_eq!(
        decode("&#65;&#x42;&lt;&amp;&nope;&#12345678;&#x;"),
        "AB<&&nope;&#12345678;&#x;"
    );
    assert_eq!(decode("&&#58;"), "&:");
    assert_eq!(decode("&#1114112;"), "&#1114112;");
    let pct = |s: &str| String::from_utf16(&percent_decoded(&units(s))).unwrap();
    assert_eq!(pct("%2541%41%4g%"), "%41A%4g%");
    assert_eq!(decoded_views("a%2541").len(), 16);
    assert!(decoded_views("x%2541").iter().any(|v| v == &units("xA")));
    assert_eq!(
        bordered_matches("g1234 51234 1234b 12345", "1234", |c| c.is_ascii_hexdigit()),
        1
    );
    assert_eq!(
        bordered_matches("1234 x1234 12345", "1234", |c| c.is_ascii_digit()),
        2
    );
    // A secret fragment another planted secret or the surrounding input carries is no leak.
    let views = vec![units("token=[redacted] AAAAzzzz")];
    assert_eq!(
        leaked_secrets(
            "t=AAAAzzzz",
            ["AAAAzzzz"].into_iter(),
            &["AAAAzzzz"],
            &views
        ),
        ["AAAAzzzz"]
    );
    assert!(leaked_secrets(
        "t=AAAAzzzz zzzz",
        ["AAAAzzzz"].into_iter(),
        &["AAAAzzzz", "AAAA"],
        &[units("AAAA zzzz")]
    )
    .is_empty());
}
