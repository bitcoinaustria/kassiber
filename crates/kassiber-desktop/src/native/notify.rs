//! OS notifications (src-tauri os_notifications.rs): the title is always "Kassiber" and the body
//! one of the fixed sentences, so no amount, name, address, txid, server, path or error text can
//! reach the system's notification store. Whether an event qualifies is decided by
//! `settings::effects` (opt-in, unfocused window, dedupe, cooldown); this module only shows it,
//! with the native backstop interval.
//!
//! - macOS: `UNUserNotificationCenter` (`rune-notify`) when running from a bundle with an
//!   identifier; the first post asks for permission. Unbundled (`cargo run`), the center refuses
//!   to post, so `osascript`'s `display notification` shows it.
//! - Windows: a toast under the installer-registered AppUserModelID (`rune_notify::adopt`,
//!   which writes nothing).
//! - Linux: the freedesktop notification service over D-Bus (`gdbus`, else `notify-send`).
//!
//! Clicks only bring Kassiber forward, as the operating system does: nothing is guessed from a
//! click (no sync, approval, file open or unlock).

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const TITLE: &str = "Kassiber";
/// The bundle / AppUserModelID identifier (`tauri.conf.json` `identifier`).
pub const APP_ID: &str = "at.bitcoinaustria.kassiber";
/// Backstop against a caller that skips its own cooldown.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);

/// Tauri's allowlisted categories. `settings::effects` raises the first two today.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    NeedsAttention,
    JobFinished,
    NewActivity,
    EvidenceChanged,
}

/// Every body Kassiber can put in an OS notification.
pub const BODIES: [&str; 6] = [
    "Background work needs your attention.",
    "Background work has finished.",
    "New activity in Kassiber.",
    "Eine Hintergrundaufgabe braucht deine Aufmerksamkeit.",
    "Eine Hintergrundaufgabe ist abgeschlossen.",
    "Neue Aktivität in Kassiber.",
];

/// The fixed body for a category in a language.
#[allow(dead_code)]
pub fn body(category: Category, de: bool) -> &'static str {
    let i = match category {
        Category::NeedsAttention | Category::EvidenceChanged => 0,
        Category::JobFinished => 1,
        Category::NewActivity => 2,
    };
    BODIES[i + if de { 3 } else { 0 }]
}

static LAST: Mutex<Option<Instant>> = Mutex::new(None);

/// `admit`'s backstop: at most one notification per `MIN_INTERVAL`.
fn admit(now: Instant) -> bool {
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
    if last.is_some_and(|l| now.saturating_duration_since(l) < MIN_INTERVAL) {
        return false;
    }
    *last = Some(now);
    true
}

/// Shows `text` (one of [`BODIES`]) through the platform. Headless runs never do. Returns
/// whether it was handed to the system.
pub fn show(text: &'static str) -> bool {
    if !BODIES.contains(&text) || cfg!(test) || rune::is_headless() || !admit(Instant::now()) {
        return false;
    }
    deliver(text)
}

#[cfg(target_os = "macos")]
fn deliver(text: &'static str) -> bool {
    if rune_notify::available() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return rune_notify::post(
            &rune_notify::Notification::new(format!("kassiber-{n}"), TITLE).body(text),
        );
    }
    // Unbundled: the user notification center needs a bundle identifier. The text is one of
    // the fixed sentences (no quotes), passed as one argument, never through a shell.
    let script = format!("display notification \"{text}\" with title \"{TITLE}\"");
    std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| {
            std::thread::spawn(move || c.wait());
        })
        .is_ok()
}

#[cfg(target_os = "windows")]
fn deliver(text: &'static str) -> bool {
    static ADOPTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *ADOPTED
        .get_or_init(|| rune_notify::adopt(&rune_notify::Identity::new(APP_ID, TITLE)).is_ok());
    ok && rune_notify::post(&rune_notify::Notification::new("kassiber", TITLE).body(text))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn deliver(text: &'static str) -> bool {
    use std::process::{Command, Stdio};
    // org.freedesktop.Notifications.Notify(app_name, replaces_id, icon, summary, body, actions,
    // hints, timeout). The arguments are fixed strings; no shell is involved.
    let quoted = format!("\"{text}\"");
    let gdbus = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.Notify",
            TITLE,
            "0",
            "",
            TITLE,
        ])
        .arg(quoted)
        .args(["[]", "{}", "-1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let child = gdbus.or_else(|_| {
        Command::new("notify-send")
            .args(["--app-name", TITLE, TITLE, text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    });
    child
        .map(|mut c| {
            std::thread::spawn(move || c.wait());
        })
        .is_ok()
}

#[cfg(not(any(unix, windows)))]
fn deliver(_: &'static str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_body_is_a_fixed_template() {
        for c in [
            Category::NeedsAttention,
            Category::JobFinished,
            Category::NewActivity,
            Category::EvidenceChanged,
        ] {
            for de in [false, true] {
                let b = body(c, de);
                assert!(BODIES.contains(&b));
                assert!(!b.chars().any(|c| c.is_ascii_digit() || c == '"'));
            }
        }
        assert_eq!(
            body(Category::EvidenceChanged, false),
            "Background work needs your attention."
        );
        assert_eq!(
            body(Category::NewActivity, true),
            "Neue Aktivität in Kassiber."
        );
        // The shell's own bodies are these.
        assert_eq!(
            crate::settings::effects::body(crate::settings::effects::Category::JobFinished, true),
            body(Category::JobFinished, true)
        );
        assert_eq!(
            crate::settings::effects::body(
                crate::settings::effects::Category::NeedsAttention,
                false
            ),
            body(Category::NeedsAttention, false)
        );
    }

    #[test]
    fn only_fixed_text_is_shown_and_tests_show_nothing() {
        assert!(!show("Your 0.5 BTC arrived"));
        // `cfg(test)` processes are headless for this purpose: nothing reaches the system.
        let start = Instant::now();
        assert!(admit(start));
        assert!(!admit(start + Duration::from_secs(1)));
        assert!(admit(start + MIN_INTERVAL));
    }
}
