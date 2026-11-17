//! Localized text for what the shared model (`model.rs`) derives in English. The model keeps
//! its rules (readiness order, stat status keys, completeness copy); this layer picks the web's
//! i18n key for each result and re-derives the count-bearing strings from the same inputs, so
//! the shared model stays untouched.

use crate::format;
use crate::i18n::{t, tc, tr};
use crate::model::{Flow, Readiness, Snapshot, TxStatus};

/// Title and detail of the readiness pill (`buildOverviewReadiness`, keys `overview:readiness.*`).
pub fn readiness(
    r: &Readiness,
    s: &Snapshot,
    review: Option<f64>,
    blocking: usize,
) -> (String, String) {
    let total = s.connections.len();
    let synced = s
        .connections
        .iter()
        .filter(|c| c.status == "synced")
        .count();
    let source_detail = if total == 0 {
        t("overview:readiness.noSources")
    } else {
        tc(
            "overview:readiness.sourceDetail",
            total,
            &[
                ("synced", &synced.to_string()),
                ("total", &total.to_string()),
            ],
        )
    };
    let quarantines = s.status.as_ref().map(|x| x.quarantines).unwrap_or(0.0);
    match r.title.as_str() {
        "Connect a source" => (
            t("overview:readiness.connectSource.title"),
            t("overview:readiness.connectSource.detail"),
        ),
        "Source attention" => {
            let n = s.connections.iter().filter(|c| c.status == "error").count();
            (
                t("overview:readiness.sourceAttention.title"),
                tc("overview:readiness.sourceAttention.detail", n, &[]),
            )
        }
        "Sources block reports" => (
            t("overview:readiness.sourcesBlocking.title"),
            tc("overview:readiness.sourcesBlocking.detail", blocking, &[]),
        ),
        "Reprocess journals" => (
            t("overview:readiness.reprocessJournals.title"),
            t("overview:readiness.reprocessJournals.detail"),
        ),
        "Review queue open" => {
            let n = review.unwrap_or(quarantines) as usize;
            (
                t("overview:readiness.reviewQueueOpen.title"),
                tc("overview:readiness.reviewQueueOpen.detail", n, &[]),
            )
        }
        "Custody gaps open" => (
            t("overview:readiness.custodyUnresolved.title"),
            t("overview:readiness.custodyUnresolved.detail"),
        ),
        "Prices missing" => {
            let n = s.fiat.completeness.missing_price_count as usize;
            (
                t("overview:readiness.missingPrices.title"),
                tc("overview:readiness.missingPrices.detail", n, &[]),
            )
        }
        "Sync in progress" => (t("overview:readiness.syncInProgress.title"), source_detail),
        _ => (t("overview:readiness.readyForReports.title"), source_detail),
    }
}

/// The Portfolio value tile's status key (`statStatusKey`).
pub fn stat_status(status: &str) -> String {
    match status {
        "No market rate" => t("overview:completeness.badge.noMarketRate"),
        "Outdated" => t("overview:completeness.badge.stale"),
        "Unavailable" => t("overview:completeness.badge.unavailable"),
        "Incomplete" => t("overview:completeness.badge.incomplete"),
        "Needs journals" => t("overview:stats.status.needsJournals"),
        "Clear" => t("overview:stats.status.clear"),
        "Estimate" => t("overview:stats.status.estimate"),
        other => {
            // "{n} quarantine(s)".
            let n: usize = other
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            tc("overview:stats.status.reviewQuarantines", n, &[])
        }
    }
}

/// `vs cost basis` / `from loaded rows`.
pub fn comparison(c: &str) -> String {
    t(if c == "vs cost basis" {
        "overview:stats.comparison.vsCostBasis"
    } else {
        "overview:stats.comparison.fromLoadedRows"
    })
}

/// The completeness copy and hint (`portfolioCompletenessDetail`).
pub fn completeness(s: &Snapshot) -> Option<(String, String)> {
    let c = &s.fiat.completeness;
    if c.market_rate_missing {
        return Some((
            t("overview:completeness.valueUnavailable"),
            t("overview:completeness.hint.unavailable"),
        ));
    }
    if c.cost_basis_complete {
        return None;
    }
    let stale = c.state == "stale";
    let copy = if stale {
        t("overview:completeness.basisStale")
    } else if let Some(m) = c.basis_uncovered_msat.filter(|m| *m > 0.0) {
        tr(
            "overview:completeness.uncovered",
            &[("amount", &format::btc(m / 1e11, 8, false))],
        )
    } else {
        t("overview:completeness.basisIncomplete")
    };
    let hint = t(if stale {
        "overview:completeness.hint.stale"
    } else {
        "overview:completeness.hint.incomplete"
    });
    Some((copy, hint))
}

