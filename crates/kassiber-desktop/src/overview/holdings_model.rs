//! The Holdings by Source settings and rows, the twelve-month trend and the Overview widgets'
//! figures (ui-tauri `store/ui.ts` `HoldingsView` + `components/overview-dashboard/model.ts`
//! `buildHoldingsBySource`, `monthEndTrend`, `monthlyFlows`, `displayBalanceBtc`).
//!
//! The fields the shared `model::Snapshot` does not carry (`lastTransactionAt`, `monthBalances`,
//! `taxFreeBalance`, `balanceSummary.totalBtc`'s presence) are read from the raw snapshot here
//! (`Extras`), so a daemon without them (before the `monthBalances` change) simply leaves the
//! trend out instead of failing.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::format;
use crate::model::{self, Connection, Rail, Snapshot, Tx};

// ---- The persisted view (`holdingsView`) --------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    List,
    Tiles,
    Table,
    Trend,
}

impl Layout {
    /// `HOLDINGS_LAYOUTS`, in the menu's order.
    pub const ALL: [Layout; 4] = [Layout::List, Layout::Tiles, Layout::Table, Layout::Trend];
    pub fn id(self) -> &'static str {
        match self {
            Layout::List => "list",
            Layout::Tiles => "tiles",
            Layout::Table => "table",
            Layout::Trend => "trend",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Balance,
    Name,
    Activity,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Balance, Sort::Name, Sort::Activity];
    pub fn id(self) -> &'static str {
        match self {
            Sort::Balance => "balance",
            Sort::Name => "name",
            Sort::Activity => "activity",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Empty {
    Fold,
    Show,
    Hide,
}

impl Empty {
    pub const ALL: [Empty; 3] = [Empty::Fold, Empty::Show, Empty::Hide];
    pub fn id(self) -> &'static str {
        match self {
            Empty::Fold => "fold",
            Empty::Show => "show",
            Empty::Hide => "hide",
        }
    }
}

/// `HoldingsRail`: the network group a source sits in; exchanges and files are `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Onchain,
    Liquid,
    Lightning,
    Other,
}

impl Group {
    /// `HOLDINGS_RAILS`, in the menu's order.
    pub const ALL: [Group; 4] = [
        Group::Onchain,
        Group::Liquid,
        Group::Lightning,
        Group::Other,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Group::Onchain => "onchain",
            Group::Liquid => "liquid",
            Group::Lightning => "lightning",
            Group::Other => "other",
        }
    }
}

/// `HOLDINGS_LIMITS`: sources listed before the rest fold into "Other sources"; 0 lists all.
pub const LIMITS: [u32; 3] = [8, 16, 0];

/// `HoldingsView`: the card's layout, order, filter and folding; the same for every book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldingsView {
    pub layout: Layout,
    pub sort: Sort,
    pub empty: Empty,
    /// Rails listed, in `Group::ALL` order; every rail when all are on.
    pub rails: Vec<Group>,
    pub limit: u32,
}

impl Default for HoldingsView {
    /// `DEFAULT_HOLDINGS_VIEW`.
    fn default() -> Self {
        HoldingsView {
            layout: Layout::List,
            sort: Sort::Balance,
            empty: Empty::Fold,
            rails: Group::ALL.to_vec(),
            limit: 8,
        }
    }
}

fn pick<T: Copy>(all: &[T], id: impl Fn(T) -> &'static str, v: &Value) -> Option<T> {
    let s = v.as_str()?;
    all.iter().copied().find(|x| id(*x) == s)
}

impl HoldingsView {
    /// `normalizeHoldingsView`: each field checked alone; an emptied rail filter lists every rail.
    pub fn from_json(v: &Value) -> HoldingsView {
        let d = HoldingsView::default();
        if !v.is_object() {
            return d;
        }
        let rails: Vec<Group> = match v["rails"].as_array() {
            Some(list) => Group::ALL
                .iter()
                .copied()
                .filter(|g| list.iter().any(|x| x.as_str() == Some(g.id())))
                .collect(),
            None => d.rails.clone(),
        };
        HoldingsView {
            layout: pick(&Layout::ALL, Layout::id, &v["layout"]).unwrap_or(d.layout),
            sort: pick(&Sort::ALL, Sort::id, &v["sort"]).unwrap_or(d.sort),
            empty: pick(&Empty::ALL, Empty::id, &v["empty"]).unwrap_or(d.empty),
            rails: if rails.is_empty() { d.rails } else { rails },
            limit: v["limit"]
                .as_u64()
                .filter(|l| LIMITS.contains(&(*l as u32)) && *l <= 16)
                .map_or(d.limit, |l| l as u32),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "layout": self.layout.id(),
            "sort": self.sort.id(),
            "empty": self.empty.id(),
            "rails": self.rails.iter().map(|g| g.id()).collect::<Vec<_>>(),
            "limit": self.limit,
        })
    }

    /// `setHoldingsView({rails})` for one rail's checkbox: kept in menu order, the last one stays.
    pub fn toggle_rail(&mut self, rail: Group) {
        if self.rails.contains(&rail) {
            if self.rails.len() > 1 {
                self.rails.retain(|g| *g != rail);
            }
        } else {
            self.rails = Group::ALL
                .iter()
                .copied()
                .filter(|g| *g == rail || self.rails.contains(g))
                .collect();
        }
    }

    pub fn all_rails(&self) -> bool {
        self.rails.len() == Group::ALL.len()
    }
}

