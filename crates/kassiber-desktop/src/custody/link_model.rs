//! Linking two transactions by hand (`routes/transfers/linkModel.ts`): what a linked
//! transaction has left open, a pair's terms for linking it again, the fee check and the date
//! helpers the page shares with "Link manually". Exact msat as `i128`.

use std::collections::HashMap;

use super::model::Pair;

/// What the pair list says about a transaction that is linked already.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenPart {
    pub linked_msat: i128,
    pub unlinked_msat: i128,
    pub can_add: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegSide {
    Send,
    Receipt,
}

/// Per side, what each linked transaction has left (`openParts`).
#[derive(Clone, Debug, Default)]
pub struct Parts {
    pub send: HashMap<String, OpenPart>,
    pub receipt: HashMap<String, OpenPart>,
}

impl Parts {
    pub fn get(&self, side: LegSide, id: &str) -> Option<&OpenPart> {
        match side {
            LegSide::Send => self.send.get(id),
            LegSide::Receipt => self.receipt.get(id),
        }
    }
}

/// `openParts`.
pub fn open_parts(pairs: &[Pair]) -> Parts {
    let mut parts = Parts::default();
    let mut shared: [HashMap<String, bool>; 2] = Default::default();
    for p in pairs {
        for (si, leg) in [(0usize, &p.out), (1, &p.inn)] {
            let map = if si == 0 {
                &mut parts.send
            } else {
                &mut parts.receipt
            };
            let unlinked = leg.unlinked_msat;
            let prev = shared[si].get(&leg.transaction_id).copied().unwrap_or(true);
            shared[si].insert(
                leg.transaction_id.clone(),
                prev && p.shares_legs == Some(true) && unlinked.is_some(),
            );
            let before = map
                .get(&leg.transaction_id)
                .map(|o| o.linked_msat)
                .unwrap_or(0);
            map.insert(
                leg.transaction_id.clone(),
                OpenPart {
                    linked_msat: before + leg.amount_msat.abs(),
                    unlinked_msat: unlinked.unwrap_or(0),
                    can_add: false,
                },
            );
        }
    }
    for (si, map) in [(0usize, &mut parts.send), (1, &mut parts.receipt)] {
        for (id, part) in map.iter_mut() {
            let size = check_fee(part.linked_msat + part.unlinked_msat, part.unlinked_msat);
            part.can_add = shared[si].get(id).copied().unwrap_or(false)
                && matches!(size, FeeCheck::High | FeeCheck::Mismatch);
        }
    }
    parts
}

