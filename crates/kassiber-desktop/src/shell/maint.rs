//! Book refresh and journal processing feedback: the running job's progress (the web's
//! `activeMaintenanceProgress`), what a finished run leaves behind (toast, Recent entry,
//! conditions), and the sync result sentences.
//!
//! Ports `hooks/useWalletSyncAction.ts`, `hooks/useJournalProcessingAction.ts`,
//! `lib/syncProgress.ts` and the parts of `lib/syncResults.ts` those use. The web hard-codes the
//! phase labels, progress labels and run summaries in English; here they come from the bundles
//! (`chrome:syncProgress`) like the titles, toasts and journal phases, and so do the running
//! job's detail lines (source, retry, outcome: `chrome:maintProgress`). The `details` lines no
//! surface renders stay English, and the daemon's own reasons and messages pass through as sent.

use std::collections::HashMap;

use serde_json::Value;

use crate::app::{App, Reply, ToastTone};
use crate::format::count;
use crate::i18n::{t, tc, tr};

/// Which job the maintenance progress belongs to (`BOOK_REFRESH_PROGRESS_ID`,
/// `JOURNAL_PROCESSING_PROGRESS_ID`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    BookRefresh,
    Journals,
}

/// `ActiveMaintenanceProgress`: one running (or failed) job's card and progress line.
#[derive(Clone, Debug, PartialEq)]
pub struct Maint {
    pub job: Job,
    pub title: String,
    pub body: String,
    /// Percent, 0–100 (`progress.value`); None while nothing determinate has arrived.
    pub value: Option<f64>,
    pub indeterminate: bool,
    pub label: String,
    /// `details`: kept as the web keeps them; no surface of the web renders them.
    pub details: Vec<String>,
    pub failed: bool,
    pub phase: Option<String>,
}

impl Maint {
    /// Whether the bar shows a percentage (`isDeterminate`).
    pub fn determinate(&self) -> bool {
        self.value.is_some() && !self.indeterminate
    }

    /// `routeProgressFromActiveMaintenance`'s label: "Title: detail" (the progress line's
    /// live region and the title bar's status detail).
    pub fn route_label(&self) -> String {
        let in_progress;
        let detail = if !self.label.trim().is_empty() {
            self.label.trim()
        } else if !self.body.trim().is_empty() {
            self.body.trim()
        } else {
            in_progress = t("chrome:syncProgress.inProgress");
            in_progress.as_str()
        };
        let title = compact_title(&self.title);
        if !title.is_empty() && title.to_lowercase() != detail.to_lowercase() {
            format!("{title}: {detail}")
        } else {
            detail.to_owned()
        }
    }
}

/// `compactProgressTitle` (progressIndicator.ts).
fn compact_title(title: &str) -> String {
    let t = title.trim();
    let lower = t.to_lowercase();
    let t = ["started", "running"]
        .iter()
        .find_map(|w| {
            lower
                .strip_suffix(w)
                .filter(|r| r.ends_with(char::is_whitespace))
                .map(|r| t[..r.trim_end().len()].to_owned())
        })
        .unwrap_or_else(|| t.to_owned());
    match t.to_lowercase().as_str() {
        "btc price refresh" => "BTC price".to_owned(),
        "journal processing" => "Journals".to_owned(),
        _ => t.trim().to_owned(),
    }
}

/// The book refresh in flight: its option and the progress it has reported.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub force_full: bool,
    /// `progressValueRef`.
    pub value: f64,
    /// `lastProgressRef` and `jobProgressRef`.
    pub last: Option<Value>,
    pub jobs: HashMap<String, Value>,
    /// The last journal run's `auto_pair` choice, for its Retry.
    pub journals_auto_pair: bool,
}

/// `STARTING_SYNC_PROGRESS_VALUE`.
pub const STARTING_VALUE: f64 = 5.0;

// ---- syncProgress.ts ------------------------------------------------------------------------

/// The checklist phase a daemon sub-phase belongs to: on-chain history's `batch_prefetch`
/// fetches source history, so its label, bar position and a failure there land on
/// "Fetching source history", not on whichever later step the overall bar had reached.
pub fn milestone_phase(phase: &str) -> &str {
    match phase {
        "batch_prefetch" => "backend_fetch",
        "importing" => "import",
        other => other,
    }
}

/// `PHASE_LABELS`; an unknown phase shows its own (daemon) name.
pub fn phase_label(phase: &str) -> String {
    let key = match milestone_phase(phase) {
        "discovery" => "chrome:syncProgress.phase.discovery",
        "backend_fetch" => "chrome:syncProgress.phase.backendFetch",
        "decode_enrich" => "chrome:syncProgress.phase.decodeEnrich",
        "import" | "importing" => "chrome:syncProgress.phase.import",
        "rate_coverage" => "chrome:syncProgress.phase.rateCoverage",
        "auto_pair" => "chrome:syncProgress.phase.autoPair",
        "journal_refresh" => "chrome:syncProgress.phase.journalRefresh",
        "rate_limited" => "chrome:syncProgress.phase.rateLimited",
        "done" => "chrome:syncProgress.phase.done",
        "error" => "chrome:syncProgress.phase.error",
        other => return other.replace('_', " "),
    };
    t(key)
}

/// `PHASE_PROGRESS_FRACTIONS`.
fn phase_fraction(phase: &str) -> Option<f64> {
    Some(match milestone_phase(phase) {
        "discovery" => 0.12,
        "backend_fetch" => 0.46,
        "decode_enrich" => 0.62,
        "import" | "importing" => 0.78,
        "rate_coverage" => 0.86,
        "auto_pair" => 0.91,
        "journal_refresh" => 0.94,
        "done" => 1.0,
        _ => return None,
    })
}

/// `FIRST_SYNC_MILESTONES`: the card's checklist, (phase, label, fraction).
pub const MILESTONES: [&str; 7] = [
    "discovery",
    "backend_fetch",
    "decode_enrich",
    "import",
    "rate_coverage",
    "auto_pair",
    "journal_refresh",
];