// ---- What the shared snapshot model leaves out --------------------------------------------------

/// A wallet's month-end bitcoin for a month it moved (`monthBalances[]`).
#[derive(Clone, Debug, PartialEq)]
pub struct MonthBalance {
    pub month: String,
    pub balance_btc: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ConnExtra {
    pub last_transaction_at: Option<String>,
    /// `None` when the daemon does not send the field (before the month-balance change).
    pub month_balances: Option<Vec<MonthBalance>>,
}

/// `taxFreeBalance` (the Austrian Altbestand split), as the Tax year widget reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxFree {
    pub status: Option<String>,
    pub needs_journals: bool,
    pub quarantines: f64,
    pub tax_free_sats: f64,
    pub taxable_sats: f64,
}

impl TaxFree {
    /// The split's status, as on Wallets: an inventory waiting on journals or blocked by
    /// quarantines is not shown as a split.
    pub fn status(&self) -> &str {
        match self.status.as_deref() {
            Some(s) => s,
            None if self.needs_journals => "needs_journals",
            None if self.quarantines > 0.0 => "quarantines",
            None => "current",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Extras {
    pub conns: HashMap<String, ConnExtra>,
    pub tax_free: Option<TaxFree>,
    /// `balanceSummary.totalBtc` when the daemon sent a finite number.
    pub display_total_btc: Option<f64>,
}

fn finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

impl Extras {
    pub fn parse(raw: Option<&Value>) -> Extras {
        let Some(raw) = raw else {
            return Extras::default();
        };
        let mut conns = HashMap::new();
        for c in raw["connections"].as_array().into_iter().flatten() {
            let Some(id) = c["id"].as_str() else { continue };
            let month_balances = c["monthBalances"].as_array().map(|list| {
                list.iter()
                    .filter_map(|p| {
                        Some(MonthBalance {
                            month: p["month"].as_str()?.to_owned(),
                            balance_btc: finite(&p["balanceBtc"])?,
                        })
                    })
                    .collect()
            });
            conns.insert(
                id.to_owned(),
                ConnExtra {
                    last_transaction_at: c["lastTransactionAt"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    month_balances,
                },
            );
        }
        let t = &raw["taxFreeBalance"];
        let tax_free = t.is_object().then(|| TaxFree {
            status: t["status"].as_str().map(str::to_owned),
            needs_journals: t["needsJournals"].as_bool() == Some(true),
            quarantines: finite(&t["quarantines"]).unwrap_or(0.0).max(0.0),
            tax_free_sats: finite(&t["taxFreeQuantitySats"]).unwrap_or(0.0),
            taxable_sats: finite(&t["taxableQuantitySats"]).unwrap_or(0.0),
        });
        Extras {
            conns,
            tax_free,
            display_total_btc: finite(&raw["balanceSummary"]["totalBtc"]),
        }
    }

    /// Whether the daemon sends month-end balances at all (the trend layout needs them).
    pub fn has_month_balances(&self) -> bool {
        self.conns.values().any(|c| c.month_balances.is_some())
    }
}

/// `displayBalanceBtc`: the daemon's display total, else the end of the portfolio history.
pub fn display_balance_btc(s: &Snapshot, x: &Extras) -> f64 {
    x.display_total_btc
        .unwrap_or_else(|| s.latest_balance_btc())
}

// ---- Local calendar months (the webview's `Date`) ---------------------------------------------------

/// The local (year, month 1–12) of an epoch second, as `Date#getFullYear/getMonth` read it.
pub fn local_month(secs: f64) -> (i32, u32) {
    crate::localtime::year_month(secs.floor() as i64)
}

/// The UTC year of an epoch second (the daemon's tax year).
pub fn utc_year(secs: f64) -> i32 {
    format::Day::from_epoch_days((secs / 86_400.0).floor() as i64).y
}

/// The `months` calendar months ending with `now`'s, oldest first, as (year, month 1–12).
pub fn window(now: (i32, u32), months: u32) -> Vec<(i32, u32)> {
    (0..months)
        .rev()
        .map(|back| {
            let idx = now.0 * 12 + now.1 as i32 - 1 - back as i32;
            (idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1)
        })
        .collect()
}

/// Months the Holdings trend spans, ending with the current one (`HOLDINGS_TREND_MONTHS`).
pub const TREND_MONTHS: u32 = 12;

/// `monthEndTrend`: month-end bitcoin over the window; each balance carries forward until the
/// next, and before the first the wallet held nothing. Empty for a wallet that never moved.
pub fn month_end_trend(points: &[MonthBalance], now: (i32, u32), months: u32) -> Vec<f64> {
    if points.is_empty() {
        return vec![];
    }
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.month.cmp(&b.month));
    let mut index = 0;
    let mut balance = 0.0;
    window(now, months)
        .into_iter()
        .map(|(y, m)| {
            let key = format!("{y:04}-{m:02}");
            while index < sorted.len() && sorted[index].month <= key {
                balance = sorted[index].balance_btc;
                index += 1;
            }
            balance
        })
        .collect()
}

// ---- Rows ---------------------------------------------------------------------------------------

/// One row of the card (`HoldingsItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    /// The folded "Other sources" row (`nameKey`).
    pub other: bool,
    pub btc: f64,
    pub value: f64,
    pub percent: f64,
    /// The rail's logo; `Rail::None` for exchanges, files and the folded row.
    pub rail: Rail,
    /// The wallet the row opens; none for "Other sources".
    pub connection_id: Option<String>,
    pub last_sync_at: Option<String>,
    /// A source that held bitcoin once and holds none now.
    pub empty: bool,
    pub last_transaction_at: Option<String>,
    /// `"syncing"` or `"error"`: a figure that is not current.
    pub sync_state: Option<&'static str>,
    pub group: Group,
    pub transaction_count: f64,
    /// Month-end bitcoin over the last twelve months; `None` without the daemon's balances.
    pub trend: Option<Vec<f64>>,
}

/// `HoldingsBySource`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Built {
    /// Holding sources first, then any listed empty ones.
    pub items: Vec<Item>,
    /// Empty sources not listed in `items`, for the "N empty" line.
    pub folded: Vec<Item>,
    /// Sources the rail filter leaves out.
    pub filtered_out: usize,
    /// The bitcoin the listed sources hold.
    pub total_btc: f64,
}

/// `railForConnection`.
fn rail_for_connection(kind: &str, label: &str) -> Group {
    match kind.to_lowercase().as_str() {
        "xpub" | "address" | "descriptor" | "btcpay" => return Group::Onchain,
        "core-ln" | "lnd" | "nwc" | "phoenix" => return Group::Lightning,
        "cashu" | "kraken" | "bitstamp" | "coinbase" | "bitpanda" | "bitvavo" | "bitfinex"
        | "bitcoinde" | "river" | "bullbitcoin" | "coinfinity" | "strike" | "csv" | "bip329" => {
            return Group::Other;
        }
        _ => {}
    }
    let value = format!("{kind} {label}").to_lowercase();
    if value.contains("liquid") || value.contains("lbtc") {
        return Group::Liquid;
    }
    if ["lightning", "phoenix", "nwc", "core-ln", "lnd"]
        .iter()
        .any(|w| value.contains(w))
    {
        return Group::Lightning;
    }
    Group::Onchain
}

/// `holdingRail`: the logo a source gets (none for exchanges, files and ecash) and its group.
pub fn holding_rail(c: &Connection) -> (Rail, Group) {
    match rail_for_connection(&c.kind, &c.label) {
        Group::Other => (Rail::None, Group::Other),
        _ if c.chain == "liquid" => (Rail::Liquid, Group::Liquid),
        Group::Onchain => (Rail::Onchain, Group::Onchain),
        Group::Liquid => (Rail::Liquid, Group::Liquid),
        Group::Lightning => (Rail::Lightning, Group::Lightning),
    }
}

/// `percentOf`.
pub fn percent_of(value: f64, total: f64) -> f64 {
    if total <= 0.0 {
        0.0
    } else {
        (value / total * 100.0).round()
    }
}

fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    // `localeCompare` in the UI locale: case-insensitive first, then exact.
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

fn order(sort: Sort, a: &Item, b: &Item) -> std::cmp::Ordering {
    if sort == Sort::Name {
        return name_cmp(&a.name, &b.name);
    }
    if sort == Sort::Activity {
        let by_date = b
            .last_transaction_at
            .as_deref()
            .unwrap_or("")
            .cmp(a.last_transaction_at.as_deref().unwrap_or(""));
        if by_date.is_ne() {
            return by_date;
        }
    }
    b.btc
        .total_cmp(&a.btc)
        .then_with(|| name_cmp(&a.name, &b.name))
}

/// `buildHoldingsBySource`: holding sources in the chosen order, past `view.limit` the rest
/// folded into one "Other sources" row, then the emptied ones listed, folded or left out. Shares
/// are of all the bitcoin held, so a filtered row keeps the share it has of the whole book.
pub fn build(s: &Snapshot, x: &Extras, view: &HoldingsView, now: (i32, u32)) -> Built {
    let rate = s.rate();
    let book_total: f64 = s.connections.iter().map(|c| c.balance.max(0.0)).sum();
    let to_item = |c: &Connection| {
        let (rail, group) = holding_rail(c);
        let extra = x.conns.get(&c.id);
        let btc = c.balance.max(0.0);
        Item {
            name: c.label.clone(),
            other: false,
            btc,
            value: btc * rate,
            percent: percent_of(btc, book_total),
            rail,
            connection_id: Some(c.id.clone()),
            last_sync_at: c.last_sync_at.clone().filter(|s| !s.is_empty()),
            empty: c.balance <= 0.0,
            last_transaction_at: extra.and_then(|e| e.last_transaction_at.clone()),
            sync_state: match c.status.as_str() {
                "syncing" => Some("syncing"),
                "error" => Some("error"),
                _ => None,
            },
            group,
            transaction_count: c.transaction_count,
            trend: extra
                .and_then(|e| e.month_balances.as_ref())
                .map(|p| month_end_trend(p, now, TREND_MONTHS)),
        }
    };
    let wallets: Vec<&Connection> = s
        .connections
        .iter()
        .filter(|c| !c.deprecated || c.balance > 0.0)
        .collect();
    let listed: Vec<&Connection> = wallets
        .iter()
        .copied()
        .filter(|c| view.rails.contains(&holding_rail(c).1))
        .collect();
    let mut sorted: Vec<Item> = listed.iter().map(|c| to_item(c)).collect();
    sorted.sort_by(|a, b| order(view.sort, a, b));
    let holding: Vec<Item> = sorted.iter().filter(|i| !i.empty).cloned().collect();
    // A source never funded is configuration, not an emptied holding.
    let emptied: Vec<Item> = sorted
        .into_iter()
        .filter(|i| i.empty && i.last_transaction_at.is_some())
        .collect();
    let limit = view.limit as usize;
    let total_btc = holding.iter().map(|i| i.btc).sum();
    let shown = if limit > 0 && holding.len() > limit {
        let rest: f64 = holding[limit - 1..].iter().map(|i| i.btc).sum();
        let mut v = holding[..limit - 1].to_vec();
        v.push(Item {
            name: String::new(),
            other: true,
            btc: rest,
            value: rest * rate,
            percent: percent_of(rest, book_total),
            rail: Rail::None,
            connection_id: None,
            last_sync_at: None,
            empty: false,
            last_transaction_at: None,
            sync_state: None,
            group: Group::Other,
            transaction_count: 0.0,
            trend: None,
        });
        v
    } else {
        holding
    };
    let (items, folded) = match view.empty {
        Empty::Show => ([shown, emptied].concat(), vec![]),
        Empty::Fold => (shown, emptied),
        Empty::Hide => (shown, vec![]),
    };
    Built {
        items,
        folded,
        filtered_out: wallets.len() - listed.len(),
        total_btc,
    }
}

// ---- Money in and out ---------------------------------------------------------------------------

const FLOW_MONTHS: u32 = 12;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MonthFlow {
    pub year: i32,
    pub month: u32,
    pub in_btc: f64,
    pub out_btc: f64,
    pub in_value: f64,
    pub out_value: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MonthlyFlows {
    pub months: Vec<MonthFlow>,
    pub in_btc: f64,
    pub out_btc: f64,
    pub in_value: f64,
    pub out_value: f64,
    pub fee_btc: f64,
    /// Every flow counted carries a value in the book's currency.
    pub fiat_complete: bool,
    /// Anything received, spent or paid in fees inside the window.
    pub has_activity: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AFlow {
    Incoming,
    Outgoing,
    Movement,
    Fee,
}

/// `activityFlowForTx`.
fn activity_flow(tx: &Tx) -> AFlow {
    if tx.tx_type == "Fee" {
        return AFlow::Fee;
    }
    match model::flow(tx) {
        model::Flow::Incoming => AFlow::Incoming,
        model::Flow::Outgoing => AFlow::Outgoing,
        model::Flow::Transfer | model::Flow::Swap => AFlow::Movement,
    }
}

/// `parseOverviewTxDate` in epoch seconds (a space-separated date is UTC).
fn tx_secs(tx: &Tx) -> Option<f64> {
    let raw = tx
        .occurred_at
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&tx.date);
    if raw.is_empty() {
        None
    } else {
        format::parse_ts(raw)
    }
}

/// `monthlyFlows`: received and spent per calendar month over the last twelve, oldest first,
/// read as the chart's activity dots read them; moves between own wallets count only their fees.
pub fn monthly_flows(s: &Snapshot, now: (i32, u32)) -> MonthlyFlows {
    let mut months: Vec<MonthFlow> = window(now, FLOW_MONTHS)
        .into_iter()
        .map(|(year, month)| MonthFlow {
            year,
            month,
            ..MonthFlow::default()
        })
        .collect();
    let book = s.fiat_currency();
    let src = if s.activity_txs.is_empty() {
        &s.txs
    } else {
        &s.activity_txs
    };
    let mut fee_btc = 0.0;
    let mut fiat_complete = true;
    for tx in src {
        if tx.excluded {
            continue;
        }
        let Some(secs) = tx_secs(tx) else { continue };
        let btc = tx.amount_sat.abs() / 1e8;
        let fee = tx.fee_sat.abs() / 1e8;
        let flow = activity_flow(tx);
        let paired = tx.pair.as_ref().map_or(btc, |p| {
            btc.max(p.out_amount_sat.abs() / 1e8)
                .max(p.in_amount_sat.abs() / 1e8)
        });
        let volume = match flow {
            AFlow::Fee => btc.max(fee),
            AFlow::Movement => paired,
            _ => btc,
        };
        if volume <= 0.0 && fee <= 0.0 {
            continue;
        }
        let (y, m) = local_month(secs);
        let Some(month) = months.iter_mut().find(|x| x.year == y && x.month == m) else {
            continue;
        };
        // A fee row may carry its fee as the amount rather than as feeSat; its volume is the
        // larger of the two, so neither is counted twice.
        fee_btc += if flow == AFlow::Fee { volume } else { fee };
        if matches!(flow, AFlow::Incoming | AFlow::Outgoing) {
            let cur = tx
                .fiat_currency
                .as_deref()
                .map(str::to_uppercase)
                .filter(|c| !c.is_empty());
            if tx.eur.is_none() || cur.is_some_and(|c| c != book) {
                fiat_complete = false;
            }
        }
        let value = tx.eur.unwrap_or(0.0).abs();
        match flow {
            AFlow::Incoming => {
                month.in_btc += btc;
                month.in_value += value;
            }
            AFlow::Outgoing => {
                month.out_btc += btc;
                month.out_value += value;
            }
            _ => {}
        }
    }
    let sum = |f: fn(&MonthFlow) -> f64| months.iter().map(f).sum::<f64>();
    let (in_btc, out_btc) = (sum(|m| m.in_btc), sum(|m| m.out_btc));
    MonthlyFlows {
        in_value: sum(|m| m.in_value),
        out_value: sum(|m| m.out_value),
        in_btc,
        out_btc,
        fee_btc,
        fiat_complete,
        has_activity: in_btc > 0.0 || out_btc > 0.0 || fee_btc > 0.0,
        months,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str, kind: &str, chain: &str, balance: f64) -> Connection {
        Connection {
            id: id.into(),
            kind: kind.into(),
            label: id.into(),
            chain: chain.into(),
            balance,
            status: "synced".into(),
            ..Connection::default()
        }
    }

    /// (connection id, its role, its other assets as (asset, amount)).
    type ExtraEntry<'a> = (&'a str, Option<&'a str>, Option<Vec<(&'a str, f64)>>);