/// `hasOpenPart` of a linked leg.
pub fn has_open_part(linked: bool, open: Option<&OpenPart>) -> bool {
    linked && open.is_some_and(|o| o.can_add && o.unlinked_msat > 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeeCheck {
    None,
    Plausible,
    Negative,
    High,
    Mismatch,
}

const HIGH_FEE_SHARE_BPS: i128 = 100;
const HIGH_FEE_FLOOR_MSAT: i128 = 2_500_000;
const MISMATCH_SHARE_BPS: i128 = 1_000;

/// `checkFee`.
pub fn check_fee(send_msat: i128, fee_msat: i128) -> FeeCheck {
    if fee_msat < 0 {
        return FeeCheck::Negative;
    }
    if fee_msat == 0 {
        return FeeCheck::None;
    }
    if fee_msat > HIGH_FEE_FLOOR_MSAT && fee_msat * 10_000 > send_msat * MISMATCH_SHARE_BPS {
        return FeeCheck::Mismatch;
    }
    if fee_msat > HIGH_FEE_FLOOR_MSAT && fee_msat * 10_000 > send_msat * HIGH_FEE_SHARE_BPS {
        return FeeCheck::High;
    }
    FeeCheck::Plausible
}

/// The terms a pair had, as its record lists them, to link it again with (`RelinkTerms`).
/// Kept as the exact JSON values the record had (a number already rounded is refused later).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelinkTerms {
    pub out_amount_msat: Option<serde_json::Value>,
    pub swap_fee_msat: Option<serde_json::Value>,
    pub send_amount_msat: Option<serde_json::Value>,
    pub receipt_amount_msat: Option<serde_json::Value>,
}

fn non_null(v: &serde_json::Value) -> Option<serde_json::Value> {
    (!v.is_null()).then(|| v.clone())
}

/// `relinkTerms` from a pair's record in the pair list.
pub fn relink_terms(raw: &serde_json::Value) -> RelinkTerms {
    RelinkTerms {
        out_amount_msat: non_null(&raw["out_amount"]),
        swap_fee_msat: non_null(&raw["swap_fee_msat"]),
        send_amount_msat: non_null(&raw["out"]["full_amount_msat"]),
        receipt_amount_msat: non_null(&raw["in"]["full_amount_msat"]),
    }
}

/// The date of an ISO time, or a dash.
pub fn date_part(v: Option<&str>) -> String {
    match v {
        Some(s) if !s.is_empty() => s.chars().take(10).collect(),
        _ => "—".into(),
    }
}

/// `HH:MM` after the `T` (or space) of an ISO time.
pub fn time_part(v: Option<&str>) -> Option<String> {
    let s = v?;
    let i = s.find(['T', ' '])?;
    let t = s.get(i + 1..i + 6)?;
    let ok = t.len() == 5
        && t.as_bytes()[2] == b':'
        && t.chars()
            .enumerate()
            .all(|(j, c)| j == 2 || c.is_ascii_digit());
    ok.then(|| t.to_owned())
}

/// "2024-09-17 02:14", or the date alone.
pub fn date_time(v: Option<&str>) -> String {
    match time_part(v) {
        Some(t) => format!("{} {t}", date_part(v)),
        None => date_part(v),
    }
}

/// A txid shortened the way the Transactions table shows one.
pub fn short_id(v: Option<&str>, head: usize, tail: usize) -> String {
    let Some(s) = v.filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let n = s.chars().count();
    if n > head + tail + 1 {
        let h: String = s.chars().take(head).collect();
        let t: String = s.chars().skip(n - tail).collect();
        format!("{h}…{t}")
    } else {
        s.to_owned()
    }
}

/// `kindKey`: the key under `review:transfersPage.kind.*`.
pub fn kind_key(kind: Option<&str>) -> &'static str {
    match kind.unwrap_or("") {
        "peg-in" => "pegIn",
        "peg-out" => "pegOut",
        "submarine-swap" => "submarine",
        "reverse-submarine-swap" => "reverseSubmarine",
        "chain-swap" => "chainSwap",
        "swap-refund" => "refund",
        "coinjoin" | "whirlpool" => "coinjoin",
        _ => "manual",
    }
}

// ---- Legs, candidates and the check (the rest of linkModel.ts) --------------------------------

use super::model::{Rail, is_bitcoin, rail_for_leg};
use serde_json::Value;

/// How a leg is linked today, when it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegLink {
    pub wallet: Option<String>,
    pub automatic: bool,
}

/// One side of a link as the picker and the check show it (`LinkLeg`).
#[derive(Clone, Debug, PartialEq)]
pub struct LinkLeg {
    pub id: String,
    pub external_id: Option<String>,
    pub wallet: String,
    pub asset: String,
    pub rail: Rail,
    pub occurred_at: Option<String>,
    /// Always positive.
    pub amount_msat: i128,
    pub fee_msat: Option<i128>,
    pub side: LegSide,
    pub linked: bool,
    pub link: Option<LegLink>,
    pub open: Option<OpenPart>,
    pub held: bool,
    pub excluded: bool,
    pub booked_as: Option<String>,
    /// Its value in the display currency: Some(None) is unpriced, None unknown here.
    pub fiat: Option<Option<f64>>,
    pub network: Option<String>,
}

/// Sats as the daemon writes them (a JSON number), as exact msat (`msatFromSats`).
pub fn msat_from_sats(v: &Value) -> Option<i128> {
    let text = match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                let f = n.as_f64()?;
                if !f.is_finite() {
                    return None;
                }
                let t = n.to_string();
                if t.contains(['e', 'E']) {
                    format!("{f:.3}")
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_owned()
                } else {
                    t
                }
            }
        }
        Value::String(s) => s.trim().to_owned(),
        _ => return None,
    };
    let (neg, body) = match text.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, text.as_str()),
    };
    let (whole, frac) = body.split_once('.').unwrap_or((body, ""));
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || frac.len() > 3
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let msat =
        whole.parse::<i128>().ok()? * 1000 + format!("{frac:0<3}").parse::<i128>().unwrap_or(0);
    Some(if neg { -msat } else { msat })
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn rail_for_tx(tx: &Value) -> Rail {
    match tx["paymentMethod"].as_str() {
        Some("Lightning") => Rail::Lightning,
        Some("Liquid") => Rail::Liquid,
        _ => rail_for_leg(
            Some(tx["asset"].as_str().unwrap_or("BTC")),
            tx["chain"].as_str(),
        ),
    }
}

