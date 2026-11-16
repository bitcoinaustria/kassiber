//! Release checks that gate the desktop as Rust tests, replacing the Python eval tools
//! (`tools/a11y-check.py`, `tools/find-key.py` and `tools/pilot-eval.sh`'s missing-key report),
//! which AGENTS.md keeps out of the product gate.
//!
//! - `every_referenced_translation_key_exists`: static, always on. Every string literal in
//!   `src/**/*.rs` shaped like an i18n key of a loaded namespace (`"nav:book.overview"`) must
//!   exist in the English and the German bundle (directly, as a plural `_one`/`_other`, or as
//!   the prefix of a nested group). Catches keys the web renamed or removed.
//! - `scenes_are_accessible_and_translated`: renders every scene headless in English and German
//!   and checks the accessibility tree and the element list the way `a11y-check.py` did
//!   (unnamed interactive nodes, focusable elements without a name, duplicate app keys), plus
//!   every lookup that fell back to English or to the key at run time. It drives the real
//!   daemon, so it is `#[ignore]`d and runs only where a disposable book is configured
//!   (`packaging/ci/rune-checks.sh` sets one up): it refuses to start without
//!   `KASSIBER_DEMO_DATA_ROOT` and the isolated preference files, so it can never open the
//!   user's own books.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The locale bundles `src/i18n.rs` embeds (keep the two paths together when they move).
fn locales_dir() -> PathBuf {
    crate_dir().join("../../assets/i18n/locales")
}

/// The namespaces `src/i18n.rs` loads, read from its `bundles!("en": …)` list so the two never
/// drift.
fn namespaces() -> Vec<String> {
    let src = std::fs::read_to_string(crate_dir().join("src/i18n.rs")).unwrap();
    let line = src
        .lines()
        .find(|l| l.contains("bundles!(\"en\":"))
        .expect("the English bundle list in src/i18n.rs");
    let list = &line[line.find("\"en\":").unwrap() + 5..];
    list.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .filter(|s| !s.is_empty() && !s.contains(')'))
        .collect()
}

fn flatten(prefix: &str, v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => m.iter().for_each(|(k, v)| {
            flatten(
                &if prefix.ends_with(':') {
                    format!("{prefix}{k}")
                } else {
                    format!("{prefix}.{k}")
                },
                v,
                out,
            )
        }),
        Value::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, v)| flatten(&format!("{prefix}.{i}"), v, out)),
        _ => {
            out.insert(prefix.to_owned());
        }
    }
}

fn bundle(lang: &str, namespaces: &[String]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for ns in namespaces {
        let path = locales_dir().join(lang).join(format!("{ns}.json"));
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
        )
        .unwrap();
        flatten(&format!("{ns}:"), &v, &mut keys);
    }
    keys
}

fn resolves(keys: &BTreeSet<String>, key: &str) -> bool {
    let group = format!("{key}.");
    keys.contains(key)
        || ["_one", "_other", "_zero"]
            .iter()
            .any(|s| keys.contains(&format!("{key}{s}")))
        || keys
            .range(group.clone()..)
            .next()
            .is_some_and(|k| k.starts_with(&group))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The ordinary (non-raw) string literals of a Rust source, with their line numbers. Line
/// comments are skipped; raw strings are skipped whole (the icon bodies live in them).
fn literals(src: &str) -> Vec<(usize, String)> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < b.len() {
        match b[i] {
            b'\n' => line += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'r' if matches!(b.get(i + 1), Some(b'"' | b'#'))
                && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) =>
            {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                if b.get(i + 1 + hashes) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    if let Some(end) = src[i + 2 + hashes..].find(&close) {
                        line += src[i..i + 2 + hashes + end].matches('\n').count();
                        i += 2 + hashes + end + close.len();
                        continue;
                    }
                }
            }
            // A char literal holding a quote: `'"'`.
            b'\'' if b.get(i + 1) == Some(&b'"') && b.get(i + 2) == Some(&b'\'') => {
                i += 3;
                continue;
            }
            b'"' => {
                let (start, start_line) = (i + 1, line);
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    } else if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                out.push((start_line, src[start..i.min(b.len())].to_owned()));
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// `ns:a.b_c` with a loaded namespace and a plain dotted path (keys built at run time, such as
/// `format!("review:cards.{id}")`, are not literals and are left to the scene check).
fn as_key<'a>(lit: &'a str, namespaces: &[String]) -> Option<&'a str> {
    let (ns, path) = lit.split_once(':')?;
    let plain = !path.is_empty()
        && path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
        && !path.ends_with('.');
    (plain && namespaces.iter().any(|n| n == ns)).then_some(lit)
}