    fn extras(entries: &[ExtraEntry]) -> Extras {
        let mut x = Extras::default();
        for (id, last, months) in entries {
            x.conns.insert(
                (*id).into(),
                ConnExtra {
                    last_transaction_at: last.map(str::to_owned),
                    month_balances: months.as_ref().map(|v| {
                        v.iter()
                            .map(|(m, b)| MonthBalance {
                                month: (*m).into(),
                                balance_btc: *b,
                            })
                            .collect()
                    }),
                },
            );
        }
        x
    }

    /// The web test's book: two holding sources, an emptied one and one never funded.
    fn book() -> (Snapshot, Extras) {
        let s = Snapshot {
            connections: vec![
                conn("Cold", "xpub", "bitcoin", 2.0),
                conn("Alpha Liquid", "xpub", "liquid", 1.0),
                conn("Emptied", "xpub", "bitcoin", 0.0),
                conn("Never funded", "xpub", "bitcoin", 0.0),
            ],
            ..Snapshot::default()
        };
        let x = extras(&[
            ("Cold", Some("2025-03-01T00:00:00Z"), None),
            ("Alpha Liquid", Some("2026-06-01T00:00:00Z"), None),
            ("Emptied", Some("2026-01-01T00:00:00Z"), None),
            ("Never funded", None, None),
        ]);
        (s, x)
    }