/// A transaction as a leg (`legFromTx`); a list row read for one `side` takes that side's
/// wallet, asset and amount from its link.
pub fn leg_from_tx(tx: &Value, side: Option<LegSide>) -> Option<LinkLeg> {
    let own = msat_from_sats(&tx["amountSat"]);
    let r#as = side.unwrap_or(if own.is_some_and(|o| o < 0) {
        LegSide::Send
    } else {
        LegSide::Receipt
    });
    let pair = if side.is_some() {
        tx.get("pair").filter(|p| p.is_object())
    } else {
        None
    };
    let send = r#as == LegSide::Send;
    let amount = pair
        .and_then(|p| msat_from_sats(&p[if send { "outAmountSat" } else { "inAmountSat" }]))
        .or(own)?;
    let fee = if pair.is_some() {
        None
    } else {
        msat_from_sats(&tx["feeSat"])
    };
    let tx_pair = tx.get("pair").filter(|p| p.is_object());
    let internal = tx["internal"].as_bool().unwrap_or(false);
    let pick = |a: &str, b: &str| {
        pair.and_then(|p| str_of(p, if send { a } else { b }))
            .filter(|s| !s.is_empty())
    };
    let asset = pick("outAsset", "inAsset")
        .or_else(|| str_of(tx, "asset"))
        .unwrap_or_else(|| "BTC".into())
        .to_uppercase();
    let asset = if asset == "L-BTC" {
        "LBTC".to_owned()
    } else {
        asset
    };
    Some(LinkLeg {
        id: str_of(tx, "id").unwrap_or_default(),
        external_id: str_of(tx, "externalId")
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        wallet: pick("outWallet", "inWallet")
            .unwrap_or_else(|| str_of(tx, "account").unwrap_or_default()),
        asset,
        rail: rail_for_tx(tx),
        occurred_at: str_of(tx, "occurredAt"),
        amount_msat: amount.abs(),
        fee_msat: fee.map(i128::abs),
        side: r#as,
        linked: tx_pair.is_some() || internal,
        link: match tx_pair {
            Some(p) => Some(LegLink {
                wallet: str_of(p, if send { "inWallet" } else { "outWallet" }),
                automatic: p["kind"].as_str() == Some("journal-derived"),
            }),
            None if internal => Some(LegLink {
                wallet: None,
                automatic: true,
            }),
            None => None,
        },
        held: tx["quarantineReason"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
            || tx["queueStatus"].as_str() == Some("review"),
        excluded: tx["excluded"].as_bool().unwrap_or(false),
        booked_as: str_of(tx, "type"),
        fiat: if pair.is_some() {
            None
        } else {
            Some(tx["eur"].as_f64().map(f64::abs))
        },
        network: str_of(tx, "network"),
        open: None,
    })
}

/// The leg with what its links leave open (`withOpenPart`).
pub fn with_open_part(leg: LinkLeg, parts: Option<&Parts>) -> LinkLeg {
    match parts.and_then(|p| p.get(leg.side, &leg.id)) {
        Some(open) => LinkLeg {
            linked: true,
            open: Some(*open),
            ..leg
        },
        None => leg,
    }
}

pub fn leg_has_open_part(leg: &LinkLeg) -> bool {
    has_open_part(leg.linked, leg.open.as_ref())
}

/// `sharesWith`: only an ordinary move within one asset takes the rest of a transaction.
pub fn shares_with(send: &LinkLeg, receipt: &LinkLeg) -> bool {
    send.asset == receipt.asset && default_pair_kind(send, receipt) == "manual"
}

/// `linkableMsat`.
pub fn linkable_msat(leg: &LinkLeg) -> i128 {
    if leg_has_open_part(leg) {
        leg.open.unwrap().unlinked_msat
    } else {
        leg.amount_msat
    }
}

/// `impliedFeeMsat`: sent plus the send's network fee less received.
pub fn implied_fee_msat(send: &LinkLeg, receipt: &LinkLeg) -> i128 {
    let fee = if leg_has_open_part(send) {
        0
    } else {
        send.fee_msat.unwrap_or(0)
    };
    linkable_msat(send) + fee - linkable_msat(receipt)
}

