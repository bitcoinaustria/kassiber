//! Demo-book fixtures for headless scenes (`parity/fixtures.json`, the one file that names demo
//! rows). Scenes ask for a fixture by name instead of hardcoding an id, so replacing the demo
//! book is a one-file change (`tools/demo-fixtures.py resolve <book>` rewrites it).
//!
//! - `id("full")`: the fixture's row id; `field("linked_in", "txid")`: another stored column.
//! - `expand("…{fx:full}…")`: the parity harness's placeholder syntax, for scene arguments.
//! - `now()`: the scenes' fixed clock, a few days after the book's last transaction.
//! - `use_book(app, "coarse-review")` (tooling builds): a named variant of the demo book. The
//!   current book is copied to a temporary root, the variant's local CLI steps run there, and the
//!   daemon switches to it. The original book and the pristine copy are never written.

use std::sync::OnceLock;

use serde_json::Value;

static FIXTURES: &str = include_str!("../parity/fixtures.json");

fn cfg() -> &'static Value {
    static CFG: OnceLock<Value> = OnceLock::new();
    CFG.get_or_init(|| serde_json::from_str(FIXTURES).expect("parity/fixtures.json is valid JSON"))
}

/// A stored column of a fixture (`id`, `txid`, `occurred_at`, …). Panics on a typo, so a scene
/// can never silently open a row that doesn't exist.
pub fn field(name: &str, key: &str) -> &'static str {
    cfg()["fixtures"][name][key]
        .as_str()
        .unwrap_or_else(|| panic!("parity/fixtures.json has no fixtures.{name}.{key}"))
}

/// The fixture's row id.
pub fn id(name: &str) -> &'static str {
    field(name, "id")
}

/// Expands `{fx:name}` (the id) and `{fx:name.field}` in `s`; `saved` resolves
/// `{fx:saved.<key>}` (values kept by earlier `use_book` steps).
fn expand_with(s: &str, saved: &[(String, String)]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("{fx:") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 4..];
        let Some(end) = tail.find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let key = &tail[..end];
        let value = match key.split_once('.') {
            Some(("saved", k)) => saved
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("no saved value {k}")),
            Some((name, f)) => field(name, f).to_owned(),
            None => id(key).to_owned(),
        };
        out.push_str(&value);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Expands the parity harness's `{fx:…}` placeholders (scene arguments such as `--tx`).
pub fn expand(s: &str) -> String {
    expand_with(s, &[])
}

/// The scenes' clock (seconds since the epoch): `now` in the fixture file.
pub fn now() -> f64 {
    let s = cfg()["now"].as_str().expect("fixtures.json has now");
    crate::format::parse_ts(s)
        .unwrap_or_else(|| panic!("fixtures.json now is not an ISO timestamp: {s}"))
}

