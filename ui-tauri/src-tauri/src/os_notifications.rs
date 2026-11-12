//! Opt-in OS (desktop) notifications with fixed, generic content.
//!
//! The renderer decides *whether* an event qualifies (see
//! `ui-tauri/src/lib/osNotifications.ts`); this module decides *what* the
//! operating system is shown, and the renderer cannot influence that. The
//! delivery command accepts only an allowlisted category and an opaque
//! correlation token. The title is always "Kassiber" and the body one of two
//! fixed sentences per language, so no amount, count, name, address, txid,
//! server, file name, path, or error text can reach the OS notification store,
//! in any privacy mode.
//!
//! The renderer gets no `notification:*` capability: the plugin's own
//! commands (and the `window.Notification` polyfill its init script installs)
//! are refused by the ACL, so this command is the only way to show one.
//!
//! Clicks: `tauri-plugin-notification` 2.4 (the newest release that runs on
//! the Tauri 2.11 runtime this app locks) does not report desktop clicks.
//! Clicking a notification activates the app on macOS and Windows; the next
//! time the main window gains focus after a delivery, the token of the last
//! delivered notification is handed back to the renderer once, which routes
//! it after unlock. Nothing else is done on activation: no sync, no approval,
//! no file access, no unlock.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_notification::NotificationExt;

/// Renderer event carrying the token of an activated notification.
pub const ACTIVATED_EVENT: &str = "os-notification://activated";

/// The only title an OS notification from Kassiber ever has.
pub const TITLE: &str = "Kassiber";

/// Backstop against a renderer that skips its own cooldown.
const MIN_INTERVAL: Duration = Duration::from_secs(10);

/// How long after a delivery a focus gain still counts as its activation.
const ACTIVATION_WINDOW: Duration = Duration::from_secs(30 * 60);

const TOKEN_MIN_LEN: usize = 16;
const TOKEN_MAX_LEN: usize = 64;

/// The allowlisted background outcomes that may raise an OS notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// A background failure that needs the user to act.
    NeedsAttention,
    /// A long job finished while Kassiber was not focused.
    JobFinished,
    /// New transactions or observations arrived in the background.
    NewActivity,
    /// A local evidence watch recorded a change.
    EvidenceChanged,
}

impl Category {
    fn template(self) -> Template {
        match self {
            Category::NeedsAttention | Category::EvidenceChanged => Template::Attention,
            Category::JobFinished | Category::NewActivity => Template::Finished,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Template {
    Attention,
    Finished,
}

/// The app's UI language, the only input besides the category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    En,
    /// Austrian German, the `de` UI bundle.
    #[serde(rename = "de")]
    DeAt,
}

/// Every body Kassiber can put in an OS notification.
pub const BODIES: [&str; 4] = [
    "Background work needs your attention.",
    "Background work has finished.",
    "Eine Hintergrundaufgabe braucht deine Aufmerksamkeit.",
    "Eine Hintergrundaufgabe ist abgeschlossen.",
];

/// The fixed body for a category in a language.
pub fn body(category: Category, language: Language) -> &'static str {
    match (category.template(), language) {
        (Template::Attention, Language::En) => BODIES[0],
        (Template::Finished, Language::En) => BODIES[1],
        (Template::Attention, Language::DeAt) => BODIES[2],
        (Template::Finished, Language::DeAt) => BODIES[3],
    }
}

/// An opaque correlation token: renderer-generated, never shown, and limited
/// to a shape that cannot carry prose.
pub fn valid_token(token: &str) -> bool {
    (TOKEN_MIN_LEN..=TOKEN_MAX_LEN).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// What a delivery request came to. Reported back to the renderer, which
/// records a delivery only for `Delivered`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Delivered,
    Disabled,
    Focused,
    Cooldown,
}

#[derive(Default)]
struct Inner {
    enabled: bool,
    language: Language,
    last_delivered: Option<Instant>,
    /// The token of the last delivered notification, until the next focus.
    pending: Option<(String, Instant)>,
}

/// RAM-only state: the opt-in mirrored from the renderer, the backstop
/// interval, and the activation token. Nothing here is written to disk.
#[derive(Default)]
pub struct OsNotificationState {
    inner: Mutex<Inner>,
}

impl OsNotificationState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Mirror the renderer's preference. Returns true when this turns
    /// delivery on, which is the only moment a permission request is made.
    pub fn set_preferences(&self, enabled: bool, language: Language) -> bool {
        let mut inner = self.lock();
        let turned_on = enabled && !inner.enabled;
        inner.enabled = enabled;
        inner.language = language;
        if !enabled {
            inner.pending = None;
        }
        turned_on
    }

