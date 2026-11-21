//! Rows, filters, periods and the server args of the Transactions screen
//! (components/transactions/dashboard/model.ts, components/transactions/model.ts, lib/period.ts).
//!
//! Times are UTC throughout: the web uses the viewer's local time, and the demo book's
//! accounting timezone is UTC, so both agree there.

use serde_json::{Map, Value, json};

use crate::format::{self, Day};
use crate::i18n::{t, tr};

// ---- Periods ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Range {
    D30,
    /// Year to date (owner request; the web offers it only through `?period=ytd`).
    Ytd,
    Y1,
    Y4,
    Y10,
    All,
}

impl Range {
    pub const ALL: [Range; 6] = [
        Range::D30,
        Range::Ytd,
        Range::Y1,
        Range::Y4,
        Range::Y10,
        Range::All,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Range::D30 => "30days",
            Range::Ytd => "ytd",
            Range::Y1 => "1year",
            Range::Y4 => "4years",
            Range::Y10 => "10years",
            Range::All => "all",
        }
    }
    pub fn label(self) -> String {
        match self {
            Range::Ytd => t("chrome:range.ytd"),
            _ => t(&format!("transactions:period.{}", self.key())),
        }
    }
    fn years(self) -> f64 {
        match self {
            Range::Y4 => 4.0,
            Range::Y10 => 10.0,
            _ => 0.0,
        }
    }
}

/// `PeriodKey`: `auto`, a range, or one calendar year.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Period {
    Auto,
    Range(Range),
    Year(i32),
}

impl Period {
    pub fn param(self) -> String {
        match self {
            Period::Auto => "auto".into(),
            Period::Range(r) => r.key().into(),
            Period::Year(y) => y.to_string(),
        }
    }
    /// `normalizePeriodParam`, human forms included.
    pub fn parse(s: &str) -> Option<Period> {
        let s = s.trim().to_lowercase();
        Some(match s.as_str() {
            "auto" | "automatic" => Period::Auto,
            "30days" | "30d" => Period::Range(Range::D30),
            "1year" | "3m" | "6m" | "12m" | "1y" => Period::Range(Range::Y1),
            "4years" | "4y" | "5y" => Period::Range(Range::Y4),
            "10years" | "10y" | "15y" => Period::Range(Range::Y10),
            "all" | "max" => Period::Range(Range::All),
            "ytd" => Period::Range(Range::Ytd),
            y if y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()) => {
                Period::Year(y.parse().ok()?)
            }
            _ => return None,
        })
    }
}

pub fn today() -> Day {
    Day::from_epoch_days((format::now_secs() / 86_400.0).floor() as i64)
}

/// `{since, until}` as `toISOString()` strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub since: String,
    pub until: String,
}

fn start_iso(d: Day) -> String {
    format!("{}T00:00:00.000Z", d.iso())
}
fn end_iso(d: Day) -> String {
    format!("{}T23:59:59.999Z", d.iso())
}

/// The same calendar day `months` earlier, clamped to the month's length.
fn shift_months(d: Day, months: i32) -> Day {
    let idx = d.y * 12 + d.m as i32 - 1 - months;
    let (y, m) = (idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1);
    let last = Day::from_epoch_days(
        Day {
            y: if m == 12 { y + 1 } else { y },
            m: if m == 12 { 1 } else { m + 1 },
            d: 1,
        }
        .epoch_days()
            - 1,
    )
    .d;
    Day {
        y,
        m,
        d: d.d.min(last),
    }
}

/// A dashboard `history.years` entry.
#[derive(Clone, Debug, Default)]
pub struct BookYear {
    pub year: i32,
    pub count: usize,
    pub since: String,
    pub until: String,
}

/// `transactionPeriodDateWindow`.
pub fn period_window(p: Period, years: &[BookYear]) -> Option<Window> {
    let now = today();
    match p {
        Period::Range(Range::All) => None,
        Period::Auto => period_window(Period::Range(Range::Y1), years),
        Period::Year(y) => Some(match years.iter().find(|e| e.year == y) {
            Some(e) => Window {
                since: e.since.clone(),
                until: e.until.clone(),
            },
            None => Window {
                since: start_iso(Day { y, m: 1, d: 1 }),
                until: end_iso(Day { y, m: 12, d: 31 }),
            },
        }),
        Period::Range(r) => {
            let start = match r {
                Range::D30 => Day::from_epoch_days(now.epoch_days() - 29),
                Range::Ytd => Day {
                    y: now.y,
                    m: 1,
                    d: 1,
                },
                Range::Y4 => shift_months(now, 48),
                Range::Y10 => shift_months(now, 120),
                _ => shift_months(now, 12),
            };
            Some(Window {
                since: start_iso(start),
                until: end_iso(now),
            })
        }
    }
}

/// `historyYearsBetween`: years from the earliest to the later of latest and today.
pub fn history_years(earliest: &str, latest: &str) -> f64 {
    let (Some(a), Some(b)) = (format::parse_ts(earliest), format::parse_ts(latest)) else {
        return 0.0;
    };
    let b = b.max(format::now_secs());
    ((b - a) / (365.2425 * 86_400.0)).max(0.0)
}

/// `selectablePeriods`.
pub fn selectable(history_years: f64) -> Vec<Range> {
    Range::ALL
        .into_iter()
        .filter(|r| r.years() == 0.0 || history_years >= r.years())
        .collect()
}

// ---- Rows ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flow {
    Incoming,
    Outgoing,
    Transfer,
    Swap,
    Layer,
}

