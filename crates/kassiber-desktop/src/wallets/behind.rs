//! The `sources.behind` Needs attention condition (useSourcesBehindCondition.ts;
//! docs/reference/desktop.md#sources-live-sync-and-file-snapshots): how many sources do not
//! reach the previous calendar year's end, the period most reports cover. A local read with
//! the shell's reads and each book refresh cycle; no timer, no network. Its action opens
//! Connections filtered to those sources for that period.

use serde_json::json;

use crate::app::{App, Reply};
use crate::i18n::{t, tc};
use crate::wallets::coverage;

/// `previousYearEnd`: the end of the previous calendar year (UTC), the period most reports cover.
pub fn previous_year_end(now_secs: f64) -> String {
    let today = crate::format::Day::from_epoch_days((now_secs / 86_400.0).floor() as i64);
    format!("{}-12-31", today.y - 1)
}

/// The shell's read for the `sources.behind` condition: the previous calendar year (the period
/// most reports cover), with the shell's reads and each book refresh cycle. Local; no timer.
pub fn read_behind(app: &mut App) {
    let period = previous_year_end(crate::format::now_secs());
    app.send(
        "ui.sources.coverage",
        json!({"period_end": period}),
        "wallets.behind",
    );
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "wallets.behind" {
        return;
    }
    // Only a successful read decides anything.
    if r.error.is_none()
        && let Some(period) = r.data["period_end"].as_str()
    {
        // Every source that needs action for the period (behind, failed, never imported): the
        // set the Connections action filters to and Reports warns about (Codex 4660319614).
        let n = r.data["summary"]["needs_action"].as_u64().unwrap_or(0);
        app.wallets.behind = Some((period.to_owned(), n));
    }
}

/// The `sources.behind` row: (title, body) for `count` sources behind `period`.
pub fn behind_text(count: usize, period: &str) -> (String, String) {
    (
        tc(
            "chrome:notifications.condition.sourcesBehind.title",
            count,
            &[("period", &coverage::period_label(period))],
        ),
        t("chrome:notifications.condition.sourcesBehind.body"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codex 4660319614: the condition counts every source that needs action for the period
    /// (behind, failed, never imported), the same set its action filters to.
    #[test]
    fn the_condition_counts_every_source_that_needs_action() {
        let mut app = crate::app::tests::quiet_app();
        let r = Reply {
            kind: "ui.sources.coverage".into(),
            data: json!({"period_end": "2025-12-31", "summary": {"behind": 0, "sync_failed": 1, "never_imported": 1, "needs_action": 2}}),
            error: None,
            error_detail: serde_json::Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.behind", &r);
        assert_eq!(app.wallets.behind, Some(("2025-12-31".to_owned(), 2)));
    }

    #[test]
    fn the_previous_year_and_its_row() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        // 2026-11-17T00:00:00Z
        assert_eq!(previous_year_end(1_794_873_600.0), "2025-12-31");
        let (title, _) = behind_text(2, "2025-12-31");
        assert_eq!(title, "2 sources need action for 2025");
        let (one, _) = behind_text(1, "2025-12-31");
        assert_eq!(one, "1 source needs action for 2025");
    }
}