    fn names(view: HoldingsView) -> (Vec<String>, Vec<String>) {
        let (s, x) = book();
        let b = build(&s, &x, &view, (2026, 11));
        (
            b.items.iter().map(|i| i.name.clone()).collect(),
            b.folded.iter().map(|i| i.name.clone()).collect(),
        )
    }

    #[test]
    fn folds_an_emptied_source_and_never_lists_one_without_history() {
        assert_eq!(
            names(HoldingsView::default()),
            (
                vec!["Cold".into(), "Alpha Liquid".into()],
                vec!["Emptied".into()]
            )
        );
        let show = HoldingsView {
            empty: Empty::Show,
            ..HoldingsView::default()
        };
        assert_eq!(names(show).0, vec!["Cold", "Alpha Liquid", "Emptied"]);
        let hide = HoldingsView {
            empty: Empty::Hide,
            ..HoldingsView::default()
        };
        assert_eq!(
            names(hide),
            (vec!["Cold".into(), "Alpha Liquid".into()], vec![])
        );
    }

    #[test]
    fn sorts_by_name_or_latest_activity() {
        assert_eq!(
            names(HoldingsView {
                sort: Sort::Name,
                ..HoldingsView::default()
            })
            .0,
            vec!["Alpha Liquid", "Cold"]
        );
        assert_eq!(
            names(HoldingsView {
                sort: Sort::Activity,
                ..HoldingsView::default()
            })
            .0,
            vec!["Alpha Liquid", "Cold"]
        );
    }

