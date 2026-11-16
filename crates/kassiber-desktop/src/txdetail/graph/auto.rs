//! The automatic lookup from the transaction's wallet's own backend (owner decision: "Auto for
//! the wallet's backend"; an improvement over the web, which only offers "Look up on-chain").
//!
//! Opening a transaction whose stored graph lacks spent-output amounts asks the daemon for
//! `ui.transactions.graph` with `lookup: "wallet_backend"`. The daemon contacts only the backend
//! that wallet syncs from (never the book's default or another configured backend), and refuses
//! before any request while offline mode or `KASSIBER_NO_EGRESS` is on, when the user turned
//! "Fill in transaction details from the wallet's backend" off, or when the wallet has no synced
//! on-chain backend (docs/reference/privacy-and-security.md). A refusal for a wallet without such
//! a backend (or the setting off, or an older daemon) falls back to the explicit lookup button.
//!
//! The request starts only for the transaction open in the panel, once per transaction (the
//! cache keeps its answer, a refusal included), never for a transaction the user already left,
//! and never in headless tooling unless `KB_GRAPH_AUTO_LOOKUP=1` opts a scripted run in (only
//! against a loopback stub). The daemon handles requests in order and cannot recall one already
//! sent; navigating away only stops new ones, and a late answer just fills that transaction's
//! cache.

use std::path::{Path, PathBuf};

use super::data::Payload;
use super::{Entry, GraphUi, model};

/// The daemon's refusal codes (crates/journals/src/transaction_graph/public.rs).
const OFFLINE: &str = "wallet_backend_lookup_offline";
const UNAVAILABLE: &str = "wallet_backend_lookup_unavailable";
const DISABLED: &str = "wallet_backend_lookup_disabled";

/// What the automatic lookup did for one transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum AutoState {
    /// Not asked (yet, or not for this transaction).
    NotAsked,
    Running,
    /// The wallet's backend filled the graph in; it is shown.
    Filled,
    /// Offline mode or `KASSIBER_NO_EGRESS` refused it before anything was sent.
    Offline,
    /// The wallet's backend did not answer (or answered incompletely).
    Unreachable,
    /// No automatic lookup for this row: the wallet has no synced on-chain backend, the setting is
    /// off, or the daemon does not know the request. The explicit lookup stays available.
    Unavailable,
}

/// Codes the daemon returns when a lookup it attempted stopped.
fn stopped(p: &Payload) -> bool {
    p.warnings.iter().any(|w| {
        w.code.contains("reference_lookup_")
            || matches!(
                w.code.as_str(),
                "invalid_reference_scope"
                    | "book_network_mismatch"
                    | "local_reference_conflict"
                    | "graph_lookup_failed"
                    | "graph_lookup_warning"
            )
    })
}

pub fn classify(entry: Option<&Entry>) -> AutoState {
    match entry {
        None => AutoState::NotAsked,
        Some(Entry::Loading) => AutoState::Running,
        // An older daemon refuses the unknown `lookup` field: keep the explicit path.
        Some(Entry::Failed(_)) => AutoState::Unavailable,
        Some(Entry::Ready(p)) if p.has_warning(OFFLINE) => AutoState::Offline,
        Some(Entry::Ready(p)) if p.has_warning(UNAVAILABLE) || p.has_warning(DISABLED) => {
            AutoState::Unavailable
        }
        Some(Entry::Ready(p)) if stopped(p) => AutoState::Unreachable,
        Some(Entry::Ready(_)) => AutoState::Filled,
    }
}

/// Headless runs (snapshots, scripts, pilot-eval, tests) never start the lookup unless a scripted
/// run against a loopback stub opts in.
pub fn allowed_here() -> bool {
    rune::tooling::headless_run().is_none()
        || std::env::var_os("KB_GRAPH_AUTO_LOOKUP").is_some_and(|v| v == "1")
}