impl Flow {
    pub const ALL: [Flow; 5] = [
        Flow::Incoming,
        Flow::Outgoing,
        Flow::Transfer,
        Flow::Swap,
        Flow::Layer,
    ];
    pub fn param(self) -> &'static str {
        match self {
            Flow::Incoming => "incoming",
            Flow::Outgoing => "outgoing",
            Flow::Transfer => "transfer",
            Flow::Swap => "swap",
            Flow::Layer => "layer-transition",
        }
    }
    pub fn parse(s: &str) -> Option<Flow> {
        Flow::ALL
            .into_iter()
            .find(|f| f.param() == s.replace('_', "-"))
    }
    pub fn label(self) -> String {
        t(match self {
            Flow::Incoming => "transactions:flow.incoming",
            Flow::Outgoing => "transactions:flow.outgoing",
            Flow::Transfer => "transactions:flow.transfer",
            Flow::Swap => "transactions:flow.swap",
            Flow::Layer => "transactions:flow.layerTransition",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Completed,
    Pending,
    Failed,
    Review,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Completed,
        Status::Pending,
        Status::Failed,
        Status::Review,
    ];
    pub fn param(self) -> &'static str {
        match self {
            Status::Completed => "completed",
            Status::Pending => "pending",
            Status::Failed => "failed",
            Status::Review => "review",
        }
    }
    /// `normalizeTransactionReviewStatus`.
    pub fn normalize(s: &str) -> Option<Status> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "completed" | "complete" => Some(Status::Completed),
            "pending" => Some(Status::Pending),
            "failed" | "error" => Some(Status::Failed),
            "review" | "needs_review" | "blocked" | "quarantined" => Some(Status::Review),
            _ => None,
        }
    }
    pub fn label(self) -> String {
        t(&format!("transactions:status.{}", self.param()))
    }
}

pub const PAYMENTS: [&str; 4] = ["On-chain", "Exchange", "Lightning", "Liquid"];

/// One transaction row as the table shows it (`toDashboardTransaction` + `draftForTransaction`).
/// Raw fields the table doesn't draw yet are kept for the row menu and detail hand-off.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    /// `externalId || id`.
    pub txn_id: String,
    pub explorer_id: Option<String>,
    pub date: String,
    /// `conf > 0`: names a placeholder `date` "Unknown" rather than "Unconfirmed".
    pub confirmed: bool,
    pub tx_type: String,
    pub wallet: String,
    pub counterparty: String,
    pub payment_method: String,
    pub asset: String,
    pub amount_sat: f64,
    pub fee_sat: f64,
    /// Absolute fiat amount, if priced.
    pub amount_fiat: Option<f64>,
    pub amount_btc: f64,
    pub fee_btc: f64,
    pub fee_fiat: Option<f64>,
    pub rate: Option<f64>,
    pub fiat_currency: String,
    pub pricing_kind: Option<String>,
    pub pricing_quality: Option<String>,
    pub pricing_provider: Option<String>,
    pub pricing_pair: Option<String>,
    pub pricing_granularity: Option<String>,
    pub status: Status,
    pub taxable: Option<bool>,
    pub at_regime: Option<String>,
    pub at_category: Option<String>,
    pub tags: Vec<String>,
    /// The classification label among the tags (`Unlabeled` when none).
    pub label: String,
    pub excluded: bool,
    pub internal: bool,
    pub pair: Option<Value>,
    pub quarantine_reason: Option<String>,
    pub flow: Flow,
    pub chain: String,
    /// A reviewed same-asset move (`pairedTransferRow`).
    pub paired: Option<Paired>,
}

/// `pairedTransferRow`: how the table shows a reviewed same-asset move. The daemon collapses
/// both legs into one row whose `amountSat` is the pair's own fee (zero for a plain move) and
/// whose counter reads "Transfer fee - BTC -> BTC"; the legs' amounts ride along on `pair`.
/// Shown: what arrived in the destination wallet (the source leg's amount when the
/// destination's is missing), valued at the row's rate, and the pair's fee apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Paired {
    pub from: String,
    pub to: String,
    pub moved_btc: f64,
    pub moved_fiat: Option<f64>,
    pub fee_btc: f64,
    pub fee_fiat: Option<f64>,
}

impl Paired {
    pub fn parse(pair: Option<&Value>, rate: Option<f64>) -> Option<Paired> {
        let p = pair?;
        if p.get("type").and_then(Value::as_str) != Some("transfer") {
            return None;
        }
        let nz = |k: &str| on(p, k).filter(|v| *v != 0.0);
        let moved_sat = nz("inAmountSat")
            .or_else(|| nz("outAmountSat"))
            .unwrap_or(0.0)
            .abs();
        let fee_sat = on(p, "feeSat").unwrap_or(0.0).abs();
        let (moved_btc, fee_btc) = (moved_sat / 1e8, fee_sat / 1e8);
        Some(Paired {
            from: s(p, "outWallet").trim().to_owned(),
            to: s(p, "inWallet").trim().to_owned(),
            moved_btc,
            moved_fiat: rate.map(|r| moved_btc * r),
            fee_btc,
            fee_fiat: rate.map(|r| fee_btc * r),
        })
    }
}

const LABELS: [&str; 10] = [
    "Unlabeled",
    "Income",
    "Expense",
    "Transfer",
    "Swap",
    "Fee",
    "Merchant payment",
    "Gift",
    "Review",
    "Other",
];

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}
fn os(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
}
fn n(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .unwrap_or(0.0)
}
fn on(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite())
}