#[test]
fn every_referenced_translation_key_exists() {
    let namespaces = namespaces();
    assert!(
        namespaces.len() >= 16,
        "namespaces from src/i18n.rs: {namespaces:?}"
    );
    let (en, de) = (bundle("en", &namespaces), bundle("de", &namespaces));
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src"), &mut files);
    files.sort();
    let mut referenced = 0;
    let mut missing: BTreeMap<String, (bool, bool, String)> = BTreeMap::new();
    for file in &files {
        if file.ends_with("src/checks.rs") {
            continue;
        }
        let src = std::fs::read_to_string(file).unwrap();
        for (line, lit) in literals(&src) {
            let Some(key) = as_key(&lit, &namespaces) else {
                continue;
            };
            referenced += 1;
            let (in_en, in_de) = (resolves(&en, key), resolves(&de, key));
            if !(in_en && in_de) {
                let at = format!(
                    "{}:{line}",
                    file.strip_prefix(crate_dir()).unwrap_or(file).display()
                );
                missing.entry(key.to_owned()).or_insert((in_en, in_de, at));
            }
        }
    }
    assert!(
        referenced > 500,
        "found only {referenced} key literals: the scanner is broken"
    );
    let report: Vec<String> = missing
        .iter()
        .map(|(k, (en, de, at))| {
            format!(
                "{k}  ({}{}) at {at}",
                if *en { "" } else { "missing in en " },
                if *de { "" } else { "missing in de" }
            )
        })
        .collect();
    assert!(
        report.is_empty(),
        "{} of {referenced} translation key reference(s) not in the bundles:\n{}",
        report.len(),
        report.join("\n")
    );
}