/// Whether to ask now for `reference`: the setting was read and is on, it is the transaction open in the
/// panel, not already asked (deduped, a refusal included), not explicitly approved for the full
/// lookup, and its stored graph lacks something a lookup could add.
pub fn wanted(g: &GraphUi, reference: &str, open: Option<&str>, allowed: bool) -> bool {
    if !allowed
        || g.auto_source.is_none()
        || !g.auto_enabled
        || reference.is_empty()
        || open != Some(reference)
        || g.public_for(reference)
        || g.entry_auto(reference).is_some()
    {
        return false;
    }
    match g.entry(reference, false) {
        Some(Entry::Ready(p)) => !p.is_liquid() && model::lookup_can_add(p),
        _ => false,
    }
}

/// Answers that should be asked again once offline mode turns off.
pub fn offline_refusal(entry: &Entry) -> bool {
    matches!(entry, Entry::Ready(p) if p.has_warning(OFFLINE))
}

// ---- The preference (Settings → Privacy) -------------------------------------------------------

/// The file the daemon reads (crates/setup/src/wallet_backend_lookup.rs): the demo copy's own
/// file while the demo is open (`launch_env`), else `KASSIBER_WALLET_BACKEND_LOOKUP_FILE`, else
/// `<default state root>/config/wallet-backend-lookup.json`. None only in tooling runs without a
/// named file (and without a home directory), where the switch is shown read-only.
pub fn preference_path_for(launch_env: &[(String, String)]) -> Option<PathBuf> {
    crate::settings::prefpath::path(crate::settings::prefpath::WALLET_BACKEND_LOOKUP, launch_env)
}

/// The path for the running daemon's launch.
pub fn preference_path(app: &crate::app::App) -> Option<PathBuf> {
    preference_path_for(&app.daemon.launch.env)
}

/// The daemon's reading: a missing file is on; anything unreadable or of another schema is off.
pub fn enabled_at(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return true;
    };
    if !meta.file_type().is_file() || meta.len() > 4096 {
        return false;
    }
    let Ok(raw) = std::fs::read(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return false;
    };
    v["schema_version"].as_u64() == Some(1) && v["enabled"] == true
}

/// The preference for a daemon started with `launch_env` (the default, on, without a file).
pub fn enabled_for(launch_env: &[(String, String)]) -> bool {
    preference_path_for(launch_env).is_none_or(|p| enabled_at(&p))
}

/// The launch's override, which changes when the demo opens or closes.
fn launch_value(app: &crate::app::App) -> Option<String> {
    app.daemon
        .launch
        .env
        .iter()
        .find(|(k, _)| k == crate::settings::prefpath::WALLET_BACKEND_LOOKUP.env)
        .map(|(_, v)| v.clone())
}

/// Every frame: re-reads the file when the daemon's launch changed which file it reads (the
/// demo opened or closed) or the graph state was reset. True when the switch changed.
pub fn sync(app: &mut crate::app::App) -> bool {
    let source = launch_value(app);
    if app.graph.auto_source.as_ref() == Some(&source) {
        return false;
    }
    let on = enabled_for(&app.daemon.launch.env);
    app.graph.auto_source = Some(source);
    let changed = app.graph.auto_enabled != on;
    app.graph.auto_enabled = on;
    changed
}