    /// Decide a delivery and, when it goes ahead, record it. Returns the
    /// body to show alongside `Outcome::Delivered`.
    pub fn admit(
        &self,
        category: Category,
        token: &str,
        window_focused: bool,
        now: Instant,
    ) -> Result<(Outcome, Option<&'static str>), String> {
        if !valid_token(token) {
            return Err("invalid notification token".into());
        }
        let mut inner = self.lock();
        if !inner.enabled {
            return Ok((Outcome::Disabled, None));
        }
        if window_focused {
            return Ok((Outcome::Focused, None));
        }
        if inner
            .last_delivered
            .is_some_and(|last| now.saturating_duration_since(last) < MIN_INTERVAL)
        {
            return Ok((Outcome::Cooldown, None));
        }
        inner.last_delivered = Some(now);
        inner.pending = Some((token.to_owned(), now));
        Ok((Outcome::Delivered, Some(body(category, inner.language))))
    }

    /// The token to hand back when the main window gains focus: the last
    /// delivered notification's, once, and only within the activation window.
    pub fn take_activation(&self, now: Instant) -> Option<String> {
        let (token, delivered_at) = self.lock().pending.take()?;
        (now.saturating_duration_since(delivered_at) <= ACTIVATION_WINDOW).then_some(token)
    }
}

#[derive(Clone, Serialize)]
struct ActivatedPayload {
    token: String,
}

#[tauri::command]
pub fn set_os_notification_preferences(
    app: AppHandle,
    state: State<'_, OsNotificationState>,
    enabled: bool,
    language: Language,
) -> Result<(), String> {
    if state.set_preferences(enabled, language) {
        // Desktop builds of the plugin answer without prompting; macOS asks
        // (if at all) when the first notification is posted, which can only
        // happen after this opt-in.
        let _ = app.notification().request_permission();
    }
    Ok(())
}

#[tauri::command]
pub fn deliver_os_notification(
    app: AppHandle,
    state: State<'_, OsNotificationState>,
    category: Category,
    token: String,
) -> Result<Outcome, String> {
    // Visibility is not enough: a visible window behind another app is
    // unfocused and qualifies; an unknown window counts as focused.
    let focused = app
        .get_webview_window("main")
        .map(|window| window.is_focused().unwrap_or(true))
        .unwrap_or(true);
    let (outcome, body) = state.admit(category, &token, focused, Instant::now())?;
    if let Some(body) = body {
        app.notification()
            .builder()
            .title(TITLE)
            .body(body)
            .show()
            .map_err(|_| "notification could not be shown".to_string())?;
    }
    Ok(outcome)
}