    #[test]
    fn filters_by_rail_and_keeps_each_share_of_the_book() {
        let (s, x) = book();
        let b = build(
            &s,
            &x,
            &HoldingsView {
                rails: vec![Group::Liquid],
                ..HoldingsView::default()
            },
            (2026, 11),
        );
        assert_eq!(
            b.items
                .iter()
                .map(|i| (i.name.as_str(), i.percent))
                .collect::<Vec<_>>(),
            vec![("Alpha Liquid", 33.0)]
        );
        assert_eq!(b.total_btc, 1.0);
        assert_eq!(b.filtered_out, 3);
    }

    #[test]
    fn folds_past_the_limit_into_other_sources() {
        let s = Snapshot {
            connections: (0..10)
                .map(|i| conn(&format!("w{i}"), "xpub", "bitcoin", 10.0 - i as f64))
                .collect(),
            ..Snapshot::default()
        };
        let b = build(&s, &Extras::default(), &HoldingsView::default(), (2026, 11));
        assert_eq!(b.items.len(), 8);
        let other = b.items.last().unwrap();
        assert!(other.other && other.connection_id.is_none());
        assert_eq!(other.btc, 3.0 + 2.0 + 1.0);
        let all = build(
            &s,
            &Extras::default(),
            &HoldingsView {
                limit: 0,
                ..HoldingsView::default()
            },
            (2026, 11),
        );
        assert_eq!(all.items.len(), 10);
    }

