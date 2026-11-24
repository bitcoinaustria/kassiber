//! Source coverage (lib/sourceCoverage.ts; docs/reference/desktop.md#sources-live-sync-and-file-snapshots):
//! how far each source reaches, whether it is current, and the one action that fixes it. The
//! daemon decides all of it (`ui.sources.coverage`, and each connection's `coverage` in
//! `ui.overview.snapshot`); this module only shapes it for display. Every read is local.

use serde_json::Value;

use crate::i18n::{has, t, tr};

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty())
}

/// `coverageDay`: the `YYYY-MM-DD` of an ISO date or timestamp.
pub fn day(value: Option<&str>) -> Option<String> {
    let v = value?.trim();
    let d = v.get(0..10)?;
    let ok = d.len() == 10
        && d.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    ok.then(|| d.to_owned())
}

/// `formatCoverageDay`: a day as a medium date in the UI language (`Jun 30, 2026` /
/// `30.06.2026`).
pub fn format_day(value: Option<&str>) -> Option<String> {
    let d = day(value)?;
    let parsed = crate::format::Day::parse(&d)?;
    Some(match crate::i18n::lang() {
        crate::i18n::Lang::De => format!("{}.{}.{}", &d[8..10], &d[5..7], &d[0..4]),
        crate::i18n::Lang::En => parsed.long(),
    })
}

/// `periodLabel`: `2025` for a year end, else the formatted day.
pub fn period_label(period_end: &str) -> String {
    if period_end.ends_with("-12-31") && period_end.len() == 10 {
        period_end[..4].to_owned()
    } else {
        format_day(Some(period_end)).unwrap_or_else(|| period_end.to_owned())
    }
}

/// `CoverageTone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warning,
    Error,
    Muted,
}

pub fn state(c: &Value) -> &str {
    s(c, "freshness_state").unwrap_or("unknown")
}

pub fn mode(c: &Value) -> &str {
    s(c, "source_mode").unwrap_or("manual")
}

/// `coverageTone`.
pub fn tone(c: &Value) -> Tone {
    match state(c) {
        "current" => Tone::Ok,
        "behind" | "never_imported" => Tone::Warning,
        "sync_failed" => Tone::Error,
        _ => Tone::Muted,
    }
}

/// `coverageNeedsAction`.
pub fn needs_action(c: Option<&Value>) -> bool {
    c.is_some_and(|c| c["needs_action"] == true)
}

/// `coverageSortRank`: problems first.
pub fn sort_rank(c: Option<&Value>) -> u8 {
    match c.map(state) {
        Some("sync_failed") => 0,
        Some("behind") => 1,
        Some("never_imported") => 2,
        Some("unknown") => 3,
        Some("current") => 4,
        _ => 5,
    }
}

fn mode_word(c: &Value) -> String {
    let key = format!("connections:coverage.mode.{}", mode(c));
    if has(&key) {
        t(&key)
    } else {
        mode(c).to_owned()
    }
}

/// `coverageSummary`: the Connections list's one line (`Live · synced 2h ago`,
/// `File · covers through Jun 30, 2026`, `Behind for 2025`, `Sync failed`).
pub fn summary(c: &Value, period_end: Option<&str>, now: f64) -> String {
    let m = mode_word(c);
    match state(c) {
        "sync_failed" => return t("connections:coverage.state.syncFailed"),
        "never_imported" => {
            return tr(
                if mode(c) == "live" {
                    "connections:coverage.state.neverSynced"
                } else {
                    "connections:coverage.state.neverImported"
                },
                &[("mode", &m)],
            );
        }
        "behind" => {
            if let Some(p) = period_end {
                return tr(
                    "connections:coverage.state.behindFor",
                    &[("period", &period_label(p))],
                );
            }
        }
        _ => {}
    }
    if let Some(a) = c.get("attestation").filter(|a| a.is_object())
        && s(c, "reason") == Some("attested")
    {
        let date = format_day(s(a, "date")).unwrap_or_default();
        return tr(
            if s(a, "kind") == Some("closed_on") {
                "connections:coverage.line.closedOn"
            } else {
                "connections:coverage.line.noActivityAfter"
            },
            &[("mode", &m), ("date", &date)],
        );
    }
    if mode(c) == "manual" {
        return t("connections:coverage.line.manual");
    }
    if matches!(mode(c), "live" | "hybrid")
        && let Some(when) =
            s(&c["live"], "last_sync_at").and_then(|ts| crate::overview::l10n::relative(ts, now))
    {
        return tr(
            "connections:coverage.line.synced",
            &[("mode", &m), ("when", &when)],
        );
    }
    if let Some(through) = format_day(s(c, "covered_through")) {
        return tr(
            "connections:coverage.line.coversThrough",
            &[("mode", &m), ("date", &through)],
        );
    }
    tr("connections:coverage.line.unknown", &[("mode", &m)])
}