impl Row {
    pub fn parse(v: &Value) -> Row {
        let tx_type = s(v, "type");
        let tag = s(v, "tag");
        let amount_sat = n(v, "amountSat");
        let fee_sat = n(v, "feeSat");
        let display_sat = if tx_type == "Consolidation" && amount_sat == 0.0 && fee_sat != 0.0 {
            fee_sat
        } else {
            amount_sat
        };
        let internal = v.get("internal").and_then(Value::as_bool).unwrap_or(false);
        let low_tag = tag.to_lowercase();
        let flow = if low_tag.contains("swap")
            || matches!(tx_type.as_str(), "Swap" | "Mint" | "Melt")
        {
            Flow::Swap
        } else if internal || matches!(tx_type.as_str(), "Transfer" | "Consolidation" | "Rebalance")
        {
            Flow::Transfer
        } else if amount_sat >= 0.0 {
            Flow::Incoming
        } else {
            Flow::Outgoing
        };
        let conf = n(v, "conf");
        let fallback = if low_tag.contains("review") {
            Status::Review
        } else if conf > 0.0 {
            Status::Completed
        } else {
            Status::Pending
        };
        let queue = os(v, "queueStatus").and_then(|q| Status::normalize(&q));
        let review = os(v, "reviewStatus").and_then(|q| Status::normalize(&q));
        let rate = on(v, "rate").filter(|r| *r > 0.0);
        let paired = Paired::parse(v.get("pair").filter(|p| p.is_object()), on(v, "rate"));
        let eur = on(v, "eur");
        let amount_fiat = eur
            .map(f64::abs)
            .or_else(|| rate.map(|r| (display_sat / 1e8 * r).abs()));
        let account = s(v, "account");
        let chain = s(v, "chain");
        let payment_method = os(v, "paymentMethod").unwrap_or_else(|| {
            let a = account.to_lowercase();
            if a.contains("lightning")
                || a.contains("phoenix")
                || a.split_whitespace().any(|w| w == "ln")
            {
                "Lightning".into()
            } else if chain == "liquid" || a.contains("liquid") || a.contains("lbtc") {
                "Liquid".into()
            } else {
                "On-chain".into()
            }
        });
        // The table reads the `tags` array; `tag` is the display string the Overview uses.
        // `txn.tags ?? splitDraftTags(txn.tag)`: without a `tags` array, the comma-split `tag`.
        let tags: Vec<String> = match v.get("tags").and_then(Value::as_array) {
            Some(a) => a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            None => tag
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect(),
        };
        let label = LABELS
            .iter()
            .find(|l| tags.iter().any(|t| t == *l))
            .map(|l| l.to_string())
            .unwrap_or_else(|| "Unlabeled".into());
        let mut rest: Vec<String> = Vec::new();
        for tg in tags.iter().filter(|t| **t != label) {
            if !rest.contains(tg) {
                rest.push(tg.clone());
            }
        }
        Row {
            id: s(v, "id"),
            txn_id: os(v, "externalId").unwrap_or_else(|| s(v, "id")),
            explorer_id: os(v, "explorerId"),
            date: s(v, "date"),
            confirmed: conf > 0.0,
            tx_type,
            wallet: if account.is_empty() {
                t("transactions:fallback.unassignedWallet")
            } else {
                account
            },
            counterparty: s(v, "counter"),
            payment_method,
            asset: s(v, "asset"),
            amount_sat,
            fee_sat,
            amount_fiat,
            amount_btc: (display_sat / 1e8).abs(),
            fee_btc: (fee_sat / 1e8).abs(),
            fee_fiat: rate.map(|r| (fee_sat / 1e8 * r).abs()),
            rate,
            fiat_currency: os(v, "fiatCurrency")
                .unwrap_or_else(|| "EUR".into())
                .to_uppercase(),
            pricing_kind: os(v, "pricingSourceKind"),
            pricing_quality: os(v, "pricingQuality"),
            pricing_provider: os(v, "pricingProvider"),
            pricing_pair: os(v, "pricingPair"),
            pricing_granularity: os(v, "pricingGranularity"),
            status: queue.or(review).unwrap_or(fallback),
            taxable: v.get("taxable").and_then(Value::as_bool),
            at_regime: os(v, "atRegime"),
            at_category: os(v, "atCategory"),
            tags: rest,
            label,
            excluded: v.get("excluded").and_then(Value::as_bool).unwrap_or(false),
            internal,
            pair: v.get("pair").filter(|p| p.is_object()).cloned(),
            quarantine_reason: os(v, "quarantineReason"),
            flow,
            chain,
            paired,
        }
    }

    /// The row's title: a reviewed move's "Transfer · A → B", else the counterparty.
    pub fn title(&self) -> String {
        match &self.paired {
            Some(p) if !p.from.is_empty() && !p.to.is_empty() => crate::i18n::tr(
                "transactions:table.pairedTransfer.title",
                &[("from", &p.from), ("to", &p.to)],
            ),
            _ => self.counterparty.clone(),
        }
    }

    /// The amount shown: what a reviewed move moved, else the row's own.
    pub fn shown_btc(&self) -> f64 {
        self.paired
            .as_ref()
            .map(|p| p.moved_btc)
            .unwrap_or(self.amount_btc)
    }

    pub fn shown_fiat(&self) -> Option<f64> {
        match &self.paired {
            Some(p) => p.moved_fiat,
            None => self.amount_fiat,
        }
    }

    /// `displayFlow`: pairing candidates show as their review type.
    pub fn display_flow(&self, c: &Candidates) -> Flow {
        if c.swap.contains(&self.id) {
            Flow::Swap
        } else if c.transfer.contains(&self.id) {
            Flow::Transfer
        } else {
            self.flow
        }
    }

    /// Signed fiat for sorting (outgoing negative).
    pub fn signed_fiat(&self, flow: Flow) -> Option<f64> {
        self.shown_fiat()
            .map(|a| if flow == Flow::Outgoing { -a } else { a })
    }
    pub fn signed_btc(&self, flow: Flow) -> f64 {
        if flow == Flow::Outgoing {
            -self.shown_btc()
        } else {
            self.shown_btc()
        }
    }