    #[test]
    fn deprecated_sources_list_only_while_they_hold_bitcoin() {
        let mut gone = conn("Old", "xpub", "bitcoin", 0.0);
        gone.deprecated = true;
        let mut kept = conn("Kept", "xpub", "bitcoin", 0.5);
        kept.deprecated = true;
        let s = Snapshot {
            connections: vec![gone, kept],
            ..Snapshot::default()
        };
        let x = extras(&[("Old", Some("2024-01-01"), None)]);
        let b = build(&s, &x, &HoldingsView::default(), (2026, 11));
        assert_eq!(
            b.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            vec!["Kept"]
        );
        assert!(b.folded.is_empty());
    }

    #[test]
    fn normalizes_a_stored_view() {
        assert_eq!(
            HoldingsView::from_json(&json!({"rails": [], "sort": "bogus", "limit": 3})),
            HoldingsView::default()
        );
        // Layouts tried and dropped fall back to the list.
        assert_eq!(
            HoldingsView::from_json(&json!({"layout": "grouped"})).layout,
            Layout::List
        );
        assert_eq!(HoldingsView::from_json(&json!(5)), HoldingsView::default());
        let v = HoldingsView {
            layout: Layout::Trend,
            sort: Sort::Activity,
            empty: Empty::Hide,
            rails: vec![Group::Liquid, Group::Other],
            limit: 0,
        };
        assert_eq!(HoldingsView::from_json(&v.to_json()), v);
        // Stored rails come back in menu order.
        assert_eq!(
            HoldingsView::from_json(&json!({"rails": ["other", "onchain"]})).rails,
            vec![Group::Onchain, Group::Other]
        );
    }