/// Writes `{"enabled": …, "schema_version": 1}` owner-only, replaced atomically.
pub fn write_at(path: &Path, enabled: bool) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("The preference has no parent directory.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("wallet-backend-lookup.json");
    let tmp = parent.join(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
    let written = f
        .write_all(format!("{{\"enabled\": {enabled}, \"schema_version\": 1}}\n").as_bytes())
        .and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// The Settings → Privacy switch. Turning it on forgets earlier "turned off" refusals, so the open
/// transaction asks again; turning it off only stops new requests.
pub fn set_enabled(app: &mut crate::app::App, on: bool) {
    let Some(path) = preference_path(app) else {
        return;
    };
    match write_at(&path, on) {
        Ok(()) => {
            app.graph.auto_enabled = on;
            if on {
                app.graph.cache.retain(|k, e| {
                    !(k.ends_with("|wallet")
                        && matches!(e, Entry::Ready(p) if p.has_warning(DISABLED)))
                });
                if let Some(id) = super::open_id(app).map(str::to_owned) {
                    super::maybe_auto(app, &id);
                }
            }
        }
        Err(_) => app.toast(
            crate::app::ToastTone::Error,
            setting_text("failed"),
            None,
            None,
        ),
    }
}

/// The Settings → Privacy row's copy (the web has none: the feature is Rune's).
pub fn setting_text(key: &str) -> &'static str {
    use crate::i18n::{Lang, lang};
    let (en, de) = match key {
        "heading" => ("Transaction details", "Transaktionsdetails"),
        "label" => (
            "Fill in transaction details from the wallet's backend",
            "Transaktionsdetails vom Backend der Wallet ergänzen",
        ),
        "on" => (
            "Opening a transaction whose spent amounts are not stored asks the backend that wallet already syncs from for them (the transaction id and the ids of the transactions it spends). No other server is asked. Offline mode turns it off.",
            "Beim Öffnen einer Transaktion, deren ausgegebene Beträge nicht gespeichert sind, wird das Backend gefragt, von dem diese Wallet ohnehin synchronisiert (Transaktions-ID und die IDs der ausgegebenen Transaktionen). Kein anderer Server wird gefragt. Der Offline-Modus schaltet es ab.",
        ),
        "off" => (
            "Off: opening a transaction sends nothing. “Look up on-chain” still asks on request.",
            "Aus: Beim Öffnen einer Transaktion wird nichts gesendet. „On-Chain abfragen“ fragt weiterhin auf Anfrage.",
        ),
        "fixed" => (
            "On by default. This build cannot change it here because no preference file is configured.",
            "Standardmäßig an. Ohne konfigurierte Einstellungsdatei lässt es sich hier nicht ändern.",
        ),
        "failed" => (
            "The setting could not be saved.",
            "Die Einstellung konnte nicht gespeichert werden.",
        ),
        _ => ("", ""),
    };
    if lang() == Lang::De { de } else { en }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use serde_json::json;

    use super::*;

    const TX: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

    fn payload(missing: bool, warnings: &[&str]) -> Rc<Payload> {
        let input = if missing {
            json!({"id": "in-0", "index": 0, "valueState": "missing"})
        } else {
            json!({"id": "in-0", "index": 0, "valueSats": 60000, "valueState": "known"})
        };
        let warnings: Vec<_> = warnings
            .iter()
            .map(|c| json!({"code": c, "level": "info", "message": "m"}))
            .collect();
        Rc::new(
            Payload::parse(&json!({
                "transaction": {"id": "t1", "chain": "bitcoin", "network": "main", "txid": TX},
                "supportLevel": "partial", "warnings": warnings,
                "inputs": [input], "outputs": [{"id": "out-0", "index": 0, "valueSats": 59000, "valueState": "known"}],
            }))
            .unwrap(),
        )
    }

    fn ui_with_local(p: Rc<Payload>) -> GraphUi {
        let mut g = GraphUi {
            auto_source: Some(None),
            ..GraphUi::default()
        };
        g.set("t1", "local", Entry::Ready(p));
        g
    }

    #[test]
    fn only_the_open_transaction_with_something_missing_is_asked() {
        let g = ui_with_local(payload(true, &[]));
        assert!(wanted(&g, "t1", Some("t1"), true));
        // Headless (or a tooling run that did not opt in), another transaction open, none open.
        assert!(!wanted(&g, "t1", Some("t1"), false));
        assert!(!wanted(&g, "t1", Some("t2"), true));
        assert!(!wanted(&g, "t1", None, true));
        // Every amount already stored: nothing to fill in.
        assert!(!wanted(
            &ui_with_local(payload(false, &[])),
            "t1",
            Some("t1"),
            true
        ));
        // The local read has not answered yet.
        assert!(!wanted(&GraphUi::default(), "t1", Some("t1"), true));
    }

    #[test]
    fn a_transaction_is_asked_once() {
        let mut g = ui_with_local(payload(true, &[]));
        assert!(g.begin_auto("t1").is_some());
        assert_eq!(classify(g.entry_auto("t1")), AutoState::Running);
        assert!(!wanted(&g, "t1", Some("t1"), true));
        assert!(g.begin_auto("t1").is_none(), "deduped while running");
        g.set("t1", "wallet", Entry::Ready(payload(true, &[UNAVAILABLE])));
        assert!(
            !wanted(&g, "t1", Some("t1"), true),
            "a refusal is remembered too"
        );
        assert!(g.begin_auto("t1").is_none());
    }

    #[test]
    fn leaving_a_transaction_cancels_what_has_not_started() {
        // The local read answers after the user moved on: nothing is asked for it.
        let g = ui_with_local(payload(true, &[]));
        assert!(!wanted(&g, "t1", Some("t2"), true));
        assert_eq!(classify(g.entry_auto("t1")), AutoState::NotAsked);
    }

    #[test]
    fn the_setting_off_asks_nothing() {
        let mut g = ui_with_local(payload(true, &[]));
        g.auto_enabled = false;
        assert!(!wanted(&g, "t1", Some("t1"), true));
    }

    #[test]
    fn nothing_is_asked_before_the_preference_is_read() {
        let mut g = ui_with_local(payload(true, &[]));
        g.auto_source = None;
        assert!(
            !wanted(&g, "t1", Some("t1"), true),
            "a reset graph waits for `sync`"
        );
    }

    #[test]
    fn the_demo_launch_reads_its_own_file() {
        let dir = std::env::temp_dir().join(format!("kb-autolookup-demo-{}", std::process::id()));
        let demo = dir.join("demo/config/wallet-backend-lookup.json");
        write_at(&demo, false).unwrap();
        let env = vec![(
            crate::settings::prefpath::WALLET_BACKEND_LOOKUP
                .env
                .to_owned(),
            demo.to_string_lossy().into_owned(),
        )];
        assert_eq!(preference_path_for(&env), Some(demo.clone()));
        assert!(!enabled_for(&env), "the demo copy's off wins");
        write_at(&demo, true).unwrap();
        assert!(enabled_for(&env));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_approved_explicit_lookup_is_not_doubled() {
        let mut g = ui_with_local(payload(true, &[]));
        g.approve("t1");
        assert!(!wanted(&g, "t1", Some("t1"), true));
    }

    #[test]
    fn answers_classify_by_the_daemons_codes() {
        let ready = |w: &[&str]| Entry::Ready(payload(true, w));
        assert_eq!(classify(None), AutoState::NotAsked);
        assert_eq!(classify(Some(&Entry::Loading)), AutoState::Running);
        assert_eq!(
            classify(Some(&Entry::Failed(
                "ui.transactions.graph received unsupported fields".into()
            ))),
            AutoState::Unavailable
        );
        assert_eq!(classify(Some(&ready(&[OFFLINE]))), AutoState::Offline);
        assert_eq!(
            classify(Some(&ready(&[UNAVAILABLE]))),
            AutoState::Unavailable
        );
        assert_eq!(classify(Some(&ready(&[DISABLED]))), AutoState::Unavailable);
        assert_eq!(
            classify(Some(&ready(&["bitcoin_reference_lookup_failed_explorer"]))),
            AutoState::Unreachable
        );
        assert_eq!(
            classify(Some(&Entry::Ready(payload(false, &[])))),
            AutoState::Filled
        );
        assert!(offline_refusal(&ready(&[OFFLINE])) && !offline_refusal(&ready(&[UNAVAILABLE])));
    }

    #[test]
    fn the_preference_reads_like_the_daemon() {
        let dir =
            std::env::temp_dir().join(format!("kb-autolookup-{}-{}", std::process::id(), line!()));
        let path = dir.join("config/wallet-backend-lookup.json");
        assert!(enabled_at(&path), "missing is on");
        write_at(&path, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"enabled\": false, \"schema_version\": 1}\n"
        );
        assert!(!enabled_at(&path));
        write_at(&path, true).unwrap();
        assert!(enabled_at(&path));
        std::fs::write(&path, "{").unwrap();
        assert!(!enabled_at(&path), "damaged is off");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            write_at(&path, true).unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_setting_copy_has_both_languages() {
        use crate::i18n::{Lang, set_lang};
        for key in ["heading", "label", "on", "off", "fixed", "failed"] {
            set_lang(Lang::En);
            let en = setting_text(key);
            set_lang(Lang::De);
            let de = setting_text(key);
            set_lang(Lang::En);
            assert!(!en.is_empty() && !de.is_empty() && en != de, "{key}");
        }
    }
}