/// `feeShareBps`.
pub fn fee_share_bps(send_msat: i128, fee_msat: i128) -> Option<i128> {
    (send_msat > 0).then(|| fee_msat.abs() * 10_000 / send_msat)
}

/// `bpsText`: "3.43".
pub fn bps_text(bps: i128) -> String {
    let whole = bps / 100;
    let rest = format!("{:02}", bps % 100);
    let rest = rest.trim_end_matches('0');
    if rest.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{rest}")
    }
}

fn epoch_ms(v: Option<&str>) -> Option<f64> {
    crate::format::parse_ts(v?).map(|s| s * 1000.0)
}

/// Signed minutes from the send to the receipt.
pub fn minutes_apart(send: &LinkLeg, receipt: &LinkLeg) -> Option<i64> {
    let a = epoch_ms(send.occurred_at.as_deref())?;
    let b = epoch_ms(receipt.occurred_at.as_deref())?;
    Some(((b - a) / 60_000.0).round() as i64)
}

/// `defaultPairKind`.
pub fn default_pair_kind(send: &LinkLeg, receipt: &LinkLeg) -> &'static str {
    if !is_bitcoin(&send.asset) || !is_bitcoin(&receipt.asset) {
        return "manual";
    }
    if send.rail == Rail::Lightning && receipt.rail != Rail::Lightning {
        return "reverse-submarine-swap";
    }
    if receipt.rail == Rail::Lightning && send.rail != Rail::Lightning {
        return "submarine-swap";
    }
    let lbtc = |a: &str| a.to_uppercase().replace('-', "") == "LBTC";
    if !lbtc(&send.asset) && lbtc(&receipt.asset) {
        return "peg-in";
    }
    if lbtc(&send.asset) && !lbtc(&receipt.asset) {
        return "peg-out";
    }
    "manual"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkQuestion {
    None,
    Bitcoin,
    Asset,
}

pub fn link_question(send: &LinkLeg, receipt: &LinkLeg) -> LinkQuestion {
    if send.asset == receipt.asset {
        LinkQuestion::None
    } else if is_bitcoin(&send.asset) && is_bitcoin(&receipt.asset) {
        LinkQuestion::Bitcoin
    } else {
        LinkQuestion::Asset
    }
}

/// `linkAnswer`: "carrying-value" or "taxable".
pub fn link_answer(policy: Option<&str>) -> Option<&'static str> {
    match policy {
        Some("carrying-value") => Some("carrying-value"),
        Some("taxable") => Some("taxable"),
        _ => None,
    }
}

pub fn comparable(send: &LinkLeg, receipt: &LinkLeg) -> bool {
    send.asset == receipt.asset || (is_bitcoin(&send.asset) && is_bitcoin(&receipt.asset))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difference {
    OtherAsset,
    Fee {
        kind: FeeCheck,
        amount_msat: i128,
        share_bps: Option<i128>,
    },
}

impl Difference {
    pub fn kind(&self) -> Option<FeeCheck> {
        match self {
            Difference::OtherAsset => None,
            Difference::Fee { kind, .. } => Some(*kind),
        }
    }
}

/// `differenceOf`.
pub fn difference_of(send: &LinkLeg, receipt: &LinkLeg) -> Difference {
    if !comparable(send, receipt) {
        return Difference::OtherAsset;
    }
    let fee = implied_fee_msat(send, receipt);
    let sent = linkable_msat(send);
    Difference::Fee {
        kind: check_fee(sent, fee),
        amount_msat: fee.abs(),
        share_bps: fee_share_bps(sent, fee),
    }
}

const BEFORE_MINUTES: i64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    Linked,
    Ignored,
    Rest,
}

/// A transaction offered for the open side (`Candidate`).
#[derive(Clone, Debug)]
pub struct Candidate {
    pub leg: LinkLeg,
    pub difference: Option<Difference>,
    pub minutes: Option<i64>,
    pub suggested: bool,
    pub same_txid: bool,
    pub close: bool,
    pub blocked: Option<Blocked>,
    pub same_wallet: bool,
    pub before: bool,
}

fn blocked_by(leg: &LinkLeg) -> Option<Blocked> {
    if leg.excluded {
        Some(Blocked::Ignored)
    } else if leg.linked && !leg_has_open_part(leg) {
        Some(Blocked::Linked)
    } else {
        None
    }
}