/// `firstSyncActiveMilestoneIndex`: the first phase the bar has not passed.
pub fn active_milestone(fraction: f64, determinate: bool) -> usize {
    if !determinate {
        return 0;
    }
    MILESTONES
        .iter()
        .position(|p| fraction <= phase_fraction(p).unwrap_or(1.0))
        .unwrap_or(MILESTONES.len())
}

fn clamp(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

fn num(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn s<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str).filter(|v| !v.is_empty())
}

fn phase(p: &Value) -> Option<&str> {
    s(p, "phase")
}

/// `progressNumbers`: discovery reports no total.
fn numbers(p: &Value) -> (Option<f64>, Option<f64>) {
    let processed = num(p, "processed");
    if phase(p) == Some("discovery") {
        (processed, None)
    } else {
        (processed, num(p, "total"))
    }
}

/// `jobNumbers`.
fn job_numbers(p: &Value) -> (Option<f64>, Option<f64>) {
    let pos = |k| num(p, k).filter(|v| *v > 0.0).map(f64::floor);
    (pos("job_index"), pos("job_total"))
}

fn source(p: &Value) -> &str {
    s(p, "wallet")
        .or_else(|| s(p, "source_label"))
        .unwrap_or("")
}

fn fmt_count(v: f64) -> String {
    count(v.max(0.0).round() as usize)
}

/// `computeProgressValue`.
pub fn progress_value(p: &Value, previous: f64) -> f64 {
    match phase(p) {
        Some("error") | Some("rate_limited") => return clamp(previous),
        _ => {}
    }
    let (processed, total) = numbers(p);
    let (index, job_total) = job_numbers(p);
    if let (Some(done), Some(total)) = (processed, total.filter(|t| *t > 0.0)) {
        let rows = clamp(done / total * 100.0) / 100.0;
        if let (Some(i), Some(n)) = (index, job_total) {
            return clamp((i - 1.0 + rows) / n * 100.0);
        }
        return clamp(rows * 100.0);
    }
    if let Some(f) = phase(p).and_then(phase_fraction) {
        if let (Some(i), Some(n)) = (index, job_total) {
            return clamp((i - 1.0 + f) / n * 100.0);
        }
        return clamp(previous.max(f * 100.0));
    }
    if processed.is_some() {
        return 85f64.min(previous.max(previous + 8.0));
    }
    85f64.min(previous + 10.0)
}

fn unit(p: &Value) -> String {
    t(match phase(p) {
        Some("discovery") => "chrome:syncProgress.unit.addresses",
        Some("backend_fetch") => "chrome:syncProgress.unit.scanTargets",
        Some("decode_enrich") => "chrome:syncProgress.unit.transactions",
        _ => "chrome:syncProgress.unit.rows",
    })
}

/// `progressLabel`: "Wallet: Phase · 3 / 10".
pub fn progress_label(p: &Value) -> String {
    let src = source(p);
    let ph = phase(p)
        .map(phase_label)
        .unwrap_or_else(|| t("chrome:syncProgress.refreshingSources"));
    let prefix = if src.is_empty() {
        String::new()
    } else {
        format!("{src}: ")
    };
    match numbers(p) {
        (Some(done), Some(total)) if total > 0.0 => {
            format!("{prefix}{ph} · {} / {}", fmt_count(done), fmt_count(total))
        }
        (Some(done), _) => format!("{prefix}{ph} · {} {}", fmt_count(done), unit(p)),
        _ => format!("{prefix}{ph}"),
    }
}

/// `formatSyncProgressBody`.
fn progress_body(p: &Value) -> String {
    let src = source(p);
    let prefix = if src.is_empty() {
        String::new()
    } else {
        format!("{src}: ")
    };
    let ph = phase(p)
        .map(phase_label)
        .unwrap_or_else(|| t("chrome:syncProgress.refreshRunning"));
    match numbers(p) {
        (Some(done), Some(total)) if total > 0.0 => format!(
            "{prefix}{ph}; {} / {} {}.",
            fmt_count(done),
            fmt_count(total),
            unit(p)
        ),
        (Some(done), _) => {
            let gap = gap_label(p).map(|g| format!(" {g}.")).unwrap_or_default();
            format!("{prefix}{ph}; {} {}.{gap}", fmt_count(done), unit(p))
        }
        _ => format!("{prefix}{ph}."),
    }
}

fn gap_label(p: &Value) -> Option<String> {
    if phase(p) != Some("discovery") {
        return None;
    }
    let gap = num(p, "gap_limit").filter(|g| *g > 0.0)?.floor();
    Some(match num(p, "unused_streak").filter(|u| *u >= 0.0) {
        Some(u) => tr(
            "chrome:syncProgress.gap.unused",
            &[
                ("unused", &fmt_count(u.floor().min(gap))),
                ("gap", &fmt_count(gap)),
            ],
        ),
        None => tr(
            "chrome:syncProgress.gap.stopsAfter",
            &[("gap", &fmt_count(gap))],
        ),
    })
}

fn source_type_label(p: &Value) -> Option<&'static str> {
    Some(match s(p, "source_type")? {
        "onchain_wallet" => "Wallet source",
        "btcpay_wallet" => "BTCPay wallet",
        "btcpay_provenance" => "BTCPay provenance",
        "market_rates" => "Market rates",
        "journals" => "Journals",
        _ => return None,
    })
}

/// `activeSyncTitle`.
fn active_title(p: &Value) -> String {
    let ph = phase(p);
    let st = s(p, "source_type");
    t(if ph == Some("rate_limited") {
        "chrome:syncProgress.phase.rateLimited"
    } else if ph == Some("auto_pair") {
        "chrome:syncProgress.phase.autoPair"
    } else if st == Some("journals") || ph == Some("journal_refresh") {
        "chrome:syncProgress.phase.journalRefresh"
    } else if st == Some("market_rates") || ph == Some("rate_coverage") {
        "chrome:syncProgress.title.checkingMarketRates"
    } else {
        "chrome:notifications.job.bookRefresh"
    })
}