#[test]
fn the_literal_scanner_reads_rust_strings() {
    let src = "let a = t(\"nav:book.overview\"); // \"chrome:in.comment\"\nlet b = r#\"common:raw\"#; let c = '\"'; tr(\"settings:x.y\", &[]);";
    let lits: Vec<String> = literals(src).into_iter().map(|(_, s)| s).collect();
    assert_eq!(lits, ["nav:book.overview", "settings:x.y"]);
    let ns = vec!["nav".to_owned()];
    assert_eq!(as_key("nav:book.overview", &ns), Some("nav:book.overview"));
    assert_eq!(as_key("nav:", &ns), None);
    assert_eq!(as_key("nav:book.", &ns), None);
    assert_eq!(as_key("http://x", &ns), None);
    assert_eq!(as_key("nav:has space", &ns), None);
    let keys: BTreeSet<String> = ["nav:a.b_one", "nav:a.b_other", "nav:g.x"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert!(
        resolves(&keys, "nav:a.b")
            && resolves(&keys, "nav:g")
            && !resolves(&keys, "nav:a.c")
            && !resolves(&keys, "nav:g.x.y")
    );
}

// --- accessibility -----------------------------------------------------------------------

const INTERACTIVE: [&str; 15] = [
    "button",
    "link",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "checkbox",
    "switch",
    "tab",
    "option",
    "combobox",
    "textbox",
    "textinput",
    "slider",
    "radio",
    "searchbox",
];

/// rune-kit's scroll-area parts, keyed per instance and scoped by path.
const KIT_SCROLL_PARTS: [&str; 2] = ["scroll-bar", "thumb-slot"];

fn unnamed(node: &Value, path: &str, out: &mut Vec<String>) {
    let role = node["role"].as_str().unwrap_or("").to_ascii_lowercase();
    let actions: Vec<&str> = node["actions"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let name = node["name"].as_str().unwrap_or("").trim();
    let here = format!("{path}/{}", node["key"].as_str().unwrap_or(&role));
    let interactive = INTERACTIVE.contains(&role.as_str())
        || actions.contains(&"click")
        || actions.contains(&"focus");
    if interactive && name.is_empty() {
        out.push(format!(
            "unnamed {} at {here} actions={actions:?}",
            if role.is_empty() { "?" } else { &role }
        ));
    }
    for child in node["children"].as_array().into_iter().flatten() {
        unnamed(child, &here, out);
    }
}

/// `a11y-check.py`'s three checks over one rendered frame.
fn a11y_findings(tree: &Value, elements: &[rune::ElementInfo]) -> Vec<String> {
    let mut problems = Vec::new();
    unnamed(tree, "", &mut problems);
    let named = |x: &rune::ElementInfo| {
        x.label.as_deref().is_some_and(|s| !s.trim().is_empty())
            || x.text.as_deref().is_some_and(|s| !s.trim().is_empty())
    };
    for e in elements.iter().filter(|e| e.focusable && !e.inert) {
        let prefix = format!("{}/", e.path);
        if !named(e)
            && !elements
                .iter()
                .any(|d| d.path.starts_with(&prefix) && named(d))
        {
            problems.push(format!("focusable without a name: {}", e.path));
        }
    }
    let mut seen: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in elements.iter().filter(|e| e.opacity != 0.0) {
        if let Some(k) = e.key.as_deref() {
            seen.entry(k).or_default().push(&e.path);
        }
    }
    for (k, paths) in seen {
        // rune-kit keys its parts with plain words and scopes them by path; app keys (with a
        // dash) are one namespace.
        if paths.len() > 1 && k.contains('-') && !KIT_SCROLL_PARTS.contains(&k) {
            problems.push(format!(
                "duplicate key {k:?} x{}: {} …",
                paths.len(),
                paths[0]
            ));
        }
    }
    problems
}

#[test]
fn a11y_findings_match_the_python_checker() {
    let tree = serde_json::json!({"role": "window", "children": [
        {"role": "button", "key": "ok", "name": "OK", "actions": ["click"]},
        {"role": "button", "key": "icon-only", "actions": ["click"]},
        {"role": "group", "actions": ["focus"]},
    ]});
    let found = a11y_findings(&tree, &[]);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].contains("unnamed button at /window/icon-only"),
        "{found:?}"
    );
    assert!(
        found[1].contains("unnamed group at /window/group"),
        "{found:?}"
    );
}

/// Scenes that write to the book (`tools/pilot-eval.sh` skips the same).
const WRITES: [&str; 1] = ["detail-acq-apply"];