    /// Classification badge text, or None when it only repeats the flow (`isRedundantTransactionLabel`).
    pub fn classification(&self, flow: Flow) -> Option<String> {
        // `isRedundantTransactionLabel`: the stored label is an English code, compared with the
        // row's own flow in stable English (a "Transfer" label on an internal move still shows).
        let low = self.label.trim().to_lowercase();
        let stable = match flow {
            Flow::Incoming => "incoming",
            Flow::Outgoing => "outgoing",
            Flow::Transfer => "internal transfer",
            Flow::Swap => "swap",
            Flow::Layer => "bitcoin swap",
        };
        if low.is_empty() || low == "unlabeled" || low == stable {
            return None;
        }
        let key = match self.label.as_str() {
            "Income" => "income",
            "Expense" => "expense",
            "Transfer" => "transfer",
            "Swap" => "swap",
            "Fee" => "fee",
            "Merchant payment" => "merchantPayment",
            "Gift" => "gift",
            "Review" => "review",
            "Other" => "other",
            _ => return Some(self.label.clone()),
        };
        Some(t(&format!("transactions:classification.{key}")))
    }

    /// Short tax classification (`austrianTaxClassificationFor`, with the draft defaults).
    pub fn tax_short(&self, flow: Flow) -> String {
        let pair = match (self.at_regime.as_deref(), self.at_category.as_deref()) {
            (Some(r), Some(c)) => format!("{r}:{c}"),
            _ if self.tx_type == "Fee" => "outside:none".into(),
            _ => match flow {
                Flow::Incoming => "neu:income_general".into(),
                Flow::Transfer => "outside:none".into(),
                Flow::Swap | Flow::Layer => "neu:neu_swap".into(),
                Flow::Outgoing => "neu:neu_gain".into(),
            },
        };
        let key = match pair.as_str() {
            "neu:income_general" => "incomeGeneral",
            "neu:neu_loss" => "neuLoss",
            "outside:none" => "ownWalletTransfer",
            "neu:neu_swap" => "neuSwap",
            "alt:alt_spekulation" => "altSpekulation",
            "alt:alt_taxfree" => "altTaxfree",
            _ => "neuGain",
        };
        t(&format!("transactions:taxClassification.{key}.short"))
    }

    /// Draft default taxable when the row carries none.
    pub fn taxable_or_default(&self, flow: Flow) -> bool {
        self.taxable
            .unwrap_or(!matches!(flow, Flow::Transfer) && self.tx_type != "Fee")
    }

    /// `pricingSelectionValue` → the pill style key and label.
    pub fn pricing_pill(&self) -> (&'static str, String) {
        let kind = self.pricing_kind.as_deref().or(if self.rate.is_some() {
            Some("generic_import")
        } else {
            None
        });
        let quality = self
            .pricing_quality
            .as_deref()
            .unwrap_or(if self.rate.is_some() {
                "exact"
            } else {
                "missing"
            });
        let Some(kind) = kind.filter(|_| quality != "missing") else {
            return ("missing", t("transactions:pricingOption.missing.label"));
        };
        let style = match kind {
            "generic_import" | "exchange_execution" => "source",
            "wallet_export" => "wallet",
            k if k.starts_with("btcpay") => "btcpay",
            "manual_override" => "override",
            _ => "fmv",
        };
        let label = match kind {
            "generic_import" => t("transactions:pricingOption.sourcePrice.label"),
            "fmv_provider" => t("transactions:pricingOption.fmvProvider.label"),
            "manual_override" => t("transactions:pricingOption.manualOverride.label"),
            other => {
                let key = format!("transactions:pricingSourceKind.{other}.label");
                if crate::i18n::has(&key) {
                    t(&key)
                } else {
                    other.to_owned()
                }
            }
        };
        (style, label)
    }