/// `rankCandidates`.
pub fn rank_candidates(
    anchor: &LinkLeg,
    legs: &[LinkLeg],
    suggested: &std::collections::HashSet<String>,
) -> Vec<Candidate> {
    let mut ranked: Vec<Candidate> = legs
        .iter()
        .filter(|l| l.id != anchor.id)
        .map(|leg| {
            let (send, receipt) = if anchor.side == LegSide::Send {
                (anchor, leg)
            } else {
                (leg, anchor)
            };
            let difference = difference_of(send, receipt);
            let minutes = minutes_apart(send, receipt);
            let same_wallet = leg.wallet == anchor.wallet;
            let before = minutes.is_some_and(|m| m < -BEFORE_MINUTES);
            let blocked = blocked_by(leg).or_else(|| {
                ((leg_has_open_part(anchor) || leg_has_open_part(leg))
                    && !shares_with(send, receipt))
                .then_some(Blocked::Rest)
            });
            let close = blocked.is_none()
                && !same_wallet
                && !before
                && matches!(
                    difference.kind(),
                    Some(FeeCheck::None | FeeCheck::Plausible)
                )
                && minutes.is_some_and(|m| m.abs() <= 24 * 60);
            let same_txid = match (&anchor.external_id, &leg.external_id) {
                (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
                _ => false,
            };
            Candidate {
                leg: leg.clone(),
                difference: Some(difference),
                minutes,
                suggested: suggested.contains(&leg.id),
                same_txid,
                close,
                blocked,
                same_wallet,
                before,
            }
        })
        .collect();
    let odd = |r: &Candidate| {
        r.same_wallet
            || r.before
            || matches!(r.difference, Some(Difference::OtherAsset))
            || matches!(
                r.difference.and_then(|d| d.kind()),
                Some(FeeCheck::Negative | FeeCheck::Mismatch)
            )
    };
    let tier = |r: &Candidate| {
        if r.blocked.is_some() {
            4
        } else if r.suggested || (r.same_txid && !r.same_wallet) {
            0
        } else if r.close {
            1
        } else if odd(r) {
            3
        } else {
            2
        }
    };
    let gap = |r: &Candidate| match r.difference {
        Some(Difference::Fee { amount_msat, .. }) => amount_msat,
        _ => 0,
    };
    ranked.sort_by(|a, b| {
        let (ta, tb) = (tier(a), tier(b));
        ta.cmp(&tb)
            .then_with(|| {
                if ta == 1 {
                    gap(a).cmp(&gap(b))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| {
                a.minutes
                    .map(i64::abs)
                    .unwrap_or(i64::MAX)
                    .cmp(&b.minutes.map(i64::abs).unwrap_or(i64::MAX))
            })
            .then_with(|| a.leg.id.cmp(&b.leg.id))
    });
    ranked
}

/// `listCandidates`: pickable ones first, as listed.
pub fn list_candidates(legs: &[LinkLeg]) -> Vec<Candidate> {
    let mut rows: Vec<(usize, Candidate)> = legs
        .iter()
        .enumerate()
        .map(|(i, leg)| {
            (
                i,
                Candidate {
                    leg: leg.clone(),
                    difference: None,
                    minutes: None,
                    suggested: false,
                    same_txid: false,
                    close: false,
                    blocked: blocked_by(leg),
                    same_wallet: false,
                    before: false,
                },
            )
        })
        .collect();
    rows.sort_by(|a, b| {
        a.1.blocked
            .is_some()
            .cmp(&b.1.blocked.is_some())
            .then(a.0.cmp(&b.0))
    });
    rows.into_iter().map(|(_, c)| c).collect()
}

/// `byTypedAmount`.
pub fn by_typed_amount(rows: Vec<Candidate>, target: Option<i128>) -> Vec<Candidate> {
    let Some(t) = target else { return rows };
    let mut indexed: Vec<(usize, Candidate)> = rows.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.blocked
            .is_some()
            .cmp(&b.1.blocked.is_some())
            .then_with(|| {
                (linkable_msat(&a.1.leg) - t)
                    .abs()
                    .cmp(&(linkable_msat(&b.1.leg) - t).abs())
            })
            .then(a.0.cmp(&b.0))
    });
    indexed.into_iter().map(|(_, c)| c).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairWarning {
    Negative,
    Mismatch,
    High,
    Before,
    SameWallet,
    SendLinked,
    ReceiptLinked,
    SendIgnored,
    ReceiptIgnored,
}

impl PairWarning {
    pub fn key(self) -> &'static str {
        match self {
            PairWarning::Negative => "negative",
            PairWarning::Mismatch => "mismatch",
            PairWarning::High => "high",
            PairWarning::Before => "before",
            PairWarning::SameWallet => "sameWallet",
            PairWarning::SendLinked => "sendLinked",
            PairWarning::ReceiptLinked => "receiptLinked",
            PairWarning::SendIgnored => "sendIgnored",
            PairWarning::ReceiptIgnored => "receiptIgnored",
        }
    }
}

/// `pairWarnings`: (warning, serious).
pub fn pair_warnings(send: &LinkLeg, receipt: &LinkLeg) -> Vec<(PairWarning, bool)> {
    let mut w = vec![];
    let difference = difference_of(send, receipt);
    let part = leg_has_open_part(send) && shares_with(send, receipt);
    match difference.kind() {
        Some(FeeCheck::Negative) => w.push((PairWarning::Negative, true)),
        Some(FeeCheck::Mismatch) if !part => w.push((PairWarning::Mismatch, true)),
        Some(FeeCheck::High)
            if !part && send.rail == receipt.rail && send.asset == receipt.asset =>
        {
            w.push((PairWarning::High, false))
        }
        _ => {}
    }
    if minutes_apart(send, receipt).is_some_and(|m| m < -BEFORE_MINUTES) {
        w.push((PairWarning::Before, true));
    }
    if send.wallet == receipt.wallet {
        w.push((PairWarning::SameWallet, true));
    }
    if send.excluded {
        w.push((PairWarning::SendIgnored, true));
    } else if send.linked && !(leg_has_open_part(send) && shares_with(send, receipt)) {
        w.push((PairWarning::SendLinked, true));
    }
    if receipt.excluded {
        w.push((PairWarning::ReceiptIgnored, true));
    } else if receipt.linked && !(leg_has_open_part(receipt) && shares_with(send, receipt)) {
        w.push((PairWarning::ReceiptLinked, true));
    }
    w
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso_z(secs: f64) -> String {
    let s = secs.floor() as i64;
    let day = crate::format::Day::from_epoch_days(s.div_euclid(86_400));
    let r = s.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        day.iso(),
        r / 3600,
        (r % 3600) / 60,
        r % 60
    )
}

/// Window choices around the send, in hours; None is any time.
pub const RECEIPT_WINDOWS: [Option<u32>; 6] =
    [Some(6), Some(24), Some(72), Some(168), Some(720), None];

/// `windowKey`: "h72" / "any".
pub fn window_key(h: Option<u32>) -> String {
    h.map(|h| format!("h{h}")).unwrap_or_else(|| "any".into())
}

/// `widerWindow`: the next wider window; None (outer) when there is none.
pub fn wider_window(h: Option<u32>) -> Option<Option<u32>> {
    let i = RECEIPT_WINDOWS.iter().position(|w| *w == h)?;
    h?;
    RECEIPT_WINDOWS.get(i + 1).copied()
}

/// `windowBounds`: (since, until).
pub fn window_bounds(
    occurred_at: Option<&str>,
    hours: Option<u32>,
) -> (Option<String>, Option<String>) {
    let (Some(at), Some(h)) = (occurred_at.and_then(crate::format::parse_ts), hours) else {
        return (None, None);
    };
    let span = h as f64 * 3600.0;
    (Some(iso_z(at - span.min(3600.0))), Some(iso_z(at + span)))
}

/// `windowBoundsBefore`.
pub fn window_bounds_before(
    occurred_at: Option<&str>,
    hours: Option<u32>,
) -> (Option<String>, Option<String>) {
    let (Some(at), Some(h)) = (occurred_at.and_then(crate::format::parse_ts), hours) else {
        return (None, None);
    };
    (
        Some(iso_z(at - h as f64 * 3600.0)),
        Some(iso_z(at + 3600.0)),
    )
}

/// `yearBounds`.
pub fn year_bounds(year: Option<i32>) -> (Option<String>, Option<String>) {
    match year {
        Some(y) => (
            Some(format!("{y}-01-01T00:00:00Z")),
            Some(format!("{y}-12-31T23:59:59Z")),
        ),
        None => (None, None),
    }
}

pub const PICK_LIMIT: u32 = 200;

/// `parseAmountInput`: ₿ or sats, as msat.
pub fn parse_amount_input(raw: &str) -> Option<i128> {
    let text: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '\'' | '’') && !c.is_whitespace())
        .collect();
    if text.is_empty() {
        return None;
    }
    let sats = text.ends_with("sats") || text.ends_with("sat");
    let number = text.trim_end_matches("sats").trim_end_matches("sat");
    let number = number
        .strip_prefix('₿')
        .unwrap_or(number)
        .replacen(',', ".", 1);
    let (whole, frac) = number
        .split_once('.')
        .map(|(a, b)| (a.to_owned(), Some(b.to_owned())))
        .unwrap_or((number.clone(), None));
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let frac = match frac {
        Some(f) if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) => return None,
        Some(f) => f,
        None => String::new(),
    };
    let w: i128 = whole.parse().ok()?;
    if sats {
        if frac.len() > 3 {
            return None;
        }
        Some(w * 1000 + format!("{frac:0<3}").parse::<i128>().unwrap_or(0))
    } else {
        if frac.len() > 11 {
            return None;
        }
        Some(w * 100_000_000_000 + format!("{frac:0<11}").parse::<i128>().unwrap_or(0))
    }
}