/// `marketRateCompactLabel` with the web's translated relative times.
pub fn market_caption(s: &Snapshot, now: f64) -> (String, bool) {
    let m = s.market_rate.as_ref();
    let when = m.and_then(|m| m.fetched_at.clone().or_else(|| m.timestamp.clone()));
    let time = when.as_deref().and_then(|w| relative(w, now));
    let source = m.and_then(|m| m.source.as_deref()).map(source_label);
    let parts: Vec<String> = [source, time].into_iter().flatten().collect();
    if parts.is_empty() {
        (t("overview:marketRate.fetchRates"), false)
    } else {
        (parts.join(" · "), when.is_some())
    }
}

fn source_label(source: &str) -> String {
    if source == "manual" {
        t("overview:marketRate.manual")
    } else {
        crate::model::source_label(source)
    }
}

/// `just now` / `{{count}}m ago` / `{{count}}h ago` / `{{count}}d ago` (`overview:marketRate.*`).
pub fn relative(ts: &str, now: f64) -> Option<String> {
    let dt = (now - format::parse_ts(ts)?).max(0.0);
    Some(match dt {
        d if d < 60.0 => t("overview:marketRate.justNow"),
        d if d < 3600.0 => tc(
            "overview:marketRate.minutesAgo",
            (d / 60.0).floor() as usize,
            &[],
        ),
        d if d < 86_400.0 => tc(
            "overview:marketRate.hoursAgo",
            (d / 3600.0).floor() as usize,
            &[],
        ),
        d => tc(
            "overview:marketRate.daysAgo",
            (d / 86_400.0).floor() as usize,
            &[],
        ),
    })
}

/// `formatMarketRateValue`: `€ 71.420,18 / BTC` or `No EUR rate`.
pub fn market_value(s: &Snapshot, cur: &str) -> String {
    match s.market_rate_value() {
        Some(r) => tr(
            "overview:marketRate.perBtc",
            &[("value", &format::fiat(r, cur))],
        ),
        None => tr("overview:marketRate.noRate", &[("currency", cur)]),
    }
}

pub fn flow(f: Flow) -> String {
    t(match f {
        Flow::Incoming => "overview:txFlow.incoming",
        Flow::Outgoing => "overview:txFlow.outgoing",
        Flow::Transfer => "overview:txFlow.transfer",
        Flow::Swap => "overview:txFlow.swap",
    })
}

pub fn status(s: TxStatus) -> String {
    t(match s {
        TxStatus::Confirmed => "overview:txStatus.confirmed",
        TxStatus::Pending => "overview:txStatus.pending",
        TxStatus::Review => "overview:txStatus.review",
        TxStatus::Failed => "overview:txStatus.failed",
    })
}

/// The recent row's type label: the model gives the first tag or the English flow label; a
/// flow label is replaced by its translation (tags are data and stay as they are).
pub fn type_label(label: &str, f: Flow) -> String {
    if label == crate::model::flow_label(f) {
        flow(f)
    } else {
        transaction_type(label)
    }
}

/// `transactionTypeLabel` (lib/transactionTypeLabel.ts): the daemon's stable English type
/// labels map to `transactions:type.*`; anything else is a user tag and stays verbatim.
pub fn transaction_type(label: &str) -> String {
    let key = match label {
        "Buy" => "buy",
        "Deposit" => "deposit",
        "Income" => "income",
        "Wages" => "wages",
        "Mining" => "mining",
        "Staking" => "staking",
        "Interest" => "interest",
        "Airdrop" => "airdrop",
        "Hard fork" => "hardFork",
        "Acquired" => "acquired",
        "LN invoice" => "lnInvoice",
        "Channel close" => "channelClose",
        "Sell" => "sell",
        "Withdrawal" => "withdrawal",
        "Spend" => "spend",
        "Gift" => "gift",
        "Donation" => "donation",
        "Lost" => "lost",
        "Stolen" => "stolen",
        "Expense" => "expense",
        "LN payment" => "lnPayment",
        "Channel open" => "channelOpen",
        "Transfer" => "transfer",
        "Swap" => "swap",
        "Fee" => "fee",
        _ => return label.to_owned(),
    };
    t(&format!("transactions:type.{key}"))
}