/// Called when the main window gains focus, which is how a click shows up:
/// the OS has already raised and focused the window by then.
pub fn on_main_window_focused<R: Runtime>(app: &AppHandle<R>) {
    let Some(token) = app
        .try_state::<OsNotificationState>()
        .and_then(|state| state.take_activation(Instant::now()))
    else {
        return;
    };
    let _ = app.emit_to("main", ACTIVATED_EVENT, ActivatedPayload { token });
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef-token";

    fn enabled() -> OsNotificationState {
        let state = OsNotificationState::default();
        state.set_preferences(true, Language::En);
        state
    }

    #[test]
    fn unknown_categories_are_rejected() {
        for raw in [
            "\"needs-attention\"",
            "\"job-finished\"",
            "\"new-activity\"",
            "\"evidence-changed\"",
        ] {
            assert!(serde_json::from_str::<Category>(raw).is_ok(), "{raw}");
        }
        for raw in [
            "\"Your 0.5 BTC arrived\"",
            "\"needs_attention\"",
            "\"NeedsAttention\"",
            "\"\"",
            "{\"title\":\"x\"}",
            "7",
            "null",
        ] {
            assert!(serde_json::from_str::<Category>(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn unknown_languages_are_rejected() {
        assert_eq!(
            serde_json::from_str::<Language>("\"en\"").unwrap(),
            Language::En
        );
        assert_eq!(
            serde_json::from_str::<Language>("\"de\"").unwrap(),
            Language::DeAt
        );
        assert!(serde_json::from_str::<Language>("\"fr\"").is_err());
    }

    #[test]
    fn every_body_is_a_fixed_template() {
        let categories = [
            Category::NeedsAttention,
            Category::JobFinished,
            Category::NewActivity,
            Category::EvidenceChanged,
        ];
        for category in categories {
            for language in [Language::En, Language::DeAt] {
                let text = body(category, language);
                assert!(BODIES.contains(&text), "{text}");
                assert!(!text.chars().any(|c| c.is_ascii_digit()), "{text}");
            }
        }
        assert_eq!(
            body(Category::NeedsAttention, Language::En),
            "Background work needs your attention."
        );
        assert_eq!(
            body(Category::EvidenceChanged, Language::En),
            "Background work needs your attention."
        );
        assert_eq!(
            body(Category::JobFinished, Language::En),
            "Background work has finished."
        );
        assert_eq!(
            body(Category::NewActivity, Language::DeAt),
            "Eine Hintergrundaufgabe ist abgeschlossen."
        );
        assert_eq!(TITLE, "Kassiber");
    }

    #[test]
    fn delivered_body_follows_the_language_preference() {
        let state = OsNotificationState::default();
        state.set_preferences(true, Language::DeAt);
        let (outcome, text) = state
            .admit(Category::NeedsAttention, TOKEN, false, Instant::now())
            .unwrap();
        assert_eq!(outcome, Outcome::Delivered);
        assert_eq!(
            text,
            Some("Eine Hintergrundaufgabe braucht deine Aufmerksamkeit.")
        );
    }

    #[test]
    fn tokens_cannot_carry_prose() {
        assert!(valid_token(TOKEN));
        assert!(!valid_token("short"));
        assert!(!valid_token("Wallet Savings received 0.5 BTC"));
        assert!(!valid_token(&"a".repeat(TOKEN_MAX_LEN + 1)));
        assert!(!valid_token("0123456789abcdef/../etc"));
        let state = enabled();
        assert!(state
            .admit(Category::JobFinished, "bc1q address", false, Instant::now())
            .is_err());
    }

    #[test]
    fn nothing_is_delivered_until_opted_in() {
        let state = OsNotificationState::default();
        let (outcome, text) = state
            .admit(Category::NeedsAttention, TOKEN, false, Instant::now())
            .unwrap();
        assert_eq!((outcome, text), (Outcome::Disabled, None));
        assert!(state.take_activation(Instant::now()).is_none());
    }

    #[test]
    fn a_focused_window_gets_no_notification() {
        let state = enabled();
        let (outcome, text) = state
            .admit(Category::JobFinished, TOKEN, true, Instant::now())
            .unwrap();
        assert_eq!((outcome, text), (Outcome::Focused, None));
    }

    #[test]
    fn deliveries_are_spaced_by_the_backstop_interval() {
        let state = enabled();
        let start = Instant::now();
        assert_eq!(
            state
                .admit(Category::JobFinished, TOKEN, false, start)
                .unwrap()
                .0,
            Outcome::Delivered
        );
        assert_eq!(
            state
                .admit(
                    Category::NeedsAttention,
                    TOKEN,
                    false,
                    start + Duration::from_secs(1)
                )
                .unwrap()
                .0,
            Outcome::Cooldown
        );
        assert_eq!(
            state
                .admit(Category::NeedsAttention, TOKEN, false, start + MIN_INTERVAL)
                .unwrap()
                .0,
            Outcome::Delivered
        );
    }

    #[test]
    fn activation_hands_back_the_last_token_once() {
        let state = enabled();
        let start = Instant::now();
        state
            .admit(Category::JobFinished, TOKEN, false, start)
            .unwrap();
        assert_eq!(
            state
                .take_activation(start + Duration::from_secs(5))
                .as_deref(),
            Some(TOKEN)
        );
        assert!(state
            .take_activation(start + Duration::from_secs(6))
            .is_none());
    }

    #[test]
    fn activation_expires_and_opting_out_forgets_it() {
        let state = enabled();
        let start = Instant::now();
        state
            .admit(Category::JobFinished, TOKEN, false, start)
            .unwrap();
        assert!(state
            .take_activation(start + ACTIVATION_WINDOW + Duration::from_secs(1))
            .is_none());

        state
            .admit(
                Category::JobFinished,
                TOKEN,
                false,
                start + ACTIVATION_WINDOW * 2,
            )
            .unwrap();
        state.set_preferences(false, Language::En);
        assert!(state
            .take_activation(start + ACTIVATION_WINDOW * 2)
            .is_none());
    }

    #[test]
    fn the_renderer_has_no_notification_permission() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(!permissions.is_empty());
        for permission in permissions {
            let text = permission.to_string();
            assert!(!text.contains("notification"), "{text}");
        }
    }

    #[test]
    fn only_the_off_to_on_change_requests_permission() {
        let state = OsNotificationState::default();
        assert!(!state.set_preferences(false, Language::En));
        assert!(state.set_preferences(true, Language::En));
        assert!(!state.set_preferences(true, Language::DeAt));
        assert!(!state.set_preferences(false, Language::DeAt));
        assert!(state.set_preferences(true, Language::DeAt));
    }
}