/// `activeSyncMaintenanceProgress`'s detail lines.
fn details(p: &Value) -> Vec<String> {
    let (index, total) = job_numbers(p);
    let job = index.zip(total).map(|(i, n)| {
        tr(
            "chrome:maintProgress.sourceOf",
            &[("index", &fmt_count(i.min(n))), ("total", &fmt_count(n))],
        )
    });
    let src = source(p);
    let named = if src.is_empty() {
        source_type_label(p).map(str::to_owned)
    } else {
        Some(src.to_owned())
    };
    let backoff = (phase(p) == Some("rate_limited")).then(|| {
        let retry = match (num(p, "retry_attempt"), num(p, "retry_max")) {
            (Some(a), Some(m)) => tr(
                "chrome:maintProgress.retry",
                &[("attempt", &a.to_string()), ("max", &m.to_string())],
            ),
            _ => t("chrome:maintProgress.retrying"),
        };
        let wait = num(p, "wait_seconds")
            .map(|w| {
                tr(
                    "chrome:maintProgress.waitIn",
                    &[("seconds", &w.to_string())],
                )
            })
            .unwrap_or_default();
        tr(
            "chrome:maintProgress.rateLimited",
            &[("retry", &retry), ("wait", &wait)],
        )
    });
    let rows = match numbers(p) {
        (Some(done), Some(total)) if total > 0.0 => Some(format!(
            "{} / {} {}",
            fmt_count(done),
            fmt_count(total),
            unit(p)
        )),
        (Some(done), _) => Some(format!("{} {}", fmt_count(done), unit(p))),
        _ => None,
    };
    let retained = (phase(p) == Some("discovery"))
        .then(|| num(p, "retained_targets").filter(|r| *r >= 0.0))
        .flatten()
        .map(|r| {
            tr(
                "chrome:maintProgress.retained",
                &[("count", &fmt_count(r.floor()))],
            )
        });
    let outcome = {
        let parts: Vec<String> = [
            num(p, "imported")
                .map(|n| tr("chrome:maintProgress.imported", &[("count", &fmt_count(n))])),
            num(p, "skipped")
                .map(|n| tr("chrome:maintProgress.known", &[("count", &fmt_count(n))])),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    };
    [job, named, backoff, rows, gap_label(p), retained, outcome]
        .into_iter()
        .flatten()
        .collect()
}

/// `activeSyncMaintenanceProgress`.
pub fn active_sync(p: &Value, previous: f64) -> Maint {
    let value = progress_value(p, previous);
    let src = source(p);
    let ph = phase(p)
        .map(phase_label)
        .unwrap_or_else(|| t("chrome:syncProgress.refreshingSources"));
    Maint {
        job: Job::BookRefresh,
        title: active_title(p),
        body: if src.is_empty() {
            progress_body(p)
        } else {
            format!("{src}: {ph}.")
        },
        value: Some(value),
        indeterminate: phase(p) == Some("discovery"),
        label: progress_label(p),
        details: details(p),
        failed: false,
        phase: phase(p).map(str::to_owned),
    }
}

/// `failedSyncMaintenanceProgress`.
fn failed_sync(p: &Value, previous: f64, body: String, title: String) -> Maint {
    Maint {
        title,
        body,
        failed: true,
        ..active_sync(p, previous)
    }
}

/// `failedSyncProgressForRun`: the progress of the job or source that failed.
fn failed_progress(data: &Value, run: &Run) -> Value {
    let completed = arr(data, "completed");
    let failed_job = completed
        .iter()
        .find(|j| matches!(s(j, "status"), Some("error" | "cancelled")));
    let sources = arr(data, "sources");
    let failed_source = sources.iter().find(|x| {
        s(x, "status") == Some("failed") || x["blocking_reports"].as_bool() == Some(true)
    });
    if let Some(p) = failed_job
        .and_then(|j| s(j, "id"))
        .and_then(|id| run.jobs.get(id))
    {
        return p.clone();
    }
    if let Some(label) = failed_source.and_then(|x| s(x, "source_label"))
        && let Some(p) = run
            .jobs
            .values()
            .find(|p| s(p, "source_label") == Some(label))
    {
        return p.clone();
    }
    if let Some(p) = &run.last {
        return p.clone();
    }
    let pick = |k: &str| {
        failed_job
            .and_then(|j| s(j, k))
            .or_else(|| failed_source.and_then(|x| s(x, k)))
    };
    let mut out = serde_json::Map::new();
    for k in ["source_label", "source_type"] {
        if let Some(v) = pick(k) {
            out.insert(k.into(), Value::from(v));
        }
    }
    Value::Object(out)
}

// ---- syncResults.ts -------------------------------------------------------------------------

fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn positive(v: Option<&Value>) -> usize {
    v.and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)
        .map(|n| n.floor() as usize)
        .unwrap_or(0)
}

/// `journalStepNeedsAttention`.
pub fn journal_step_needs_attention(j: &Value) -> bool {
    matches!(
        s(j, "status"),
        Some("failed" | "cancelled" | "disabled" | "deferred")
    ) || j["auto_pair_skipped"].as_bool() == Some(true)
}

fn error_text(j: &Value) -> String {
    s(&j["error"], "message")
        .or_else(|| s(&j["error"], "code"))
        .unwrap_or("")
        .to_owned()
}

/// `describeJournalStep`: one sentence, or None when no journal step was requested.
pub fn describe_journal_step(j: &Value) -> Option<String> {
    let status = s(j, "status").filter(|st| *st != "skipped")?;
    let mut parts = Vec::new();
    match status {
        "processed" | "current" => {
            let n = positive(j.get("quarantines"));
            parts.push(if status == "current" {
                t("connections:journalStep.current")
            } else if n > 0 {
                tc("connections:journalStep.processed", n, &[])
            } else {
                t("connections:journalStep.processedClear")
            });
            let paired = positive(j.get("auto_pair_applied"));
            if paired > 0 {
                parts.push(tc("connections:journalStep.autoPaired", paired, &[]));
            }
            if j["auto_pair_skipped"].as_bool() == Some(true) {
                parts.push(tr(
                    "connections:journalStep.autoPairSkipped",
                    &[("message", &error_text(j))],
                ));
            }
        }
        "failed" => parts.push(tr(
            "connections:journalStep.failed",
            &[("message", &error_text(j))],
        )),
        "deferred" => parts.push(t("connections:journalStep.deferred")),
        "disabled" => parts.push(t("connections:journalStep.disabled")),
        "cancelled" => parts.push(t("connections:journalStep.cancelled")),
        _ => {}
    }
    let out = parts.join(" ");
    (!out.is_empty()).then_some(out)
}