    #[test]
    fn the_last_rail_stays_on() {
        let mut v = HoldingsView {
            rails: vec![Group::Lightning],
            ..HoldingsView::default()
        };
        v.toggle_rail(Group::Lightning);
        assert_eq!(v.rails, vec![Group::Lightning]);
        v.toggle_rail(Group::Onchain);
        assert_eq!(v.rails, vec![Group::Onchain, Group::Lightning]);
    }

    #[test]
    fn carries_month_end_balances_forward_across_the_year() {
        let p = |m: &str, b: f64| MonthBalance {
            month: m.into(),
            balance_btc: b,
        };
        let trend = month_end_trend(
            &[p("2024-05", 3.0), p("2025-06", 2.5), p("2026-01", 1.0)],
            (2026, 2),
            12,
        );
        assert_eq!(
            trend,
            vec![3.0, 3.0, 3.0, 2.5, 2.5, 2.5, 2.5, 2.5, 2.5, 2.5, 1.0, 1.0]
        );
        assert_eq!(
            month_end_trend(&[p("2026-02", 0.25)], (2026, 2), 3),
            vec![0.0, 0.0, 0.25]
        );
        assert!(month_end_trend(&[], (2026, 2), 12).is_empty());
    }

    #[test]
    fn rails_follow_the_web_rules() {
        assert_eq!(
            holding_rail(&conn("a", "descriptor", "liquid", 1.0)),
            (Rail::Liquid, Group::Liquid)
        );
        assert_eq!(
            holding_rail(&conn("a", "kraken", "", 1.0)),
            (Rail::None, Group::Other)
        );
        assert_eq!(
            holding_rail(&conn("a", "silent-payment", "bitcoin", 1.0)),
            (Rail::Onchain, Group::Onchain)
        );
        assert_eq!(
            holding_rail(&conn("a", "lnd", "", 1.0)),
            (Rail::Lightning, Group::Lightning)
        );
    }

    #[test]
    fn extras_tolerate_a_daemon_without_month_balances() {
        let raw = json!({"connections": [{"id": "a", "lastTransactionAt": "2025-01-01T00:00:00Z"}], "balanceSummary": {"totalBtc": 0.0075}});
        let x = Extras::parse(Some(&raw));
        assert!(!x.has_month_balances());
        assert_eq!(x.display_total_btc, Some(0.0075));
        assert!(x.tax_free.is_none());
        let raw = json!({"connections": [{"id": "a", "monthBalances": [{"month": "2026-01", "balanceBtc": 1.5}, {"month": 5}]}], "balanceSummary": {}});
        let x = Extras::parse(Some(&raw));
        assert!(x.has_month_balances());
        assert_eq!(x.conns["a"].month_balances.as_ref().unwrap().len(), 1);
        assert_eq!(x.display_total_btc, None);
    }

    #[test]
    fn display_balance_prefers_the_daemon_total() {
        let s = Snapshot {
            portfolio_series: vec![model::PortfolioPoint {
                date: "2026-03-01".into(),
                balance_btc: 0.00761,
                ..Default::default()
            }],
            ..Snapshot::default()
        };
        assert_eq!(display_balance_btc(&s, &Extras::default()), 0.00761);
        let x = Extras {
            display_total_btc: Some(0.0075),
            ..Extras::default()
        };
        assert_eq!(display_balance_btc(&s, &x), 0.0075);
    }