#[test]
#[ignore = "drives the daemon over a disposable book: packaging/ci/rune-checks.sh"]
fn scenes_are_accessible_and_translated() {
    // Fail closed: never against the user's own state.
    for var in [
        "KASSIBER_BIN",
        "KASSIBER_DEMO_DATA_ROOT",
        "KASSIBER_DESKTOP_PREFS_FILE",
        "KASSIBER_OFFLINE_PREFERENCE_FILE",
        "KASSIBER_AGENT_ACCESS_FILE",
    ] {
        assert!(
            std::env::var_os(var).is_some_and(|v| !v.is_empty()),
            "{var} must point at disposable test state"
        );
    }
    let only: Option<Vec<String>> = std::env::var("RUNE_CHECK_SCENES")
        .ok()
        .map(|s| s.split(',').map(str::to_owned).collect());
    let mut findings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut missing: BTreeMap<String, String> = BTreeMap::new();
    let mut ran = 0;
    let mut sizes: Vec<usize> = Vec::new();
    let mut named_total = 0;
    for &(name, scene) in crate::all_scenes() {
        if WRITES.contains(&name) || only.as_ref().is_some_and(|o| !o.iter().any(|s| s == name)) {
            continue;
        }
        for lang in [crate::i18n::Lang::En, crate::i18n::Lang::De] {
            // Progress, so a scene that stalls is named in the log.
            eprintln!("scene check: {name} ({})", lang.code());
            // A `crash` scene's recovery state must not carry into the next render.
            crate::shell::crash::reset();
            let app = rune::App::new(rune::AppSpec::<crate::app::App> {
                fonts: crate::FONTS,
                families: (crate::sans_stack(), crate::MONO),
                setup: Some(crate::setup),
                poll: Some(crate::poll),
                on_key: Some(crate::on_key),
                ..rune::AppSpec::new(
                    "Kassiber",
                    (1440.0, 900.0),
                    crate::app::App::new,
                    crate::view,
                )
            });
            let mut rt = app.headless_runtime((1440.0, 900.0), rune::Env::new(true, 1.0));
            rt.state.prefs.lang = lang;
            scene(&mut rt.state);
            for _ in 0..4 {
                rt.frame(1.0 / 60.0);
            }
            let label = format!("{name} ({})", lang.code());
            let (tree, elements) = (rt.accessibility_json(), rt.elements());
            // Not vacuous: every render has a real tree. (A modal dialog or open menu takes the
            // page out of the tree, so a single render can be small; the totals are checked below.)
            let (nodes, named) = tree_size(&tree);
            // `crash-bare` is the recovery card drawn alone (the shell failed too), by design.
            let min_elements = if name == "crash-bare" { 10 } else { 20 };
            assert!(
                nodes >= 3 && elements.len() >= min_elements,
                "{label}: {nodes} a11y nodes, {} elements",
                elements.len()
            );
            sizes.push(nodes);
            named_total += named;
            let found = a11y_findings(&tree, &elements);
            if !found.is_empty() {
                findings.insert(label.clone(), found);
            }
            // The miss log is per thread and cumulative: attribute each key to the first scene.
            for key in crate::i18n::missing() {
                missing.entry(key).or_insert_with(|| label.clone());
            }
            ran += 1;
        }
    }
    assert!(ran > 0, "no scene ran");
    sizes.sort_unstable();
    eprintln!(
        "{ran} scene renders; a11y nodes per render: min {}, median {}, max {}",
        sizes[0],
        sizes[sizes.len() / 2],
        sizes[sizes.len() - 1]
    );
    assert!(
        sizes[sizes.len() / 2] >= 50,
        "median tree of {} nodes: the trees are not being built",
        sizes[sizes.len() / 2]
    );
    assert!(
        named_total >= 5 * ran,
        "only {named_total} named interactive nodes over {ran} renders"
    );
    let mut report = Vec::new();
    for (scene, list) in &findings {
        report.push(format!("{scene}:"));
        report.extend(list.iter().map(|f| format!("  - {f}")));
    }
    for (key, scene) in &missing {
        report.push(format!("missing translation {key} (first in {scene})"));
    }
    assert!(
        report.is_empty(),
        "{ran} scene render(s) checked; {} with a11y findings, {} missing translation(s):\n{}",
        findings.len(),
        missing.len(),
        report.join("\n")
    );
}

/// (nodes, named interactive nodes) of an accessibility tree.
fn tree_size(node: &Value) -> (usize, usize) {
    let role = node["role"].as_str().unwrap_or("");
    let named = node["name"].as_str().is_some_and(|n| !n.trim().is_empty());
    let mut size = (1, usize::from(named && INTERACTIVE.contains(&role)));
    for child in node["children"].as_array().into_iter().flatten() {
        let (n, i) = tree_size(child);
        size = (size.0 + n, size.1 + i);
    }
    size
}