/// Switches to `root` and waits (up to 30 s) for the switch's outcome: the switch warms in the
/// background, and a scene waits for it.
#[cfg(feature = "tooling")]
fn switch_and_wait(a: &mut crate::app::App, root: std::path::PathBuf) -> Result<(), String> {
    let outcome: std::rc::Rc<std::cell::RefCell<Option<Result<(), String>>>> = Default::default();
    let sink = outcome.clone();
    crate::app::switch_root(a, Some(root), Vec::new(), move |_, r| {
        *sink.borrow_mut() = Some(r)
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while outcome.borrow().is_none() && std::time::Instant::now() < deadline {
        a.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    outcome
        .borrow_mut()
        .take()
        .unwrap_or_else(|| Err("the book switch did not finish".into()))
}

/// Switches the app to a named variant of the demo book (`books.<variant>` in the fixture file),
/// built on a temporary copy of the current book. Returns false (and leaves the app on its book)
/// when the variant could not be built; the scene then shows the plain book.
#[cfg(feature = "tooling")]
pub fn use_book(a: &mut crate::app::App, variant: &str) -> bool {
    match build_book(a, variant) {
        Ok(root) => match switch_and_wait(a, root) {
            Ok(()) => {
                crate::txdetail::pump_until(a, |a| {
                    !matches!(a.overview, crate::app::Load::Loading)
                        && a.read("ui.backends.settings.list").is_some()
                });
                true
            }
            Err(e) => {
                eprintln!("fixtures: switching to book variant {variant} failed: {e}");
                false
            }
        },
        Err(e) => {
            eprintln!("fixtures: building book variant {variant} failed: {e}");
            false
        }
    }
}

/// Shipped builds (no `tooling`) pass `main` an empty scene table, so no scene, and nothing
/// else, calls this; it keeps the always-compiled scene modules building. The book stays put.
#[cfg(not(feature = "tooling"))]
pub fn use_book(_: &mut crate::app::App, _: &str) -> bool {
    false
}

#[cfg(feature = "tooling")]
fn build_book(a: &crate::app::App, variant: &str) -> Result<std::path::PathBuf, String> {
    use std::process::Command;
    let book = &cfg()["books"][variant];
    let steps = book["steps"]
        .as_array()
        .ok_or_else(|| format!("no books.{variant} in parity/fixtures.json"))?;
    let src = a.daemon.launch.data_root.clone().ok_or(
        "the app runs on the default state root; scenes need an explicit disposable data root",
    )?;
    let base = std::env::temp_dir().join("kassiber-scene-books");
    sweep(&base);
    let root = base
        .join(format!("{variant}-{}", std::process::id()))
        .join("data");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    for f in ["kassiber.sqlite3", "kassiber.sqlite3-wal"] {
        let from = src.join(f);
        if from.exists() {
            std::fs::copy(&from, root.join(f))
                .map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    let bin = a.launch.binary.clone();
    let mut saved: Vec<(String, String)> = Vec::new();
    for step in steps {
        let args: Vec<String> = step["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|s| expand_with(s, &saved))
            .collect();
        // Local steps only; the no-egress switch makes that a guarantee, not a convention.
        let out = Command::new(&bin)
            .arg("--data-root")
            .arg(&root)
            .arg("--machine")
            .args(&args)
            .env("KASSIBER_NO_EGRESS", "1")
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "`kassiber {}` failed: {}",
                args.first().map(String::as_str).unwrap_or(""),
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            ));
        }
        if let Some(key) = step["save"].as_str() {
            let reply: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
            let v = reply["data"]["id"]
                .as_str()
                .ok_or_else(|| format!("step {key}: no data.id in the reply"))?;
            saved.push((key.to_owned(), v.to_owned()));
        }
    }
    Ok(root)
}

/// Removes variant copies left by headless runs that have exited (one copy per live process).
#[cfg(feature = "tooling")]
fn sweep(base: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let pid = name.rsplit('-').next().and_then(|p| p.parse::<u32>().ok());
        let alive = pid.is_some_and(|p| p == std::process::id() || process_alive(p));
        if !alive {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(all(feature = "tooling", unix))]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(all(feature = "tooling", not(unix)))]
fn process_alive(_pid: u32) -> bool {
    // No cheap check without a new dependency: keep the copy (the temp folder's own cleanup).
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_has_a_uuid_id() {
        let fx = cfg()["fixtures"].as_object().unwrap();
        assert!(fx.len() > 10);
        for (name, f) in fx {
            let id = f["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{name} has no id"));
            assert_eq!(id.len(), 36, "{name}: {id}");
            assert!(
                f["select"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("SELECT")),
                "{name} has no select"
            );
        }
    }

    #[test]
    fn expands_placeholders() {
        assert_eq!(
            expand("/overview?tx={fx:full}&tab=tax"),
            format!("/overview?tx={}&tab=tax", id("full"))
        );
        assert_eq!(expand("{fx:linked_in.txid}"), field("linked_in", "txid"));
        assert_eq!(
            expand_with("{fx:saved.source}", &[("source".into(), "abc".into())]),
            "abc"
        );
        assert_eq!(expand("no placeholder"), "no placeholder");
    }

    #[test]
    fn book_steps_reference_known_fixtures() {
        for (name, b) in cfg()["books"].as_object().unwrap() {
            let mut saved = Vec::new();
            for step in b["steps"].as_array().unwrap() {
                for a in step["args"].as_array().unwrap() {
                    // Panics on an unknown fixture name.
                    let _ = expand_with(a.as_str().unwrap(), &saved);
                }
                if let Some(k) = step["save"].as_str() {
                    saved.push((k.to_owned(), "x".to_owned()));
                }
            }
            assert!(!b["steps"].as_array().unwrap().is_empty(), "{name}");
        }
    }

    #[test]
    fn clock_is_after_the_book() {
        // 2026-11-15T12:00:00Z
        assert_eq!(now(), 1_794_744_000.0);
    }
}