    fn tx(id: &str, date: &str, amount_sat: f64, eur: Option<f64>) -> Tx {
        Tx {
            id: id.into(),
            date: date.into(),
            amount_sat,
            eur,
            tx_type: if amount_sat >= 0.0 {
                "Income".into()
            } else {
                "Expense".into()
            },
            conf: 1.0,
            ..Tx::default()
        }
    }

    fn flows(txs: Vec<Tx>) -> MonthlyFlows {
        let s = Snapshot {
            activity_txs: txs,
            fiat: model::Fiat {
                fiat_currency: Some("EUR".into()),
                ..Default::default()
            },
            ..Snapshot::default()
        };
        monthly_flows(&s, (2026, 11))
    }

    #[test]
    fn sums_received_spent_and_fees_by_month() {
        let mut out = tx("out", "2026-11-02 10:00", -10_000_000.0, Some(-6_000.0));
        out.fee_sat = 2_000.0;
        let mut mv = tx("move", "2026-06-01 10:00", -20_000_000.0, Some(-12_000.0));
        mv.tx_type = "Transfer".into();
        mv.internal = true;
        mv.fee_sat = 1_000.0;
        let f = flows(vec![
            tx("in", "2026-03-04 10:00", 50_000_000.0, Some(30_000.0)),
            out,
            mv,
            tx("old", "2025-10-30 10:00", 90_000_000.0, Some(50_000.0)),
        ]);
        assert_eq!(f.months.len(), 12);
        assert_eq!((f.months[0].year, f.months[0].month), (2025, 12));
        assert!((f.in_btc - 0.5).abs() < 1e-12);
        assert!((f.out_btc - 0.1).abs() < 1e-12);
        assert_eq!(f.in_value, 30_000.0);
        // A move between own wallets adds only its fee.
        assert!((f.fee_btc - 0.00003).abs() < 1e-12);
        assert!(f.has_activity);
    }

    #[test]
    fn counts_a_fee_row_whose_fee_is_its_amount_once() {
        let mut a = tx("a", "2026-05-02 10:00", -3_000.0, Some(-2.0));
        a.tx_type = "Fee".into();
        let mut b = tx("b", "2026-05-03 10:00", -2_000.0, Some(-1.0));
        b.tx_type = "Fee".into();
        b.fee_sat = 2_000.0;
        let f = flows(vec![a, b]);
        assert!((f.fee_btc - 0.00005).abs() < 1e-12);
        assert_eq!(f.in_btc + f.out_btc, 0.0);
        assert!(f.has_activity);
    }

    #[test]
    fn fiat_totals_need_every_flow_in_the_book_currency() {
        assert!(
            flows(vec![tx(
                "in",
                "2026-03-04 10:00",
                50_000_000.0,
                Some(30_000.0)
            )])
            .fiat_complete
        );
        let mut usd = tx("usd", "2026-04-04 10:00", 10_000_000.0, Some(9_000.0));
        usd.fiat_currency = Some("USD".into());
        assert!(
            !flows(vec![
                tx("in", "2026-03-04 10:00", 50_000_000.0, Some(30_000.0)),
                usd
            ])
            .fiat_complete
        );
        assert!(!flows(vec![tx("in", "2026-03-04 10:00", 50_000_000.0, None)]).fiat_complete);
    }

    #[test]
    fn a_quiet_year_has_no_activity() {
        assert!(
            !flows(vec![tx(
                "old",
                "2024-01-10 10:00",
                90_000_000.0,
                Some(50_000.0)
            )])
            .has_activity
        );
    }

    #[test]
    fn tax_free_status_falls_back_to_its_flags() {
        let t = |status: Option<&str>, nj: bool, q: f64| TaxFree {
            status: status.map(str::to_owned),
            needs_journals: nj,
            quarantines: q,
            tax_free_sats: 0.0,
            taxable_sats: 0.0,
        };
        assert_eq!(t(None, true, 0.0).status(), "needs_journals");
        assert_eq!(t(None, false, 2.0).status(), "quarantines");
        assert_eq!(t(None, false, 0.0).status(), "current");
        assert_eq!(t(Some("quarantines"), false, 0.0).status(), "quarantines");
    }

    #[test]
    fn windows_cross_the_year() {
        assert_eq!(window((2026, 2), 3), vec![(2025, 12), (2026, 1), (2026, 2)]);
    }
}