/// `searchIntent`: text for the daemon's search, or an amount to sort by.
pub fn search_intent(raw: &str) -> (String, Option<i128>) {
    let text = raw.trim();
    let lower = text.to_lowercase();
    let decimal = {
        let digits_sep = |s: &str| {
            s.chars().all(|c| {
                c.is_ascii_digit() || matches!(c, ' ' | '_' | '\'' | '’') || c.is_whitespace()
            })
        };
        match text.find(['.', ',']) {
            Some(i) if i > 0 => {
                let (a, b) = (&text[..i], &text[i + 1..]);
                a.starts_with(|c: char| c.is_ascii_digit())
                    && digits_sep(a)
                    && !b.is_empty()
                    && b.chars().all(|c| c.is_ascii_digit())
            }
            _ => false,
        }
    };
    let marked =
        text.starts_with('₿') || lower.ends_with("sats") || lower.ends_with("sat") || decimal;
    let amount = if marked {
        parse_amount_input(text)
    } else {
        None
    };
    match amount {
        Some(a) => (String::new(), Some(a)),
        None => (text.to_owned(), None),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelinkPortion {
    Whole,
    Part(i128),
    Unsendable,
}

fn strict_msat(v: &Option<Value>) -> Option<i128> {
    match v.as_ref()? {
        Value::Number(n) => n.as_i64().map(i128::from),
        Value::String(s) if !s.trim().is_empty() => s.trim().parse().ok(),
        _ => None,
    }
}

/// `relinkPortion`.
pub fn relink_portion(
    terms: Option<&RelinkTerms>,
    send_fee_msat: Option<i128>,
    applies: bool,
) -> RelinkPortion {
    let saved = terms.and_then(|t| t.out_amount_msat.as_ref());
    if !applies || saved.is_none() || saved.is_some_and(|v| v.is_null() || v.as_str() == Some("")) {
        return RelinkPortion::Whole;
    }
    let t = terms.unwrap();
    let (part, fee, sent, received) = (
        strict_msat(&t.out_amount_msat),
        strict_msat(&t.swap_fee_msat),
        strict_msat(&t.send_amount_msat),
        strict_msat(&t.receipt_amount_msat),
    );
    let (Some(part), Some(fee), Some(sent), Some(received)) = (part, fee, sent, received) else {
        return RelinkPortion::Unsendable;
    };
    if part <= 0 {
        return RelinkPortion::Unsendable;
    }
    if fee == part - received {
        return if part > 9_007_199_254_740_991 {
            RelinkPortion::Unsendable
        } else {
            RelinkPortion::Part(part)
        };
    }
    if fee == sent - received || send_fee_msat.is_some_and(|f| fee == sent - received + f) {
        return RelinkPortion::Whole;
    }
    RelinkPortion::Unsendable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date_time(Some("2024-09-17T02:14:00Z")), "2024-09-17 02:14");
        assert_eq!(date_time(Some("2024-09-17")), "2024-09-17");
        assert_eq!(date_part(None), "—");
    }

    #[test]
    fn fee_checks() {
        assert_eq!(check_fee(100_000_000, 0), FeeCheck::None);
        assert_eq!(check_fee(100_000_000, 1_000), FeeCheck::Plausible);
        assert_eq!(check_fee(100_000_000, 5_000_000), FeeCheck::High);
        assert_eq!(check_fee(100_000_000, 50_000_000), FeeCheck::Mismatch);
    }

    fn leg(id: &str, side: LegSide, asset: &str, rail: Rail, at: &str, msat: i128) -> LinkLeg {
        LinkLeg {
            id: id.into(),
            external_id: None,
            wallet: format!("w-{id}"),
            asset: asset.into(),
            rail,
            occurred_at: Some(at.into()),
            amount_msat: msat,
            fee_msat: None,
            side,
            linked: false,
            link: None,
            open: None,
            held: false,
            excluded: false,
            booked_as: None,
            fiat: None,
            network: None,
        }
    }

    #[test]
    fn sats_parse_exactly() {
        assert_eq!(
            msat_from_sats(&serde_json::json!(-4200000)),
            Some(-4_200_000_000)
        );
        assert_eq!(msat_from_sats(&serde_json::json!(12.345)), Some(12_345));
        assert_eq!(msat_from_sats(&serde_json::json!(1.2345)), None);
    }

    #[test]
    fn amounts_typed() {
        assert_eq!(parse_amount_input("0,042"), Some(4_200_000_000));
        assert_eq!(parse_amount_input("4 200 000 sats"), Some(4_200_000_000));
        assert_eq!(search_intent("0.042"), (String::new(), Some(4_200_000_000)));
        assert_eq!(search_intent("abc123").1, None);
        assert_eq!(search_intent("123456").1, None);
    }

    #[test]
    fn windows() {
        assert_eq!(wider_window(Some(72)), Some(Some(168)));
        assert_eq!(wider_window(Some(720)), Some(None));
        assert_eq!(wider_window(None), None);
        assert_eq!(
            window_bounds(Some("2024-09-17T02:14:00Z"), Some(6)),
            (
                Some("2024-09-17T01:14:00Z".into()),
                Some("2024-09-17T08:14:00Z".into())
            )
        );
        assert_eq!(bps_text(343), "3.43");
        assert_eq!(bps_text(300), "3");
    }

    #[test]
    fn ranks_close_first_and_blocked_last() {
        let a = leg(
            "s",
            LegSide::Send,
            "BTC",
            Rail::Onchain,
            "2024-01-01T00:00:00Z",
            100_000_000,
        );
        let mut linked = leg(
            "l",
            LegSide::Receipt,
            "BTC",
            Rail::Onchain,
            "2024-01-01T00:10:00Z",
            100_000_000,
        );
        linked.linked = true;
        let close = leg(
            "c",
            LegSide::Receipt,
            "BTC",
            Rail::Onchain,
            "2024-01-01T01:00:00Z",
            99_999_000,
        );
        let far = leg(
            "f",
            LegSide::Receipt,
            "BTC",
            Rail::Onchain,
            "2024-01-01T00:05:00Z",
            50_000_000,
        );
        let rows = rank_candidates(&a, &[linked, far, close], &Default::default());
        assert_eq!(
            rows.iter().map(|r| r.leg.id.as_str()).collect::<Vec<_>>(),
            ["c", "f", "l"]
        );
    }

    #[test]
    fn default_kinds() {
        let s = leg(
            "s",
            LegSide::Send,
            "BTC",
            Rail::Onchain,
            "2024-01-01T00:00:00Z",
            1,
        );
        let r = leg(
            "r",
            LegSide::Receipt,
            "LBTC",
            Rail::Liquid,
            "2024-01-01T00:00:00Z",
            1,
        );
        assert_eq!(default_pair_kind(&s, &r), "peg-in");
        assert_eq!(link_question(&s, &r), LinkQuestion::Bitcoin);
    }

    #[test]
    fn relink_portions() {
        let t = |o: i64, f: i64, s: i64, r: i64| RelinkTerms {
            out_amount_msat: Some(serde_json::json!(o)),
            swap_fee_msat: Some(serde_json::json!(f)),
            send_amount_msat: Some(serde_json::json!(s)),
            receipt_amount_msat: Some(serde_json::json!(r)),
        };
        assert_eq!(
            relink_portion(Some(&t(4_000, 100, 10_000, 3_900)), None, true),
            RelinkPortion::Part(4_000)
        );
        assert_eq!(
            relink_portion(Some(&t(10_000, 150, 10_000, 9_900)), Some(50), true),
            RelinkPortion::Whole
        );
        assert_eq!(
            relink_portion(Some(&t(10_000, 7, 10_000, 9_900)), None, true),
            RelinkPortion::Unsendable
        );
        assert_eq!(relink_portion(None, None, true), RelinkPortion::Whole);
    }
}