/// `syncResultDetail`.
fn result_detail(r: &Value) -> Option<String> {
    let primary = s(r, "message")
        .or_else(|| s(r, "reason"))
        .or_else(|| s(r, "code"));
    let out = [primary, s(r, "hint")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    (!out.is_empty()).then_some(out)
}

/// `syncResultObservability`: what a synced wallet's refresh saw.
fn observability(r: &Value) -> Option<String> {
    if s(r, "status") != Some("synced") {
        return None;
    }
    let n = |k: &str| r.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut parts: Vec<String> = Vec::new();
    // A count in the sentence (`{{count}}`), grouped as the language groups it.
    let counted = |key: &str, v: f64| tr(key, &[("count", &fmt_count(v))]);
    let plural = |key: &str, v: f64| tc(key, v.max(0.0).round() as usize, &[]);
    if r["force_full"].as_bool() == Some(true) {
        parts.push(t("chrome:syncProgress.observed.fullRescan"));
    }
    if s(r, "observer_route") == Some("compatibility") {
        // The daemon's reason code, read as words; the fallback is ours.
        let reason = s(r, "observer_compatibility_reason")
            .map(|x| x.replace('_', " "))
            .unwrap_or_else(|| t("chrome:syncProgress.observed.capabilityFallback"));
        parts.push(tr(
            "chrome:syncProgress.observed.compatibility",
            &[("reason", &reason)],
        ));
    }
    let (imported, updated, unchanged) = (n("imported"), n("updated"), n("unchanged"));
    if imported != 0.0 || updated != 0.0 || unchanged != 0.0 {
        let rows: Vec<String> = [
            (imported, "chrome:syncProgress.observed.imported"),
            (updated, "chrome:syncProgress.observed.updated"),
            (unchanged, "chrome:syncProgress.observed.unchanged"),
        ]
        .iter()
        .filter(|(v, _)| *v != 0.0)
        .map(|(v, key)| counted(key, *v))
        .collect();
        parts.push(rows.join(", "));
    } else if let Some(f) = num(r, "records_fetched") {
        parts.push(counted("chrome:syncProgress.observed.sourceRows", f));
    }
    if let Some(c) = num(r, "scripts_checked").filter(|c| *c > 0.0) {
        parts.push(counted("chrome:syncProgress.observed.scriptsChecked", c));
    } else if let Some(c) = num(r, "target_count").filter(|c| *c > 0.0) {
        parts.push(counted("chrome:syncProgress.observed.targets", c));
    }
    if num(r, "reconciliation_rows").is_some_and(|v| v > 0.0) && n("reconciliation_changed") > 0.0 {
        if n("matched") != 0.0 {
            parts.push(counted(
                "chrome:syncProgress.observed.overlapsMatched",
                n("matched"),
            ));
        }
        if n("ambiguous") != 0.0 {
            parts.push(counted(
                "chrome:syncProgress.observed.overlapsReview",
                n("ambiguous"),
            ));
        }
        let re = n("reactivated");
        if re != 0.0 {
            parts.push(plural("chrome:syncProgress.observed.reactivated", re));
        }
    }
    let sup = n("observer_superseded");
    if sup != 0.0 {
        parts.push(plural("chrome:syncProgress.observed.retired", sup));
    }
    if r["utxos_skipped_unchanged"].as_bool() == Some(true) {
        parts.push(t("chrome:syncProgress.observed.utxosUnchanged"));
    } else if r["utxos_refreshed"].as_bool() == Some(true) {
        parts.push(t("chrome:syncProgress.observed.utxosRefreshed"));
    }
    match r["journal_invalidated"].as_bool() {
        Some(false) => parts.push(t("chrome:syncProgress.observed.journalsUnchanged")),
        Some(true) => parts.push(t("chrome:syncProgress.observed.journalsStale")),
        None => {}
    }
    if let Some(ms) = num(r, "elapsed_ms").filter(|m| *m >= 0.0) {
        parts.push(counted("chrome:syncProgress.observed.elapsed", ms.round()));
    }
    let out = parts.join(" · ");
    (!out.is_empty()).then_some(out)
}

/// `describeSingleWalletSyncResult`.
fn describe_single(result: Option<&Value>, label: &str) -> String {
    let Some(r) = result else {
        return tr("connections:detail.sync.missingResult", &[("label", label)]);
    };
    let wallet = s(r, "wallet").unwrap_or(label);
    // The daemon's message, reason or code, as sent.
    let detail = result_detail(r);
    let with = |plain: &str, detailed: &str| match &detail {
        Some(d) => tr(detailed, &[("wallet", wallet), ("detail", d)]),
        None => tr(plain, &[("wallet", wallet)]),
    };
    match s(r, "status").unwrap_or("") {
        "error" => with(
            "chrome:syncProgress.result.failed",
            "chrome:syncProgress.result.failedDetail",
        ),
        "skipped" => with(
            "chrome:syncProgress.result.skipped",
            "chrome:syncProgress.result.skippedDetail",
        ),
        "synced" => match observability(r) {
            Some(o) => tr(
                "chrome:syncProgress.result.refreshedDetail",
                &[("wallet", wallet), ("detail", &o)],
            ),
            None => tr(
                "chrome:syncProgress.result.refreshed",
                &[("wallet", wallet)],
            ),
        },
        "queued" => tr("chrome:syncProgress.result.queued", &[("wallet", wallet)]),
        "rate_limited" => with(
            "chrome:syncProgress.result.coolingDown",
            "chrome:syncProgress.result.coolingDownDetail",
        ),
        "partially_stale" => with(
            "chrome:syncProgress.result.partial",
            "chrome:syncProgress.result.partialDetail",
        ),
        other => tr(
            "chrome:syncProgress.result.other",
            &[("wallet", wallet), ("status", other)],
        ),
    }
}

/// `describeWalletSyncResult`: the wallet's sentence, then the journal step's.
pub fn describe_wallet_sync(result: Option<&Value>, label: &str, journals: &Value) -> String {
    let journal = describe_journal_step(journals);
    let described = match (result, &journal) {
        // The sync finished with the journal step, so "journals marked stale" would be stale.
        (Some(r), Some(_)) if s(r, "status") == Some("synced") => {
            let mut r = r.clone();
            if let Some(o) = r.as_object_mut() {
                o.remove("journal_invalidated");
            }
            describe_single(Some(&r), label)
        }
        _ => describe_single(result, label),
    };
    match journal {
        Some(j) => format!("{described} {j}"),
        None => described,
    }
}

/// `summarizeSyncResults`.
fn summarize_results(results: &[Value]) -> String {
    let n = |st: &str| {
        results
            .iter()
            .filter(|r| s(r, "status") == Some(st))
            .count()
    };
    let parts: Vec<String> = [
        (n("synced"), "chrome:syncProgress.summary.refreshed"),
        (n("skipped"), "chrome:syncProgress.summary.skipped"),
        (n("error"), "chrome:syncProgress.summary.failed"),
    ]
    .iter()
    .filter(|(c, _)| *c > 0)
    .map(|(c, key)| tr(key, &[("count", &count(*c))]))
    .collect();
    let summary = if parts.is_empty() {
        t("chrome:syncProgress.summary.noChanges")
    } else {
        parts.join(", ")
    };
    match results
        .iter()
        .find(|r| s(r, "status") == Some("error"))
        .and_then(|r| result_detail(r).map(|d| (s(r, "wallet").unwrap_or("").to_owned(), d)))
    {
        Some((w, d)) => format!("{summary}: {w}: {d}"),
        None => summary,
    }
}

fn pending_jobs(data: &Value) -> Vec<Value> {
    let terminal: Vec<String> = arr(data, "completed")
        .iter()
        .filter(|j| matches!(s(j, "status"), Some("done" | "error" | "cancelled")))
        .filter_map(|j| s(j, "id").map(str::to_owned))
        .collect();
    arr(data, "enqueued")
        .into_iter()
        .filter(|j| s(j, "id").is_none_or(|id| !terminal.iter().any(|x| x == id)))
        .collect()
}

/// `freshnessRunHasPendingJobs`.
pub fn has_pending(data: &Value) -> bool {
    !pending_jobs(data).is_empty()
}

fn auto_pair_problem(job: &Value) -> bool {
    let a = &job["result"]["auto_pair"];
    a.is_object()
        && (a["skipped"].as_bool() == Some(true)
            || (!a["error"].is_null() && a.get("error").is_some()))
}

/// `freshnessRunNeedsAttention`.
pub fn needs_attention(data: &Value) -> bool {
    let completed = arr(data, "completed");
    completed
        .iter()
        .any(|j| matches!(s(j, "status"), Some("error" | "cancelled")))
        || completed
            .iter()
            .any(|j| s(j, "job_type") == Some("journal_refresh") && auto_pair_problem(j))
        || journal_step_needs_attention(&data["journals"])
        || arr(data, "sources").iter().any(|x| {
            x["blocking_reports"].as_bool() == Some(true) || s(x, "status") == Some("failed")
        })
        || positive(data["summary"].get("failed")) > 0
        || positive(data["summary"].get("blocking_reports")) > 0
}

fn journal_jobs(data: &Value) -> Vec<Value> {
    arr(data, "completed")
        .into_iter()
        .filter(|j| s(j, "job_type") == Some("journal_refresh"))
        .collect()
}

/// `freshnessRunQuarantineCount`.
pub fn quarantine_count(data: &Value) -> usize {
    journal_jobs(data)
        .iter()
        .map(|j| {
            positive(j["result"].get("quarantined"))
                .max(positive(j["result"].get("quarantine_count")))
        })
        .sum()
}

/// `summarizeFreshnessRun`: the failed card's body.
pub fn summarize_run(data: &Value) -> String {
    let completed = arr(data, "completed");
    if completed.is_empty() {
        let enqueued = arr(data, "enqueued").len();
        if enqueued > 0 {
            return tc("chrome:syncProgress.summary.queued", enqueued, &[]);
        }
        return summarize_results(&arr(data, "results"));
    }
    let count_of = |pred: &dyn Fn(&Value) -> bool| completed.iter().filter(|j| pred(j)).count();
    let done = count_of(&|j| s(j, "status") == Some("done"));
    let cooling = count_of(&|j| s(j, "status") == Some("rate_limited"));
    let failed = count_of(&|j| matches!(s(j, "status"), Some("error" | "cancelled")));
    let quarantined = quarantine_count(data);
    let jobs = journal_jobs(data);
    let paired: usize = jobs
        .iter()
        .map(|j| positive(j["result"]["auto_pair"].get("applied")))
        .sum();
    let review: usize = jobs
        .iter()
        .map(|j| positive(j["result"]["auto_pair"]["remaining"].get("total")))
        .sum();
    let pair_problem = jobs.iter().find(|j| auto_pair_problem(j));
    let counted = |key: &str, n: usize| tr(key, &[("count", &count(n))]);
    let parts: Vec<String> = [
        (done > 0).then(|| counted("chrome:syncProgress.summary.completed", done)),
        (cooling > 0).then(|| counted("chrome:syncProgress.summary.coolingDown", cooling)),
        (failed > 0).then(|| counted("chrome:syncProgress.summary.needsAttention", failed)),
        pair_problem.map(|_| t("chrome:syncProgress.summary.pairingSkipped")),
        (paired > 0).then(|| tc("chrome:syncProgress.summary.pairsApplied", paired, &[])),
        (review > 0).then(|| tc("chrome:syncProgress.summary.candidates", review, &[])),
        (quarantined > 0).then(|| tc("chrome:syncProgress.summary.quarantined", quarantined, &[])),
        journal_step_needs_attention(&data["journals"])
            .then(|| describe_journal_step(&data["journals"]))
            .flatten(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let summary = if parts.is_empty() {
        t("chrome:syncProgress.summary.noChanges")
    } else {
        parts.join(", ")
    };
    let first_problem = completed
        .iter()
        .find(|j| s(j, "status").is_some_and(|st| st != "done"));
    let detail = first_problem
        .map(|j| {
            [
                s(&j["error"], "message").or_else(|| s(&j["error"], "code")),
                s(&j["error"], "hint"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        })
        .unwrap_or_default();
    let pair_detail = pair_problem.and_then(|j| s(&j["result"]["auto_pair"]["error"], "message"));
    match (first_problem, detail.is_empty(), pair_detail) {
        (Some(j), false, _) => format!(
            "{summary}: {}: {detail}",
            s(j, "source_label")
                .map(str::to_owned)
                .unwrap_or_else(|| t("chrome:syncProgress.summary.source"))
        ),
        (_, _, Some(d)) => format!("{summary}: {d}"),
        _ => summary,
    }
}

// ---- Journal processing ----------------------------------------------------------------------

/// `journalProcessPhaseKey`.
pub fn journal_phase_key(phase: Option<&str>) -> &'static str {
    match phase {
        Some("writer_wait") => "journals:processing.phase.writerWait",
        Some("auto_pair") => "journals:processing.phase.autoPair",
        Some("preparing") => "journals:processing.phase.preparing",
        Some("repairing") => "journals:processing.phase.repairing",
        Some("pricing") => "journals:processing.phase.pricing",
        Some("building") => "journals:processing.phase.building",
        Some("storing") => "journals:processing.phase.storing",
        Some("complete") => "journals:processing.phase.complete",
        _ => "journals:processing.phase.fallback",
    }
}

fn journal_maint(body: String, label: String) -> Maint {
    Maint {
        job: Job::Journals,
        title: t("journals:processing.title"),
        body,
        value: None,
        indeterminate: true,
        label,
        details: vec![t("journals:processing.reportHint")],
        failed: false,
        phase: None,
    }
}

/// `journalCountsText`: the success toast's description.
pub fn journal_counts(data: &Value) -> Option<String> {
    let n = |k: &str| {
        data.get(k)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .map(|v| v.max(0.0) as usize)
    };
    let parts: Vec<String> = [
        n("processed_transactions").map(|c| {
            tc(
                "chrome:notifications.result.journalsProcessed.transactions",
                c,
                &[],
            )
        }),
        n("entries_created").map(|c| {
            tc(
                "chrome:notifications.result.journalsProcessed.entries",
                c,
                &[],
            )
        }),
        n("quarantined").filter(|c| *c > 0).map(|c| {
            tc(
                "chrome:notifications.result.journalsProcessed.quarantined",
                c,
                &[],
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// `recordJournalStep`: a journal step that did not finish after a sync or import. Failed or
/// cancelled raises the journal-processing failure (Retry processes again); deferred or disabled
/// is the stale-journals condition the review badges raise; a skipped automatic pairing is a
/// toast only.
pub fn record_journal_step(app: &mut App, journals: &Value) {
    if !journal_step_needs_attention(journals) {
        return;
    }
    match s(journals, "status") {
        Some("failed" | "cancelled") => {
            app.shell.journals_failed = Some(error_text(journals))
                .filter(|m| !m.is_empty())
                .or_else(|| describe_journal_step(journals));
            app.shell.run.journals_auto_pair = true;
        }
        // The badges' next read raises "Journals need processing" (`journals.stale`).
        Some("deferred" | "disabled") => {}
        _ => {
            let body = describe_journal_step(journals);
            app.toast(
                ToastTone::Warning,
                &t("connections:journalStep.attentionTitle"),
                body,
                Some("journal-step"),
            );
        }
    }
}

// ---- The app's runs ---------------------------------------------------------------------------

/// The book refresh started (`startRefresh`): the card at "Preparing source refresh", 5 %.
pub fn start_refresh(app: &mut App, force_full: bool) {
    app.shell.run = Run {
        force_full,
        value: STARTING_VALUE,
        journals_auto_pair: app.shell.run.journals_auto_pair,
        ..Run::default()
    };
    let m = Maint {
        job: Job::BookRefresh,
        title: t(if force_full {
            "overview:bookRefresh.rescanInProgressTitle"
        } else {
            "overview:bookRefresh.refreshInProgressTitle"
        }),
        body: t(if force_full {
            "overview:bookRefresh.rescanStartedBody"
        } else {
            "overview:bookRefresh.refreshStartedBody"
        }),
        value: Some(STARTING_VALUE),
        indeterminate: false,
        // `startingSyncProgress`.
        label: t("chrome:firstSync.preparing"),
        details: vec![
            t("overview:bookRefresh.configuredSourcesQueued"),
            t("overview:bookRefresh.autoPairIncluded"),
            t("overview:bookRefresh.journalsIncluded"),
        ],
        failed: false,
        phase: None,
    };
    set(app, m);
    // A new run starts expanded (the web restores a minimized card once a run is gone).
    app.shell.sync_card_open = true;
}

/// A journal run started (`startJournalRun`).
pub fn start_journals(app: &mut App, auto_pair: bool) {
    app.shell.run.journals_auto_pair = auto_pair;
    set(
        app,
        journal_maint(
            t("journals:processing.startedBody"),
            t("journals:processing.phase.fallback"),
        ),
    );
}

fn set(app: &mut App, m: Maint) {
    app.busy.progress = Some(m.label.clone());
    app.shell.maint = Some(m);
}

/// Clears the job's progress (`clearActiveMaintenanceProgress(id)`).
pub fn clear(app: &mut App, job: Job) {
    if app.shell.maint.as_ref().is_some_and(|m| m.job == job) {
        app.shell.maint = None;
    }
    app.busy.progress = None;
}

/// A progress record of a running book refresh or journal run.
pub fn on_progress(app: &mut App, tag: &str, env: &Value) -> bool {
    let data = &env["data"];
    // Some producers nest the phase under `progress`.
    let p = if phase(data).is_none() && data["progress"].is_object() {
        &data["progress"]
    } else {
        data
    };
    match tag {
        "app.freshness_run" => {
            let ph = phase(p);
            if ph != Some("error") && ph != Some("done") {
                app.shell.run.last = Some(p.clone());
                if let Some(id) = s(p, "job_id") {
                    app.shell.run.jobs.insert(id.to_owned(), p.clone());
                }
            }
            let previous = app.shell.run.value;
            let m = active_sync(p, previous);
            app.shell.run.value = m.value.unwrap_or(previous);
            set(app, m);
            true
        }
        "app.journals_process" => {
            let label = t(journal_phase_key(phase(p)));
            set(app, journal_maint(label.clone(), label));
            true
        }
        _ => false,
    }
}

/// `daemon` error code `stale_context`: the run was refused because another book is open.
fn stale_book(r: &Reply) -> bool {
    r.error_detail.get("code").and_then(Value::as_str) == Some("stale_context")
}

/// A finished book refresh (`startRefresh`'s settle + `recordBookRefreshOutcome` /
/// `recordBookRefreshFailure`).
pub fn on_refresh_reply(app: &mut App, r: &Reply) {
    let force = app.shell.run.force_full;
    let at = app.clock;
    if let Some(e) = &r.error {
        if stale_book(r) {
            clear(app, Job::BookRefresh);
            app.toast(
                ToastTone::Info,
                &t("chrome:notifications.staleBook"),
                None,
                Some("stale-book"),
            );
            return;
        }
        app.shell.sync_failed = true;
        let title = t(if force {
            "chrome:notifications.condition.bookRefreshFailed.rescanTitle"
        } else {
            "chrome:notifications.condition.bookRefreshFailed.title"
        });
        app.toast(ToastTone::Error, &title, None, Some("book-refresh.outcome"));
        app.shell
            .note(&title, Some(e.clone()), ToastTone::Error, at);
        let last = app
            .shell
            .run
            .last
            .clone()
            .unwrap_or_else(|| serde_json::json!({"phase": "error"}));
        let body = if e.is_empty() {
            t("overview:bookRefresh.failedBody")
        } else {
            e.clone()
        };
        let m = failed_sync(
            &last,
            app.shell.run.value,
            body,
            t("overview:bookRefresh.failedTitle"),
        );
        set(app, m);
        app.busy.progress = None;
        app.shell.sync_card_open = true;
        return;
    }
    // A returned run resolves an earlier whole-run failure.
    app.shell.sync_failed = false;
    let data = &r.data;
    let attention = needs_attention(data);
    let pending = has_pending(data);
    let blocks_first_sync = attention || pending || quarantine_count(data) > 0;
    if !blocks_first_sync
        && let Some(k) =
            crate::onboarding::model::book_identity_key(app.onboarding.identity.as_ref())
    {
        app.onboarding.first_sync_done.insert(k, true);
    }
    if attention {
        let title = t("chrome:notifications.result.bookRefreshProblems.title");
        app.toast(
            ToastTone::Warning,
            &title,
            None,
            Some("book-refresh.outcome"),
        );
        let journals_only = journal_step_needs_attention(&data["journals"]) && {
            let mut d = data.clone();
            d["journals"] = Value::Null;
            !needs_attention(&d)
        };
        let detail = journals_only
            .then(|| describe_journal_step(&data["journals"]))
            .flatten()
            .or_else(|| Some(t("chrome:notifications.result.bookRefreshProblems.body")));
        app.shell.note(&title, detail, ToastTone::Warning, at);
        let p = failed_progress(data, &app.shell.run);
        let m = failed_sync(
            &p,
            app.shell.run.value,
            summarize_run(data),
            t("overview:bookRefresh.needsAttentionTitle"),
        );
        set(app, m);
        app.busy.progress = None;
        app.shell.sync_card_open = true;
        return;
    }
    clear(app, Job::BookRefresh);
    app.shell.sync_card_open = false;
    if pending {
        let title = t("chrome:notifications.result.bookRefreshPending.title");
        app.toast(ToastTone::Info, &title, None, Some("book-refresh.outcome"));
        app.shell.note(
            &title,
            Some(t("chrome:notifications.result.bookRefreshPending.body")),
            ToastTone::Info,
            at,
        );
        return;
    }
    let title = t(if force {
        "chrome:notifications.result.bookRefreshed.rescanTitle"
    } else {
        "chrome:notifications.result.bookRefreshed.title"
    });
    app.toast(
        ToastTone::Success,
        &title,
        None,
        Some("book-refresh.outcome"),
    );
    app.shell.note(&title, None, ToastTone::Success, at);
}

/// A finished journal run (`recordJournalSuccess` / `recordJournalFailure`); the progress goes
/// either way.
pub fn on_journals_reply(app: &mut App, r: &Reply) {
    clear(app, Job::Journals);
    let at = app.clock;
    if let Some(e) = &r.error {
        if stale_book(r) {
            app.toast(
                ToastTone::Info,
                &t("chrome:notifications.staleBook"),
                None,
                Some("stale-book"),
            );
            return;
        }
        app.shell.journals_failed = Some(e.clone());
        let title = t("chrome:notifications.condition.journalsFailed.title");
        app.toast(
            ToastTone::Error,
            &title,
            None,
            Some("journals.process-failed"),
        );
        app.shell
            .note(&title, Some(e.clone()), ToastTone::Error, at);
        return;
    }
    app.shell.journals_failed = None;
    let data = &r.data;
    let warnings = arr(data, "warnings").len();
    let skipped = data["auto_pair"]["skipped"].as_bool() == Some(true);
    let quarantined = positive(data.get("quarantined"));
    let warn = warnings > 0 || skipped || quarantined > 0;
    let title = t("chrome:notifications.result.journalsProcessed.title");
    let counts = journal_counts(data);
    app.toast(
        if warn {
            ToastTone::Warning
        } else {
            ToastTone::Success
        },
        &title,
        counts.clone(),
        Some("journals.processed"),
    );
    app.shell.note(
        &title,
        counts,
        if warn {
            ToastTone::Warning
        } else {
            ToastTone::Success
        },
        at,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_lang};
    use serde_json::json;

    #[test]
    fn batch_prefetch_belongs_to_fetching_source_history() {
        assert_eq!(phase_label("batch_prefetch"), phase_label("backend_fetch"));
        assert_eq!(
            phase_fraction("batch_prefetch"),
            phase_fraction("backend_fetch")
        );
        assert_eq!(
            MILESTONES
                .iter()
                .position(|p| *p == milestone_phase("batch_prefetch")),
            Some(1)
        );
    }

    #[test]
    fn progress_values_follow_sync_progress_ts() {
        // Rows of a job: (index - 1 + rows) / total.
        assert_eq!(
            progress_value(
                &json!({"phase": "backend_fetch", "processed": 5, "total": 10, "job_index": 2, "job_total": 4}),
                5.0
            ),
            37.5
        );
        // A phase alone moves to its fraction, never back.
        assert_eq!(progress_value(&json!({"phase": "import"}), 5.0), 78.0);
        assert_eq!(progress_value(&json!({"phase": "discovery"}), 50.0), 50.0);
        // Rate limits and errors hold the bar.
        assert_eq!(
            progress_value(&json!({"phase": "rate_limited"}), 40.0),
            40.0
        );
        // Unknown phases creep, capped at 85.
        assert_eq!(progress_value(&json!({"phase": "mystery"}), 80.0), 85.0);
        assert_eq!(active_milestone(0.5, true), 2);
        assert_eq!(active_milestone(1.0, true), 7);
        assert_eq!(active_milestone(0.9, false), 0);
    }

    #[test]
    fn labels_and_titles() {
        set_lang(Lang::En);
        assert_eq!(
            progress_label(
                &json!({"phase": "backend_fetch", "wallet": "Cold", "processed": 1200, "total": 3400})
            ),
            "Cold: Fetching source history · 1,200 / 3,400"
        );
        assert_eq!(
            progress_label(&json!({"phase": "discovery", "processed": 20})),
            "Discovering wallet history · 20 addresses probed"
        );
        let m = active_sync(
            &json!({"phase": "journal_refresh", "source_type": "journals"}),
            5.0,
        );
        assert_eq!(m.title, "Refreshing journals");
        assert_eq!(m.details, vec!["Journals".to_owned()]);
        assert_eq!(m.route_label(), "Refreshing journals");
        let m = active_sync(
            &json!({"phase": "rate_limited", "source_label": "Hot", "retry_attempt": 2, "retry_max": 5, "wait_seconds": 30}),
            20.0,
        );
        assert_eq!(
            m.details,
            vec![
                "Hot".to_owned(),
                "Rate limited — retry 2/5 in 30s".to_owned()
            ]
        );
        assert_eq!(
            m.route_label(),
            "Waiting out rate limit: Hot: Waiting out rate limit"
        );
        set_lang(Lang::De);
        assert_ne!(phase_label("backend_fetch"), "Fetching source history");
        assert_ne!(summarize_run(&json!({})), "No source changes returned.");
        set_lang(Lang::En);
    }

    #[test]
    fn run_outcomes() {
        set_lang(Lang::En);
        let clean = json!({"completed": [{"id": "a", "status": "done", "job_type": "wallet_sync"}], "journals": {"status": "processed", "quarantines": 0}});
        assert!(!needs_attention(&clean) && !has_pending(&clean));
        assert_eq!(summarize_run(&clean), "1 completed");
        let failed = json!({"completed": [{"id": "a", "status": "error", "source_label": "Cold", "error": {"message": "timeout", "hint": "retry later"}}]});
        assert!(needs_attention(&failed));
        assert_eq!(
            summarize_run(&failed),
            "1 needs attention: Cold: timeout retry later"
        );
        let pending = json!({"enqueued": [{"id": "a"}, {"id": "b"}], "completed": [{"id": "a", "status": "done"}]});
        assert!(has_pending(&pending));
        let journals = json!({"completed": [{"job_type": "journal_refresh", "status": "done", "result": {"quarantined": 3, "auto_pair": {"applied": 2, "remaining": {"total": 1}}}}]});
        assert_eq!(quarantine_count(&journals), 3);
        assert_eq!(
            summarize_run(&journals),
            "1 completed, 2 pairs applied, 1 swap/transfer candidate to review, 3 quarantined transactions"
        );
    }

    #[test]
    fn wallet_sync_sentences_carry_the_journal_step() {
        set_lang(Lang::En);
        let r = json!({"wallet": "Cold", "status": "synced", "imported": 2, "unchanged": 10, "journal_invalidated": true});
        assert_eq!(
            describe_wallet_sync(Some(&r), "Cold", &Value::Null),
            "Cold refreshed: 2 imported, 10 unchanged · journals marked stale."
        );
        let j = json!({"status": "processed", "quarantines": 2, "auto_pair_applied": 1});
        assert_eq!(
            describe_wallet_sync(Some(&r), "Cold", &j),
            "Cold refreshed: 2 imported, 10 unchanged. Journals updated; 2 transactions in quarantine. 1 exact transfer pair applied."
        );
        let e = json!({"wallet": "Cold", "status": "error", "message": "connection refused"});
        assert_eq!(
            describe_wallet_sync(
                Some(&e),
                "Cold",
                &json!({"status": "failed", "error": {"code": "locked"}})
            ),
            "Cold refresh failed: connection refused Journals could not be updated: locked"
        );
        assert_eq!(
            describe_wallet_sync(None, "Cold", &Value::Null),
            tr(
                "connections:detail.sync.missingResult",
                &[("label", "Cold")]
            )
        );
        set_lang(Lang::De);
        let de = describe_journal_step(&json!({"status": "current"})).unwrap();
        assert_ne!(de, "Journals were already current.");
        set_lang(Lang::En);
    }

    #[test]
    fn journal_phases_and_counts() {
        set_lang(Lang::En);
        assert_eq!(
            t(journal_phase_key(Some("building"))),
            "Building journal entries"
        );
        assert_eq!(t(journal_phase_key(Some("unknown"))), "Processing journals");
        assert_eq!(
            journal_counts(
                &json!({"processed_transactions": 1, "entries_created": 4, "quarantined": 0})
            )
            .as_deref(),
            Some("1 transaction, 4 entries")
        );
        assert_eq!(journal_counts(&json!({})), None);
    }
}
