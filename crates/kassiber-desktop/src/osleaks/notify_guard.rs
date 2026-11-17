//! OS notifications leave Kassiber's process: macOS keeps them in Notification Center, shows
//! them on the lock screen and mirrors them to other devices. So every notification is the title
//! "Kassiber" and one of `native::notify::BODIES`, which name no amount, wallet, label, txid,
//! address, server, path or error (and so respect "Hide amounts" whatever it is set to). These
//! tests scan every body Kassiber can build, drive the real offer path, and make sure no other
//! module talks to the platform's notification services.

use crate::native::notify::{BODIES, TITLE};
use crate::settings::effects::{self, Category};

/// Wallet names from the demo book: none may ever appear in a notification.
const WALLET_NAMES: [&str; 6] = [
    "Exchange self custody",
    "Strike",
    "Regtest Cold Storage",
    "Regtest Treasury",
    "Regtest Merchant Wallet",
    "Regtest Spending Wallet",
];

/// Why `s` could leak something, if it could.
fn leak(s: &str) -> Option<String> {
    if s.chars().any(|c| c.is_ascii_digit()) {
        return Some("a digit (amount, count of coins, date, id)".into());
    }
    for unit in ["BTC", "sat", "msat", "₿", "€", "$", "EUR", "USD", "L-BTC"] {
        if s.split(|c: char| !c.is_alphanumeric() && !"₿€$-".contains(c))
            .any(|w| w.eq_ignore_ascii_case(unit))
        {
            return Some(format!("a unit ({unit})"));
        }
    }
    for w in s
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
    {
        let lower = w.to_ascii_lowercase();
        if [
            "bc1", "tb1", "bcrt1", "lq1", "ex1", "lnbc", "lntb", "lnbcrt", "xpub", "zpub", "tpub",
        ]
        .iter()
        .any(|p| lower.starts_with(p) && w.len() > 8)
        {
            return Some(format!("an address/invoice/key-like word ({w})"));
        }
        if w.len() >= 16 && w.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("a txid/hash-like word ({w})"));
        }
        if w.len() >= 25 && w.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(format!("an encoded-value-like word ({w})"));
        }
    }
    if s.contains("{{") || s.contains('"') || s.contains('/') || s.contains('\\') || s.contains(':')
    {
        return Some("a placeholder, quote, path or URL character".into());
    }
    WALLET_NAMES
        .iter()
        .find(|n| s.contains(*n))
        .map(|n| format!("a wallet name ({n})"))
}

#[test]
fn the_scanner_catches_what_it_should() {
    for bad in [
        "Received 0.5 BTC",
        "Sent 50000 sats",
        "Regtest Treasury synced",
        "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq arrived",
        "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b confirmed",
        "Error at /Users/u/x",
        "{{count}} items",
    ] {
        assert!(leak(bad).is_some(), "missed: {bad}");
    }
    assert_eq!(leak("Background work has finished."), None);
}

#[test]
fn every_notification_text_is_generic() {
    assert_eq!(leak(TITLE), None);
    for b in BODIES {
        assert_eq!(leak(b), None, "{b}");
    }
    for c in [Category::NeedsAttention, Category::JobFinished] {
        for de in [false, true] {
            let b = effects::body(c, de);
            assert!(BODIES.contains(&b), "{b} is not an allowlisted body");
        }
    }
    use crate::native::notify::{Category as N, body};
    for c in [
        N::NeedsAttention,
        N::JobFinished,
        N::NewActivity,
        N::EvidenceChanged,
    ] {
        for de in [false, true] {
            assert!(BODIES.contains(&body(c, de)));
        }
    }
}

/// The real offer path, whatever the privacy mode: only allowlisted bodies are delivered.
#[test]
fn offered_notifications_carry_only_fixed_bodies() {
    for privacy in [
        crate::app::Privacy::Normal,
        crate::app::Privacy::HideAmounts,
    ] {
        let mut app = crate::app::App::new();
        app.prefs.privacy = privacy;
        app.settings.prefs.desktop_notifications = true;
        effects::window_focus(&mut app, false);
        let reply = |error: Option<&str>| crate::app::Reply {
            kind: "ui.freshness.run".into(),
            data: serde_json::json!({"wallets": [{"label": "Regtest Treasury", "balance_msat": 50_000_000_000u64}]}),
            error: error.map(str::to_owned),
            error_detail: serde_json::Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        effects::on_reply(
            &mut app,
            "app.freshness_run",
            &reply(Some(
                "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq unreachable",
            )),
        );
        app.clock += 3600.0;
        effects::on_reply(&mut app, "app.journals_process", &reply(None));
        let delivered = &app.settings.fx.os.delivered;
        assert_eq!(delivered.len(), 2);
        for d in delivered {
            assert!(BODIES.contains(d) && leak(d).is_none(), "{d}");
        }
    }
}

/// No module but `native::notify` reaches the platform's notification services, and only
/// `settings::effects` asks it to show something.
#[test]
fn no_other_module_posts_notifications() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root.clone()];
    let mut offenders = vec![];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") || p.ends_with("osleaks/notify_guard.rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let src = std::fs::read_to_string(&p).unwrap();
            let platform = [
                "rune_notify::",
                "display notification",
                "notify-send",
                "org.freedesktop.Notifications",
                "UNUserNotificationCenter",
                "NSUserNotification",
            ]
            .iter()
            .any(|n| src.contains(n));
            if platform && rel != "native/notify.rs" {
                offenders.push(format!("{rel}: talks to the notification service"));
            }
            if src.contains("notify::show(")
                && rel != "settings/effects.rs"
                && rel != "native/notify.rs"
            {
                offenders.push(format!(
                    "{rel}: shows a notification outside settings::effects"
                ));
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}