/// `parseConnectionsSearch`: `/connections?coverage=needs_action&period=YYYY-MM-DD`; anything
/// else is dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    pub needs_action: bool,
    pub period: Option<String>,
}

pub fn parse_search(query: &str) -> Search {
    let mut out = Search::default();
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "coverage" if v == "needs_action" => out.needs_action = true,
            "period" if day(Some(v)).as_deref() == Some(v) => out.period = Some(v.to_owned()),
            _ => {}
        }
    }
    out
}

/// The route for a search (`/connections`, `/connections?coverage=needs_action&period=…`).
pub fn search_route(needs_action: bool, period: Option<&str>) -> String {
    match (needs_action, period) {
        (false, _) => "/connections".to_owned(),
        (true, None) => "/connections?coverage=needs_action".to_owned(),
        (true, Some(p)) => format!("/connections?coverage=needs_action&period={p}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cov(v: Value) -> Value {
        let mut base = json!({
            "wallet_id": "w1", "wallet": "River", "source_mode": "file",
            "freshness_state": "current", "reason": "", "needs_action": false,
            "live": {}, "file": {"import_count": 0},
        });
        for (k, x) in v.as_object().unwrap() {
            base[k] = x.clone();
        }
        base
    }

    #[test]
    fn days_periods_and_routes() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            day(Some("2026-06-30T12:00:00Z")).as_deref(),
            Some("2026-06-30")
        );
        assert_eq!(day(Some("30.06.2026")), None);
        assert_eq!(
            format_day(Some("2026-06-30")).as_deref(),
            Some("Jun 30, 2026")
        );
        assert_eq!(period_label("2025-12-31"), "2025");
        assert_eq!(period_label("2026-06-30"), "Jun 30, 2026");
        assert_eq!(
            parse_search("coverage=needs_action&period=2025-12-31"),
            Search {
                needs_action: true,
                period: Some("2025-12-31".into())
            }
        );
        assert_eq!(parse_search("coverage=x&period=2025-13"), Search::default());
        assert_eq!(search_route(false, Some("2025-12-31")), "/connections");
        assert_eq!(
            search_route(true, Some("2025-12-31")),
            "/connections?coverage=needs_action&period=2025-12-31"
        );
        crate::i18n::set_lang(crate::i18n::Lang::De);
        assert_eq!(
            format_day(Some("2026-06-30")).as_deref(),
            Some("30.06.2026")
        );
        crate::i18n::set_lang(crate::i18n::Lang::En);
    }

    #[test]
    fn summaries_say_state_mode_and_reach() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let failed = cov(json!({"freshness_state": "sync_failed"}));
        assert_eq!(summary(&failed, None, 0.0), "Sync failed");
        assert_eq!(tone(&failed), Tone::Error);
        let never = cov(json!({"freshness_state": "never_imported", "source_mode": "live"}));
        assert_eq!(summary(&never, None, 0.0), "Live · never synced");
        let behind = cov(json!({"freshness_state": "behind", "covered_through": "2025-06-30"}));
        assert_eq!(summary(&behind, Some("2025-12-31"), 0.0), "Behind for 2025");
        assert_eq!(
            summary(&behind, None, 0.0),
            "File · covers through Jun 30, 2025"
        );
        let attested = cov(json!({
            "reason": "attested",
            "attestation": {"kind": "closed_on", "date": "2026-01-31"},
        }));
        assert_eq!(
            summary(&attested, None, 0.0),
            "File · closed on Jan 31, 2026"
        );
        let manual = cov(json!({"source_mode": "manual"}));
        assert_eq!(summary(&manual, None, 0.0), "Manual entries");
        assert_eq!(summary(&cov(json!({})), None, 0.0), "File · no dated rows");
        let ranks: Vec<u8> = [
            "sync_failed",
            "behind",
            "never_imported",
            "unknown",
            "current",
        ]
        .iter()
        .map(|s| sort_rank(Some(&cov(json!({"freshness_state": s})))))
        .collect();
        assert_eq!(ranks, [0, 1, 2, 3, 4]);
        assert_eq!(sort_rank(None), 5);
        assert!(needs_action(Some(&cov(json!({"needs_action": true})))));
        assert!(!needs_action(None));
    }
}