    /// `provider · pair · granularity`.
    pub fn pricing_detail(&self) -> String {
        let provider = self.pricing_provider.as_deref().map(|p| match p {
            "kraken-csv" => "Kraken CSV".to_owned(),
            "coinbase-exchange" => "Coinbase Exchange".to_owned(),
            "coingecko" => "CoinGecko".to_owned(),
            "manual" => t("transactions:pricingProvider.manual"),
            other => other
                .split(['-', '_'])
                .map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" "),
        });
        [
            provider,
            self.pricing_pair.clone(),
            self.pricing_granularity.clone(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }

    /// `pairRailLabel` for swap / layer rows.
    pub fn rail_label(&self) -> String {
        let rail = |txt: &str| {
            let l = txt.to_lowercase();
            if l.contains("lightning")
                || l.contains("phoenix")
                || l.split(|c: char| !c.is_alphanumeric()).any(|w| w == "ln")
            {
                "Lightning"
            } else if l.contains("lbtc") || l.contains("liquid") {
                "Liquid"
            } else if l.contains("btc") || l.contains("onchain") || l.contains("on-chain") {
                "On-chain"
            } else {
                "Other"
            }
        };
        let (out, inn) = match &self.pair {
            Some(p) => (
                format!("{} {}", s(p, "outAsset"), s(p, "outWallet")),
                format!("{} {}", s(p, "inAsset"), s(p, "inWallet")),
            ),
            None => {
                let mut parts = self.wallet.splitn(2, "->");
                (
                    format!("{} {}", self.asset, parts.next().unwrap_or("")),
                    format!("{} {}", self.asset, parts.next().unwrap_or("")),
                )
            }
        };
        let (a, b) = (rail(&out), rail(&inn));
        if a == b {
            a.to_owned()
        } else {
            format!("{a} -> {b}")
        }
    }

    /// The legs of a pair's `Out -> In` wallet text.
    pub fn wallet_legs(&self) -> Vec<String> {
        split_legs(&self.wallet)
    }

    /// Parsed `date` ("YYYY-MM-DD HH:MM") as epoch seconds, for the client date sort.
    pub fn time(&self) -> Option<f64> {
        format::parse_ts(&self.date)
    }
}

/// Splits a wallet text on `→ ⇄ ↔ ⟶ -> =>`.
pub fn split_legs(w: &str) -> Vec<String> {
    let mut out = vec![w.to_owned()];
    for sep in ["→", "⇄", "↔", "⟶", "->", "=>"] {
        out = out
            .iter()
            .flat_map(|p| p.split(sep).map(|x| x.trim().to_owned()))
            .filter(|x| !x.is_empty())
            .collect();
    }
    out
}

/// `formatShortTxid`: ids over 18 characters as `first10...last6`.
pub fn short_txid(id: &str) -> String {
    let c: Vec<char> = id.chars().collect();
    if c.len() <= 18 {
        id.to_owned()
    } else {
        format!(
            "{}...{}",
            c[..10].iter().collect::<String>(),
            c[c.len() - 6..].iter().collect::<String>()
        )
    }
}

// ---- Pairing candidates -------------------------------------------------------------------

/// Leg ids of non-conflicted pairing candidates, by review type (`nonConflictedCandidateRefs`).
#[derive(Clone, Debug, Default)]
pub struct Candidates {
    pub swap: Vec<String>,
    pub transfer: Vec<String>,
}

impl Candidates {
    pub fn from_dashboard(list: &[Value]) -> Candidates {
        let mut sets: std::collections::HashMap<String, usize> = Default::default();
        for c in list {
            if let Some(k) = c.get("conflict_set_id").and_then(Value::as_str) {
                *sets.entry(k.to_owned()).or_default() += 1;
            }
        }
        let mut used: Vec<String> = Vec::new();
        let mut out = Candidates::default();
        for c in list {
            let size = c.get("conflict_size").and_then(Value::as_u64).unwrap_or(1) as usize;
            let cluster = c
                .get("conflict_set_id")
                .and_then(Value::as_str)
                .and_then(|k| sets.get(k).copied())
                .unwrap_or(1);
            let (a, b) = (s(c, "in_id"), s(c, "out_id"));
            if size > 1 || cluster > 1 || used.contains(&a) || used.contains(&b) {
                continue;
            }
            used.push(a.clone());
            used.push(b.clone());
            let kind = os(c, "candidate_type").unwrap_or_else(|| {
                let dk = s(c, "default_kind");
                if [
                    "chain-swap",
                    "peg-in",
                    "peg-out",
                    "reverse-submarine-swap",
                    "submarine-swap",
                    "swap-refund",
                ]
                .contains(&dk.as_str())
                    || s(c, "in_asset") == s(c, "out_asset")
                {
                    "transfer".into()
                } else {
                    "swap".into()
                }
            });
            let target = if kind == "transfer" {
                &mut out.transfer
            } else {
                &mut out.swap
            };
            target.push(a);
            target.push(b);
        }
        out
    }
}

// ---- Filters ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quick {
    ExternalFlow,
    ReviewQueue,
    NoExplorerId,
    MissingPrice,
    FailedImport,
}

