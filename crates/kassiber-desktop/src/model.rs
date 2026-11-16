//! `ui.overview.snapshot` as typed data, plus the derived values the dashboard shows
//! (a port of ui-tauri/src/components/overview-dashboard/model.ts and normalizeUiSnapshots.ts).
//!
//! Deserialization is lenient like the web normalizer: missing or null fields take defaults,
//! and unknown fields are ignored.

use serde::{Deserialize, Deserializer};

fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// A finite number or 0 (`finiteNumber`); strings and nulls become 0.
fn num<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|x| x.is_finite()).unwrap_or(0.0))
}

fn opt_num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|x| x.is_finite()))
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    #[serde(deserialize_with = "num")]
    pub price_eur: f64,
    #[serde(deserialize_with = "num")]
    pub price_usd: f64,
    pub market_rate: Option<MarketRate>,
    #[serde(deserialize_with = "nullable")]
    pub connections: Vec<Connection>,
    #[serde(deserialize_with = "nullable")]
    pub txs: Vec<Tx>,
    #[serde(deserialize_with = "nullable")]
    pub activity_txs: Vec<Tx>,
    #[serde(deserialize_with = "nullable")]
    pub balance_series: Vec<f64>,
    #[serde(deserialize_with = "nullable")]
    pub portfolio_series: Vec<PortfolioPoint>,
    #[serde(deserialize_with = "nullable")]
    pub fiat: Fiat,
    pub balance_summary: Option<BalanceSummary>,
    pub status: Option<BookStatus>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarketRate {
    pub fiat_currency: Option<String>,
    pub pair: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub rate: Option<f64>,
    pub timestamp: Option<String>,
    pub source: Option<String>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub kind: String,
    #[serde(deserialize_with = "nullable")]
    pub label: String,
    #[serde(deserialize_with = "nullable")]
    pub last: String,
    pub last_sync_at: Option<String>,
    #[serde(deserialize_with = "num")]
    pub balance: f64,
    #[serde(deserialize_with = "nullable")]
    pub status: String,
    #[serde(deserialize_with = "num")]
    pub transaction_count: f64,
    #[serde(deserialize_with = "nullable")]
    pub chain: String,
    pub network: Option<String>,
    pub deprecated: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pair {
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub kind: String,
    #[serde(deserialize_with = "nullable")]
    pub out_wallet: String,
    #[serde(deserialize_with = "num")]
    pub out_amount_sat: f64,
    #[serde(deserialize_with = "nullable")]
    pub in_wallet: String,
    #[serde(deserialize_with = "num")]
    pub in_amount_sat: f64,
    #[serde(deserialize_with = "num")]
    pub fee_sat: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tx {
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub external_id: String,
    pub explorer_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub date: String,
    pub occurred_at: Option<String>,
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub tx_type: String,
    #[serde(deserialize_with = "nullable")]
    pub kind: String,
    #[serde(deserialize_with = "nullable")]
    pub asset: String,
    #[serde(deserialize_with = "nullable")]
    pub chain: String,
    pub network: Option<String>,
    pub payment_method: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub account: String,
    #[serde(deserialize_with = "nullable")]
    pub counter: String,
    #[serde(deserialize_with = "num")]
    pub amount_sat: f64,
    #[serde(deserialize_with = "num")]
    pub fee_sat: f64,
    #[serde(deserialize_with = "opt_num")]
    pub eur: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub rate: Option<f64>,
    pub fiat_currency: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub tag: String,
    #[serde(deserialize_with = "nullable")]
    pub tags: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub note: String,
    pub excluded: bool,
    #[serde(deserialize_with = "num")]
    pub conf: f64,
    pub internal: bool,
    pub queue_status: Option<String>,
    pub review_status: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub balance_btc: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub cost_basis_eur: Option<f64>,
    pub pair: Option<Pair>,
    pub summary_only: bool,
    pub quarantine_reason: Option<String>,
    pub pricing_provider: Option<String>,
    pub pricing_pair: Option<String>,
    pub pricing_timestamp: Option<String>,
    pub pricing_quality: Option<String>,
    pub pricing_source_kind: Option<String>,
    pub taxable: Option<bool>,
    pub at_category: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PortfolioPoint {
    #[serde(deserialize_with = "nullable")]
    pub date: String,
    #[serde(deserialize_with = "num")]
    pub balance_btc: f64,
    #[serde(deserialize_with = "num")]
    pub value_eur: f64,
    #[serde(deserialize_with = "num")]
    pub cost_basis_eur: f64,
    #[serde(deserialize_with = "opt_num")]
    pub price_eur: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fiat {
    pub fiat_currency: Option<String>,
    #[serde(deserialize_with = "num")]
    pub eur_balance: f64,
    #[serde(deserialize_with = "num")]
    pub eur_cost_basis: f64,
    #[serde(deserialize_with = "num")]
    pub eur_unrealized: f64,
    #[serde(rename = "eurRealizedYTD", deserialize_with = "num")]
    pub eur_realized_ytd: f64,
    #[serde(deserialize_with = "nullable")]
    pub completeness: Completeness,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Completeness {
    #[serde(deserialize_with = "nullable")]
    pub state: String,
    pub cost_basis_complete: bool,
    #[serde(deserialize_with = "nullable")]
    pub reasons: Vec<String>,
    #[serde(deserialize_with = "num")]
    pub quarantine_count: f64,
    #[serde(deserialize_with = "opt_num")]
    pub basis_uncovered_msat: Option<f64>,
    pub earliest_incomplete_at: Option<String>,
    #[serde(deserialize_with = "num")]
    pub missing_price_count: f64,
    pub market_rate_missing: bool,
}

impl Default for Completeness {
    /// `UNKNOWN_FIAT_COMPLETENESS`.
    fn default() -> Self {
        Completeness {
            state: "unavailable".into(),
            cost_basis_complete: false,
            reasons: vec![],
            quarantine_count: 0.0,
            basis_uncovered_msat: None,
            earliest_incomplete_at: None,
            missing_price_count: 0.0,
            market_rate_missing: false,
        }
    }
}

impl Completeness {
    /// `normalizeFiatCompleteness`.
    fn normalize(&mut self) {
        if !matches!(
            self.state.as_str(),
            "complete" | "incomplete" | "stale" | "unavailable"
        ) {
            self.state = "unavailable".into();
        }
        const ORDER: [&str; 5] = [
            "journals_stale",
            "quarantines",
            "custody_unresolved",
            "missing_prices",
            "market_rate_missing",
        ];
        self.reasons = ORDER
            .iter()
            .filter(|r| self.reasons.iter().any(|x| x == *r))
            .map(|r| r.to_string())
            .collect();
        self.cost_basis_complete =
            self.cost_basis_complete && !matches!(self.state.as_str(), "stale" | "incomplete");
        self.quarantine_count = self.quarantine_count.max(0.0);
        self.missing_price_count = self.missing_price_count.max(0.0);
        self.basis_uncovered_msat = self.basis_uncovered_msat.map(|v| v.max(0.0));
        if self
            .earliest_incomplete_at
            .as_deref()
            .is_some_and(str::is_empty)
        {
            self.earliest_incomplete_at = None;
        }
    }
    pub fn has(&self, reason: &str) -> bool {
        self.reasons.iter().any(|r| r == reason)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BalanceSummary {
    #[serde(deserialize_with = "num")]
    pub total_btc: f64,
    pub needs_journals: bool,
    #[serde(deserialize_with = "num")]
    pub quarantines: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BookStatus {
    pub workspace: Option<String>,
    pub profile: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub transaction_count: Option<f64>,
    pub needs_journals: bool,
    #[serde(deserialize_with = "num")]
    pub quarantines: f64,
}

impl Snapshot {
    pub fn parse(data: &serde_json::Value) -> Result<Snapshot, String> {
        let mut s: Snapshot = Snapshot::deserialize(data).map_err(|e| e.to_string())?;
        s.fiat.completeness.normalize();
        s.balance_series.retain(|v| v.is_finite());
        Ok(s)
    }

    /// `activeMarketFiatCurrency`.
    pub fn fiat_currency(&self) -> String {
        self.market_rate
            .as_ref()
            .and_then(|m| m.fiat_currency.clone())
            .or_else(|| self.fiat.fiat_currency.clone())
            .unwrap_or_else(|| "EUR".into())
            .to_uppercase()
    }

    /// `activeMarketFiatRate`: the rate for every fiat↔BTC conversion.
    pub fn rate(&self) -> f64 {
        match self.market_rate.as_ref().and_then(|m| m.rate) {
            Some(r) if r > 0.0 => r,
            _ if self.fiat_currency() == "USD" => self.price_usd,
            _ => self.price_eur,
        }
    }

    pub fn market_rate_value(&self) -> Option<f64> {
        self.market_rate
            .as_ref()
            .and_then(|m| m.rate)
            .filter(|r| *r > 0.0)
    }

    /// `latestPortfolioBalanceBtc`.
    pub fn latest_balance_btc(&self) -> f64 {
        if let Some(p) = self
            .portfolio_series
            .iter()
            .max_by(|a, b| a.date.cmp(&b.date))
        {
            return p.balance_btc;
        }
        let rate = self.rate();
        let from_fiat = if rate > 0.0 {
            self.fiat.eur_balance / rate
        } else {
            0.0
        };
        if from_fiat > 0.0 {
            return from_fiat;
        }
        self.balance_series.last().copied().unwrap_or(from_fiat)
    }

    pub fn needs_journals(&self) -> bool {
        self.balance_summary
            .as_ref()
            .map(|b| b.needs_journals)
            .unwrap_or(false)
            || self.status.as_ref().is_some_and(|s| s.needs_journals)
    }

    pub fn quarantines(&self) -> f64 {
        self.balance_summary
            .as_ref()
            .map(|b| b.quarantines)
            .or_else(|| self.status.as_ref().map(|s| s.quarantines))
            .unwrap_or(0.0)
    }

    /// The display label of the active book (`status.profile ?? status.workspace`).
    pub fn book_label(&self) -> Option<String> {
        self.status
            .as_ref()
            .and_then(|s| s.profile.clone().or_else(|| s.workspace.clone()))
    }
}

// ---- Readiness (WelcomeSection) -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneKind {
    Good,
    Warning,
    Alert,
    Neutral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessIcon {
    Plus,
    WalletCards,
    RefreshCw,
    ShieldAlert,
    CheckCircle,
}

#[derive(Clone, Debug)]
pub struct Readiness {
    pub title: String,
    pub href: Option<&'static str>,
    pub icon: ReadinessIcon,
    pub tone: ToneKind,
    /// Rule 1 hides the pill and shows only the detail.
    pub connect_source: bool,
    /// Rule 4 is a button that runs journal processing.
    pub reprocess: bool,
}

/// `buildOverviewReadiness`: the first matching rule wins.
pub fn readiness(s: &Snapshot, blocking_sources: usize) -> Readiness {
    let r = |title: &str, href, icon, tone| Readiness {
        title: title.into(),
        href,
        icon,
        tone,
        connect_source: false,
        reprocess: false,
    };
    let total = s.connections.len();
    if s.txs.is_empty() && total == 0 {
        let mut x = r(
            "Connect a source",
            None,
            ReadinessIcon::Plus,
            ToneKind::Neutral,
        );
        x.connect_source = true;
        return x;
    }
    let errors = s.connections.iter().filter(|c| c.status == "error").count();
    if errors > 0 {
        return r(
            "Source attention",
            Some("/connections"),
            ReadinessIcon::WalletCards,
            ToneKind::Alert,
        );
    }
    if blocking_sources > 0 {
        return r(
            "Sources block reports",
            Some("/connections"),
            ReadinessIcon::WalletCards,
            ToneKind::Alert,
        );
    }
    if s.status.as_ref().is_some_and(|x| x.needs_journals) {
        let mut x = r(
            "Reprocess journals",
            None,
            ReadinessIcon::RefreshCw,
            ToneKind::Warning,
        );
        x.reprocess = true;
        return x;
    }
    let q = s.status.as_ref().map(|x| x.quarantines).unwrap_or(0.0);
    if q > 0.0 {
        return r(
            "Review queue open",
            Some("/review"),
            ReadinessIcon::ShieldAlert,
            ToneKind::Alert,
        );
    }
    let c = &s.fiat.completeness;
    if c.has("custody_unresolved") {
        return r(
            "Custody gaps open",
            Some("/journals"),
            ReadinessIcon::ShieldAlert,
            ToneKind::Alert,
        );
    }
    if c.has("missing_prices") {
        return r(
            "Prices missing",
            Some("/review"),
            ReadinessIcon::ShieldAlert,
            ToneKind::Warning,
        );
    }
    if s.connections.iter().any(|c| c.status == "syncing") {
        return r(
            "Sync in progress",
            Some("/connections"),
            ReadinessIcon::RefreshCw,
            ToneKind::Warning,
        );
    }
    r(
        "Ready for reports",
        Some("/reports"),
        ReadinessIcon::CheckCircle,
        ToneKind::Good,
    )
}

// ---- Stats cards ----------------------------------------------------------------------------

/// `marketRateCompactLabel`'s source names.
pub fn source_label(source: &str) -> String {
    let named = match source {
        "coinbase-exchange" => "Coinbase Exchange",
        "kraken-csv" => "Kraken CSV",
        "coingecko" => "CoinGecko",
        "manual" => "Manual",
        "mempool" => "mempool",
        other => other,
    };
    named.strip_suffix(" Exchange").unwrap_or(named).to_owned()
}

/// `marketRateDetailLabel`: `Source · pair`.
pub fn market_rate_detail(s: &Snapshot) -> String {
    let m = s.market_rate.as_ref();
    let source = m.and_then(|m| m.source.as_deref()).map(source_label);
    let pair = m.and_then(|m| m.pair.clone());
    match (source, pair) {
        (Some(s), Some(p)) => format!("{s} · {p}"),
        (Some(s), None) => s,
        (None, Some(p)) => p,
        (None, None) => "Fetch rates".into(),
    }
}

#[derive(Clone, Debug)]
pub struct CompletenessIssue {
    pub href: Option<&'static str>,
}

/// `completenessHref`.
pub fn completeness_href(c: &Completeness) -> &'static str {
    if c.state == "stale" || (c.quarantine_count == 0.0 && c.has("custody_unresolved")) {
        "/journals"
    } else {
        "/review"
    }
}

/// `portfolioCompletenessDetail`: whether the figures carry an issue, and where it resolves (the
/// localized copy is `overview::l10n`'s).
pub fn completeness_issue(s: &Snapshot) -> Option<CompletenessIssue> {
    let c = &s.fiat.completeness;
    if c.market_rate_missing {
        return Some(CompletenessIssue { href: None });
    }
    if c.cost_basis_complete {
        return None;
    }
    Some(CompletenessIssue {
        href: Some(completeness_href(c)),
    })
}

#[derive(Clone, Debug)]
pub struct PortfolioStat {
    pub value: f64,
    pub change_percent: f64,
    pub positive: bool,
    pub comparison: &'static str,
    /// A status key replacing the percent (`Needs journals`, `Incomplete`, …).
    pub status: Option<String>,
    pub href: &'static str,
    pub issue: Option<CompletenessIssue>,
}

/// `buildPortfolioStat` + `statStatusKey`.
pub fn portfolio_stat(s: &Snapshot) -> PortfolioStat {
    let c = &s.fiat.completeness;
    let f = &s.fiat;
    let change_percent = if c.cost_basis_complete && f.eur_cost_basis != 0.0 {
        f.eur_unrealized / f.eur_cost_basis * 100.0
    } else {
        0.0
    };
    let issue = completeness_issue(s);
    let badge = if c.market_rate_missing {
        Some("No market rate".to_owned())
    } else if c.cost_basis_complete {
        None
    } else if c.state == "stale" {
        Some("Outdated".into())
    } else if c.state == "unavailable" {
        Some("Unavailable".into())
    } else {
        Some("Incomplete".into())
    };
    let q = s.quarantines();
    let status = badge.or_else(|| {
        if s.needs_journals() {
            Some("Needs journals".into())
        } else if q > 0.0 {
            Some(if q == 1.0 {
                "1 quarantine".into()
            } else {
                format!("{} quarantines", q as usize)
            })
        } else if f.eur_cost_basis > 0.0 {
            None
        } else if f.eur_balance == 0.0 {
            Some("Clear".into())
        } else {
            Some("Estimate".into())
        }
    });
    let href = issue
        .as_ref()
        .and_then(|i| i.href)
        .unwrap_or(if s.needs_journals() {
            "/journals"
        } else if q > 0.0 {
            "/review"
        } else {
            "/reports"
        });
    PortfolioStat {
        value: f.eur_balance,
        change_percent,
        positive: f.eur_unrealized >= 0.0,
        comparison: if f.eur_cost_basis != 0.0 {
            "vs cost basis"
        } else {
            "from loaded rows"
        },
        status,
        href,
        issue,
    }
}

// ---- Holdings by source ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    Onchain,
    Lightning,
    Liquid,
    None,
}

// ---- Transactions ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Incoming,
    Outgoing,
    Transfer,
    Swap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatus {
    Confirmed,
    Pending,
    Review,
    Failed,
}

/// `flowForOverviewTx`.
pub fn flow(tx: &Tx) -> Flow {
    match tx.tx_type.as_str() {
        _ if tx.internal => Flow::Transfer,
        "Transfer" | "Consolidation" | "Rebalance" => Flow::Transfer,
        "Swap" | "Mint" | "Melt" => Flow::Swap,
        _ if tx.amount_sat >= 0.0 => Flow::Incoming,
        _ => Flow::Outgoing,
    }
}

/// `statusForOverviewTx`.
pub fn tx_status(tx: &Tx) -> TxStatus {
    match tx.queue_status.as_deref() {
        Some("review") => TxStatus::Review,
        Some("pending") => TxStatus::Pending,
        Some("failed") => TxStatus::Failed,
        Some("completed") => TxStatus::Confirmed,
        _ if tx.internal => TxStatus::Pending,
        _ if tx.conf > 0.0 => TxStatus::Confirmed,
        _ if tx.tag.to_lowercase().contains("review") => TxStatus::Review,
        _ => TxStatus::Pending,
    }
}

pub fn flow_label(f: Flow) -> &'static str {
    match f {
        Flow::Incoming => "Incoming",
        Flow::Outgoing => "Outgoing",
        Flow::Transfer => "Transfer",
        Flow::Swap => "Swap",
    }
}

pub fn status_label(s: TxStatus) -> &'static str {
    match s {
        TxStatus::Confirmed => "Confirmed",
        TxStatus::Pending => "Pending",
        TxStatus::Review => "Review",
        TxStatus::Failed => "Failed",
    }
}

/// One row of the Recent Transactions card (`toDashboardTransaction`).
#[derive(Clone, Debug)]
pub struct RecentRow {
    pub id: String,
    pub txid: String,
    pub date: String,
    pub counterparty: String,
    pub type_label: String,
    pub extra_tags: usize,
    pub status: TxStatus,
    pub flow: Flow,
    pub amount_btc: f64,
    pub amount_fiat: Option<f64>,
    pub fiat_currency: Option<String>,
}

pub fn recent_rows(s: &Snapshot) -> Vec<RecentRow> {
    s.txs
        .iter()
        .map(|tx| {
            let display_sat =
                if tx.tx_type == "Consolidation" && tx.amount_sat == 0.0 && tx.fee_sat != 0.0 {
                    -tx.fee_sat.abs()
                } else {
                    tx.amount_sat
                };
            let amount_fiat = tx.eur.or_else(|| tx.rate.map(|r| display_sat / 1e8 * r));
            let tags: Vec<String> = if tx.tag.trim().is_empty() {
                vec![tx.tx_type.clone()]
            } else {
                tx.tag
                    .split(',')
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .collect()
            };
            let status = tx_status(tx);
            let f = flow(tx);
            let first = tags.first().cloned().unwrap_or_default();
            let dup = first.eq_ignore_ascii_case(status_label(status))
                || first.eq_ignore_ascii_case(&format!("{status:?}"));
            let type_label = if first.is_empty() || dup {
                flow_label(f).to_owned()
            } else {
                first
            };
            let counterparty = [&tx.account, &tx.counter]
                .into_iter()
                .find(|v| !v.is_empty())
                .cloned()
                .unwrap_or_else(|| "Unassigned".into());
            RecentRow {
                id: tx.id.clone(),
                txid: if tx.external_id.is_empty() {
                    tx.id.clone()
                } else {
                    tx.external_id.clone()
                },
                date: tx.date.clone(),
                counterparty,
                type_label,
                extra_tags: tags.len().saturating_sub(1),
                status,
                flow: f,
                amount_btc: display_sat / 1e8,
                amount_fiat,
                fiat_currency: tx.fiat_currency.clone(),
            }
        })
        .collect()
}

// ---- Review badges --------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Badges {
    #[serde(deserialize_with = "opt_num")]
    pub quarantine: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub quarantine_attention: Option<f64>,
    pub journals_needs_processing: bool,
    #[serde(deserialize_with = "opt_num")]
    pub swaps: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub review: Option<f64>,
}

impl Badges {
    /// `reviewTotal`.
    pub fn total(&self) -> f64 {
        self.review.unwrap_or_else(|| {
            self.quarantine_attention.or(self.quarantine).unwrap_or(0.0) + self.swaps.unwrap_or(0.0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_rules_follow_the_web_order() {
        let mut s = Snapshot::default();
        assert!(readiness(&s, 0).connect_source);
        s.connections.push(Connection {
            status: "synced".into(),
            balance: 1.0,
            ..Default::default()
        });
        s.fiat.completeness = Completeness {
            state: "complete".into(),
            cost_basis_complete: true,
            ..Default::default()
        };
        let r = readiness(&s, 0);
        assert_eq!(r.title, "Ready for reports");
        s.status = Some(BookStatus {
            needs_journals: true,
            ..Default::default()
        });
        assert!(readiness(&s, 0).reprocess);
        assert_eq!(readiness(&s, 2).title, "Sources block reports");
    }
}