impl Quick {
    pub fn param(self) -> &'static str {
        match self {
            Quick::ExternalFlow => "external_flow",
            Quick::ReviewQueue => "review_queue",
            Quick::NoExplorerId => "no_explorer_id",
            Quick::MissingPrice => "missing_price",
            Quick::FailedImport => "failed_import",
        }
    }
    pub fn parse(s: &str) -> Option<Quick> {
        [
            Quick::ExternalFlow,
            Quick::ReviewQueue,
            Quick::NoExplorerId,
            Quick::MissingPrice,
            Quick::FailedImport,
        ]
        .into_iter()
        .find(|q| q.param() == s)
    }
    pub fn label(self) -> String {
        t(match self {
            Quick::ExternalFlow => "transactions:quickFilter.externalFlow",
            Quick::ReviewQueue => "transactions:quickFilter.reviewQueue",
            Quick::NoExplorerId => "transactions:quickFilter.missingExplorerLink",
            Quick::MissingPrice => "transactions:quickFilter.missingPrice",
            Quick::FailedImport => "transactions:quickFilter.failedImport",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breakdown {
    pub network: bool,
    pub key: String,
    /// Wallet matched against each leg of a pair's wallet text.
    pub leg: bool,
}

impl Breakdown {
    pub fn label(&self) -> String {
        if self.network {
            crate::i18n::tr("transactions:selectionLabel.network", &[("key", &self.key)])
        } else {
            crate::i18n::tr(
                "transactions:selectionLabel.walletSource",
                &[("key", &self.key)],
            )
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Segment {
    Incoming,
    Outgoing,
    Transfers,
    Swaps,
}

impl Segment {
    pub const ALL: [Segment; 4] = [
        Segment::Incoming,
        Segment::Outgoing,
        Segment::Transfers,
        Segment::Swaps,
    ];
    pub fn param(self) -> &'static str {
        match self {
            Segment::Incoming => "incoming",
            Segment::Outgoing => "outgoing",
            Segment::Transfers => "transfers",
            Segment::Swaps => "swaps",
        }
    }
    pub fn label(self) -> String {
        t(&format!("transactions:chartSegment.{}", self.param()))
    }
    pub fn flow(self) -> Flow {
        match self {
            Segment::Incoming => Flow::Incoming,
            Segment::Outgoing => Flow::Outgoing,
            Segment::Transfers => Flow::Transfer,
            Segment::Swaps => Flow::Swap,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartSel {
    pub bucket_key: Option<String>,
    pub bucket_label: String,
    pub segment: Option<Segment>,
    /// External (true) or All.
    pub external: bool,
}

impl ChartSel {
    pub fn label(&self) -> String {
        let seg = self
            .segment
            .map(Segment::label)
            .unwrap_or_else(|| t("transactions:selectionLabel.allFlows"));
        let mode = t(if self.external {
            "transactions:chartMode.external"
        } else {
            "transactions:chartMode.all"
        });
        crate::i18n::tr(
            "transactions:selectionLabel.flowChart",
            &[
                ("bucket", &self.bucket_label),
                ("segment", &seg),
                ("mode", &mode),
            ],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Date,
    Amount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

impl Dir {
    pub fn param(self) -> &'static str {
        match self {
            Dir::Asc => "asc",
            Dir::Desc => "desc",
        }
    }
}

/// A date range a link opened the list on (`?since=&until=`, the Reports page's fiscal year):
/// inclusive instants, with the zone its days are named in.
#[derive(Clone, Debug, PartialEq)]
pub struct DateRange {
    pub window: Window,
    pub zone: Option<String>,
}

impl DateRange {
    /// `?since=…&until=…[&zone=…]`: both ends as whole `YYYY-MM-DDTHH:MM:SSZ` instants, in order.
    pub fn parse(
        since: Option<String>,
        until: Option<String>,
        zone: Option<String>,
    ) -> Option<Self> {
        let instant = |v: &str| {
            let b = v.as_bytes();
            b.len() == 20
                && b[10] == b'T'
                && b[19] == b'Z'
                && Day::parse(v).is_some()
                && format::parse_ts(v).is_some()
        };
        let (since, until) = (since?, until?);
        (instant(&since) && instant(&until) && since <= until).then(|| DateRange {
            window: Window { since, until },
            zone: zone.filter(|z| crate::tax_calendar::calendar::zone(z).is_some()),
        })
    }

    /// The chip: the range's first and last day in its zone (UTC without one).
    pub fn label(&self) -> String {
        let zone = self.zone.as_deref().unwrap_or("UTC");
        let day = |v: &str| {
            crate::tax_calendar::local_day(zone, v).unwrap_or_else(|| v.chars().take(10).collect())
        };
        tr(
            "transactions:table.chip.dateRange",
            &[
                ("start", &day(&self.window.since)),
                ("end", &day(&self.window.until)),
            ],
        )
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Filters {
    pub chart: Option<ChartSel>,
    pub quick: Option<Quick>,
    pub breakdown: Option<Breakdown>,
    pub txids: Vec<String>,
    /// The date range a link asked for; it narrows the period's window.
    pub range: Option<DateRange>,
    pub status: Option<Status>,
    pub flow: Option<Flow>,
    pub payment: Option<String>,
    pub with_fees: bool,
    pub sort: Option<(SortKey, Dir)>,
}

impl Filters {
    /// `activeFilterCount` (the Filters button badge): txids don't count.
    pub fn active_count(&self) -> usize {
        [
            self.chart.is_some(),
            self.quick.is_some(),
            self.breakdown.is_some(),
            self.status.is_some(),
            self.flow.is_some(),
            self.payment.is_some(),
            self.with_fees,
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }
    pub fn any_chip(&self) -> bool {
        self.active_count() > 0 || !self.txids.is_empty() || self.range.is_some()
    }
    /// "Clear all": chart, quick, breakdown, txids and the table filters (sort included).
    pub fn clear_all(&mut self) {
        *self = Filters::default();
    }
    /// `resetTableFilters`: status/flow/payment/fees and sort.
    pub fn reset_table(&mut self) {
        self.status = None;
        self.flow = None;
        self.payment = None;
        self.with_fees = false;
        self.sort = None;
    }
    pub fn leg_wallet(&self) -> Option<&str> {
        self.breakdown
            .as_ref()
            .filter(|b| !b.network && b.leg)
            .map(|b| b.key.as_str())
    }
}

/// A chart bucket key's window, `[start, next start − 1 ms]`.
pub fn bucket_window(key: &str) -> Option<Window> {
    let y: i32 = key.get(0..4)?.parse().ok()?;
    let (start, next) = if key.len() == 4 {
        (
            Day { y, m: 1, d: 1 },
            Day {
                y: y + 1,
                m: 1,
                d: 1,
            },
        )
    } else if let Some(q) = key.strip_prefix(&format!("{y}-Q")) {
        let q: u32 = q.parse().ok()?;
        let m = (q - 1) * 3 + 1;
        (
            Day { y, m, d: 1 },
            if q == 4 {
                Day {
                    y: y + 1,
                    m: 1,
                    d: 1,
                }
            } else {
                Day { y, m: m + 3, d: 1 }
            },
        )
    } else if key.len() == 7 {
        let m: u32 = key.get(5..7)?.parse().ok()?;
        (
            Day { y, m, d: 1 },
            if m == 12 {
                Day {
                    y: y + 1,
                    m: 1,
                    d: 1,
                }
            } else {
                Day { y, m: m + 1, d: 1 }
            },
        )
    } else {
        let d = Day::parse(key)?;
        (d, Day::from_epoch_days(d.epoch_days() + 1))
    };
    Some(Window {
        since: start_iso(start),
        until: end_iso(Day::from_epoch_days(next.epoch_days() - 1)),
    })
}

fn intersect(a: &Window, b: &Window) -> Option<Window> {
    let since = if a.since > b.since {
        a.since.clone()
    } else {
        b.since.clone()
    };
    let until = if a.until < b.until {
        a.until.clone()
    } else {
        b.until.clone()
    };
    (since <= until).then_some(Window { since, until })
}

/// `buildTransactionListFilterArgs` plus the route's `limit` and leg-wallet scope.
pub fn list_args(
    f: &Filters,
    resolved: Period,
    years: &[BookYear],
    fiat: bool,
    cand: &Candidates,
) -> Value {
    let mut m = Map::new();
    m.insert("limit".into(), json!(100));
    if let Some(w) = f.leg_wallet() {
        m.insert("wallet".into(), json!(w));
    }
    // A linked date range narrows the period's window (or stands alone under `all`).
    let period_window = match (period_window(resolved, years), &f.range) {
        (Some(p), Some(r)) => Some(intersect(&p, &r.window).unwrap_or_else(|| r.window.clone())),
        (None, Some(r)) => Some(r.window.clone()),
        (p, None) => p,
    };
    if f.txids.is_empty()
        && let Some(w) = &period_window
    {
        m.insert("since".into(), json!(w.since));
        m.insert("until".into(), json!(w.until));
    }
    if !f.txids.is_empty() {
        m.insert("txids".into(), json!(f.txids));
    }
    if let Some(c) = &f.chart {
        if let Some(bw) = c.bucket_key.as_deref().and_then(bucket_window) {
            let w = match &period_window {
                Some(p) => intersect(&bw, p).unwrap_or(bw),
                None => bw,
            };
            m.insert("since".into(), json!(w.since));
            m.insert("until".into(), json!(w.until));
        }
        if let Some(seg) = c.segment {
            m.insert("flow".into(), json!(seg.flow().param()));
        } else if c.external {
            m.insert("quick".into(), json!("external_flow"));
        }
    }
    if let Some(q) = f.quick {
        m.insert("quick".into(), json!(q.param()));
    }
    if let Some(b) = &f.breakdown {
        if b.network {
            m.insert("payment_method".into(), json!(b.key));
        } else if !b.leg && split_legs(&b.key).len() == 1 {
            m.insert("wallet".into(), json!(b.key));
        }
    }
    if let Some(s) = f.status {
        m.insert("status".into(), json!(s.param()));
    }
    if let Some(fl) = f.flow {
        m.insert("flow".into(), json!(fl.param()));
    }
    if let Some(p) = &f.payment {
        m.insert("payment_method".into(), json!(p));
    }
    if f.with_fees {
        m.insert("withFees".into(), json!(true));
    }
    match f.sort {
        Some((SortKey::Date, d)) => {
            m.insert("sort".into(), json!("occurred-at"));
            m.insert("order".into(), json!(d.param()));
        }
        Some((SortKey::Amount, d)) => {
            m.insert(
                "sort".into(),
                json!(if fiat { "fiat-value" } else { "amount" }),
            );
            m.insert("order".into(), json!(d.param()));
        }
        None => {}
    }
    let final_flow = m.get("flow").and_then(Value::as_str).map(str::to_owned);
    match final_flow.as_deref() {
        Some("swap") if !cand.swap.is_empty() => {
            m.insert("candidate_txids".into(), json!(cand.swap));
        }
        Some("transfer" | "layer-transition") if !cand.transfer.is_empty() => {
            m.insert("candidate_txids".into(), json!(cand.transfer));
        }
        _ => {}
    }
    Value::Object(m)
}

/// Client-side filter over the loaded rows (all conditions AND-ed).
pub fn matches(r: &Row, f: &Filters, cand: &Candidates) -> bool {
    let flow = r.display_flow(cand);
    if !f.txids.is_empty() {
        let hit = f.txids.iter().any(|x| {
            let x = x.to_lowercase();
            r.id.to_lowercase() == x
                || r.txn_id.to_lowercase() == x
                || r.explorer_id
                    .as_deref()
                    .is_some_and(|e| e.to_lowercase() == x)
        });
        if !hit {
            return false;
        }
    }
    if f.status.is_some_and(|s| s != r.status) {
        return false;
    }
    if f.flow.is_some_and(|fl| fl != flow) {
        return false;
    }
    if f.payment.as_deref().is_some_and(|p| p != r.payment_method) {
        return false;
    }
    if f.with_fees && !(r.fee_btc > 0.0 || r.fee_fiat.is_some_and(|x| x > 0.0)) {
        return false;
    }
    if let Some(c) = &f.chart {
        if let Some(seg) = c.segment {
            // `flowChartSelectionMatchesFlow`: layer transitions count as transfers.
            let seg_flow = match seg {
                Segment::Transfers => matches!(flow, Flow::Transfer | Flow::Layer),
                other => other.flow() == flow,
            };
            if !seg_flow {
                return false;
            }
        } else if c.external && !matches!(flow, Flow::Incoming | Flow::Outgoing) {
            return false;
        }
        if let (Some(w), Some(ts)) = (c.bucket_key.as_deref().and_then(bucket_window), r.time()) {
            let (a, b) = (
                format::parse_ts(&w.since).unwrap_or(f64::MIN),
                format::parse_ts(&w.until).unwrap_or(f64::MAX) + 1.0,
            );
            if ts < a || ts >= b {
                return false;
            }
        }
    }
    match f.quick {
        Some(Quick::ExternalFlow) if !matches!(flow, Flow::Incoming | Flow::Outgoing) => {
            return false;
        }
        Some(Quick::ReviewQueue) if r.status == Status::Completed => return false,
        Some(Quick::NoExplorerId) if r.explorer_id.is_some() => return false,
        Some(Quick::MissingPrice) if r.rate.is_some() => return false,
        Some(Quick::FailedImport) if r.status != Status::Failed => return false,
        _ => {}
    }
    if let Some(b) = &f.breakdown {
        let ok = if b.network {
            r.payment_method == b.key
        } else if b.leg {
            r.wallet_legs()
                .iter()
                .any(|l| l.eq_ignore_ascii_case(&b.key))
        } else {
            r.wallet == b.key
        };
        if !ok {
            return false;
        }
    }
    true
}

/// The client sort (`sort=None` keeps the daemon's order). Ties keep the original order;
/// unparseable dates and unpriced amounts go last.
pub fn sort_indices(
    rows: &[Row],
    idx: &mut [usize],
    sort: Option<(SortKey, Dir)>,
    fiat: bool,
    cand: &Candidates,
) {
    let Some((key, dir)) = sort else { return };
    idx.sort_by(|&a, &b| {
        let (ra, rb) = (&rows[a], &rows[b]);
        let (va, vb) = match key {
            SortKey::Date => (ra.time(), rb.time()),
            SortKey::Amount if fiat => (
                ra.signed_fiat(ra.display_flow(cand)),
                rb.signed_fiat(rb.display_flow(cand)),
            ),
            SortKey::Amount => (
                Some(ra.signed_btc(ra.display_flow(cand))),
                Some(rb.signed_btc(rb.display_flow(cand))),
            ),
        };
        match (va, vb) {
            (Some(x), Some(y)) => {
                let o = x.total_cmp(&y);
                if dir == Dir::Asc { o } else { o.reverse() }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_and_buckets() {
        assert_eq!(
            shift_months(
                Day {
                    y: 2024,
                    m: 3,
                    d: 31
                },
                1
            ),
            Day {
                y: 2024,
                m: 2,
                d: 29
            }
        );
        assert_eq!(
            shift_months(
                Day {
                    y: 2026,
                    m: 11,
                    d: 15
                },
                48
            ),
            Day {
                y: 2022,
                m: 11,
                d: 15
            }
        );
        let w = bucket_window("2024-Q4").unwrap();
        assert_eq!(
            (w.since.as_str(), w.until.as_str()),
            ("2024-10-01T00:00:00.000Z", "2024-12-31T23:59:59.999Z")
        );
        assert_eq!(
            bucket_window("2024-02").unwrap().until,
            "2024-02-29T23:59:59.999Z"
        );
        assert_eq!(Period::parse("5y"), Some(Period::Range(Range::Y4)));
        assert_eq!(split_legs("A -> B"), vec!["A", "B"]);
        assert_eq!(
            short_txid("373b15e0a627488e80c3ad301d0fe61efdcfd3a2d8cf387b5577bb6faf3ea6cb"),
            "373b15e0a6...3ea6cb"
        );
    }

    #[test]
    fn paired_transfer_rows_show_what_moved() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let sweep = |pair: Value, rate: Value| {
            let mut p = json!({"id": "c7a09aab", "type": "transfer", "kind": "bark_txid_exact", "policy": "carrying-value", "outWallet": "BTCPay Bark", "outAsset": "BTC", "outAmountSat": 600_000, "inWallet": "Store on-chain", "inAsset": "BTC", "inAmountSat": 600_000, "feeSat": 0, "feeKind": null});
            for (k, v) in pair.as_object().unwrap() {
                p[k] = v.clone();
            }
            Row::parse(
                &json!({"id": "170cb33f", "date": "2026-11-13 11:52", "type": "Transfer", "account": "BTCPay Bark -> Store on-chain", "counter": "Transfer fee - BTC -> BTC", "amountSat": 0, "feeSat": p["feeSat"], "eur": -0.0, "rate": rate, "tag": "Transfer", "internal": true, "pair": p}),
            )
        };
        let r = sweep(json!({}), json!(92_000));
        assert_eq!(r.title(), "Transfer · BTCPay Bark → Store on-chain");
        let p = r.paired.clone().unwrap();
        assert_eq!(
            (p.moved_btc, p.moved_fiat, p.fee_btc, p.fee_fiat),
            (0.006, Some(552.0), 0.0, Some(0.0))
        );
        let p = sweep(
            json!({"outAmountSat": 600_420, "inAmountSat": 600_000, "feeSat": 420}),
            json!(92_000),
        )
        .paired
        .unwrap();
        assert_eq!(p.moved_btc, 0.006);
        assert_eq!(p.fee_btc, 0.0000042);
        assert!((p.fee_fiat.unwrap() - 0.3864).abs() < 1e-10);
        let p = sweep(json!({}), Value::Null).paired.unwrap();
        assert_eq!((p.moved_btc, p.moved_fiat), (0.006, None));
        assert!(
            sweep(json!({"type": "swap", "outAsset": "LBTC"}), json!(92_000))
                .paired
                .is_none()
        );
        crate::i18n::set_lang(crate::i18n::Lang::De);
        assert_eq!(
            sweep(json!({}), json!(92_000)).title(),
            "Übertragung · BTCPay Bark → Store on-chain"
        );
        crate::i18n::set_lang(crate::i18n::Lang::En);
    }

    #[test]
    fn chart_segments_group_flows_like_the_web() {
        let mut r = Row::parse(
            &json!({"id": "t1", "date": "2026-11-13 11:52", "type": "Transfer", "amountSat": 1000}),
        );
        let sel = |seg| Filters {
            chart: Some(ChartSel {
                bucket_key: None,
                bucket_label: String::new(),
                segment: Some(seg),
                external: false,
            }),
            ..Filters::default()
        };
        let c = Candidates::default();
        for (flow, transfers, swaps) in [
            (Flow::Layer, true, false),
            (Flow::Transfer, true, false),
            (Flow::Swap, false, true),
        ] {
            r.flow = flow;
            assert_eq!(
                matches(&r, &sel(Segment::Transfers), &c),
                transfers,
                "{flow:?} under Transfers"
            );
            assert_eq!(
                matches(&r, &sel(Segment::Swaps), &c),
                swaps,
                "{flow:?} under Swaps"
            );
        }
    }

    #[test]
    fn args_follow_the_web_builder() {
        let f = Filters {
            status: Some(Status::Review),
            sort: Some((SortKey::Amount, Dir::Desc)),
            breakdown: Some(Breakdown {
                network: false,
                key: "Cold".into(),
                leg: true,
            }),
            ..Filters::default()
        };
        let a = list_args(
            &f,
            Period::Range(Range::All),
            &[],
            true,
            &Candidates::default(),
        );
        assert_eq!(a["limit"], 100);
        assert_eq!(a["wallet"], "Cold");
        assert_eq!(a["status"], "review");
        assert_eq!(a["sort"], "fiat-value");
        assert!(a.get("since").is_none());
    }
}
