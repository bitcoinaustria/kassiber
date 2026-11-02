use super::{string, text, zero, Leg};
use crate::{
    codec,
    components::time,
    evidence,
    python_text::{self, PythonText},
};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::{collections::BTreeSet, str::FromStr};

pub(super) enum Row<'a> {
    Value(&'a Value),
    Fields(&'a serde_json::Map<String, Value>),
    Record(&'a crate::snapshot::Record),
}
impl<'a> From<&'a crate::snapshot::Record> for Row<'a> {
    fn from(row: &'a crate::snapshot::Record) -> Self {
        Self::Record(row)
    }
}
impl<'a> From<&'a Value> for Row<'a> {
    fn from(row: &'a Value) -> Self {
        Self::Value(row)
    }
}
impl<'a> From<&'a serde_json::Map<String, Value>> for Row<'a> {
    fn from(row: &'a serde_json::Map<String, Value>) -> Self {
        Self::Fields(row)
    }
}
impl Row<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Value(row) => row.get(key),
            Self::Fields(row) => row.get(key),
            Self::Record(row) => row.get(key),
        }
    }
    fn scope(&self) -> Result<(String, String), crate::components::Error> {
        evidence::scope_fields(|key| &self[key])
    }
}
impl std::ops::Index<&str> for Row<'_> {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&Value::Null)
    }
}

pub(super) fn exact(v: &Value) -> Result<Option<BigInt>, String> {
    if v.is_number() && !v.to_string().contains(['.', 'e', 'E']) {
        let n = if let Some(n) = v.as_i64() {
            BigInt::from(n)
        } else if let Some(n) = v.as_u64() {
            BigInt::from(n)
        } else {
            BigInt::from_str(&v.to_string()).expect("JSON integer")
        };
        return Ok((n >= zero()).then_some(n));
    }
    if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
        if s.chars().all(python_text::isdigit) {
            // isdigit includes superscripts, which int(str) refuses.
            let n = codec::integer(v).map_err(|_| "ValueError".to_owned())?;
            return Ok(Some(
                BigInt::from_str(&n.to_string()).expect("converted integer"),
            ));
        }
    }
    Ok(None)
}
fn truthy(v: &Value) -> bool {
    v == true
        || v.as_f64() == Some(1.0)
        || ["true", "yes", "on"].contains(&text(v).py_strip().py_lower().as_str())
}
fn lower(v: &Value) -> String {
    text(v).py_strip().py_lower()
}
const SAMOURAI_KINDS: &[&str] = &["samourai", "samourai-whirlpool", "whirlpool"];
const SAMOURAI_TRANSACTIONS: &[&str] = &[
    "samourai_deposit",
    "samourai_tx0",
    "samourai_premix",
    "samourai_postmix",
    "samourai_badbank",
    "whirlpool_tx0",
    "whirlpool_mix",
    "premix",
    "postmix",
    "badbank",
    "coinjoin",
];
fn policy(role: &Value, section: &Value) -> bool {
    ["parent", "child"].contains(&lower(role).as_str())
        && ["deposit", "badbank", "premix", "postmix", "ricochet"]
            .contains(&lower(section).as_str())
}
fn codes(row: &Row<'_>) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut signals = BTreeSet::new();
    let mut disqualifiers = BTreeSet::new();
    if ["coinjoin", "payjoin", "whirlpool"].contains(&lower(&row["privacy_boundary"]).as_str()) {
        signals.insert("structured_privacy_boundary".into());
    }
    if SAMOURAI_KINDS.contains(&lower(&row["wallet_kind"]).as_str()) {
        signals.insert("structured_samourai_wallet".into());
    }
    if SAMOURAI_TRANSACTIONS.contains(&lower(&row["kind"]).as_str()) {
        signals.insert("structured_samourai_transaction".into());
    }
    let kind = if row["kind_override"].is_null() || row["kind_override"] == "" {
        &row["kind"]
    } else {
        &row["kind_override"]
    };
    // normalized_transaction_kind uses str(kind), including false and zero.
    let economic = if kind.is_null() {
        String::new()
    } else {
        string(kind).py_strip().py_lower().replace(['-', ' '], "_")
    };
    if [
        "income",
        "revenue",
        "sale",
        "sell",
        "exchange_buy",
        "customer_payment",
    ]
    .contains(&economic.as_str())
        || lower(&row["direction"]) == "inbound"
            && [
                "buy",
                "airdrop",
                "hardfork",
                "hard_fork",
                "income",
                "interest",
                "lending_interest",
                "mining",
                "mining_reward",
                "routing_income",
                "staking",
                "wages",
            ]
            .contains(&economic.as_str())
    {
        disqualifiers.insert("structured_external_origin".into());
    }
    let config = codec::object(&row["wallet_config_json"]);
    let samourai = &config["samourai"];
    if samourai.is_object() && policy(&samourai["role"], &samourai["section"])
        || policy(&row["samourai_role"], &row["samourai_section"])
    {
        signals.insert("structured_samourai_policy".into());
    }
    (signals, disqualifiers)
}

pub(super) fn leg<'a>(
    row: impl Into<Row<'a>>,
    ignored: &BTreeSet<String>,
) -> Result<Option<Leg>, String> {
    let row = row.into();
    let id = text(&row["id"]).py_strip().to_owned();
    if id.is_empty() || ignored.contains(&id) || truthy(&row["excluded"]) {
        return Ok(None);
    }
    let direction = lower(&row["direction"]);
    if !["outbound", "inbound"].contains(&direction.as_str()) {
        return Ok(None);
    }
    let profile_id = text(&row["profile_id"]).py_strip().to_owned();
    let wallet_id = text(&row["wallet_id"]).py_strip().to_owned();
    let asset = text(&row["asset"]).py_strip().py_upper();
    let when = time::instant(&row["occurred_at"]).map_err(|e| e.code.to_owned())?;
    let amount = exact(row.get("amount_msat").unwrap_or(&row["amount"]))?.filter(|v| *v > zero());
    if profile_id.is_empty()
        || wallet_id.is_empty()
        || asset.is_empty()
        || when.is_none()
        || amount.is_none()
    {
        return Ok(None);
    }
    let Some(fee) = exact(row.get("fee").unwrap_or(&json!(0)))? else {
        return Ok(None);
    };
    let outbound = direction == "outbound";
    let fee = if outbound { fee } else { zero() };
    let amount = amount.expect("admitted positive amount");
    let principal = if outbound && truthy(&row["amount_includes_fee"]) {
        &amount - &fee
    } else {
        amount
    };
    let debit = if outbound {
        &principal + &fee
    } else {
        principal.clone()
    };
    if principal <= zero() {
        return Ok(None);
    }
    let Ok((chain, network)) = row.scope() else {
        return Ok(None);
    };
    let (signals, disqualifiers) = codes(&row);
    Ok(Some(Leg {
        id,
        profile_id,
        wallet_id: wallet_id.clone(),
        wallet_label: if codec::truth(&row["wallet_label"]) {
            string(&row["wallet_label"])
        } else {
            wallet_id
        },
        when: when.expect("admitted timestamp"),
        direction,
        asset,
        chain,
        network,
        principal,
        fee,
        debit,
        signals,
        disqualifiers,
        rank: 0,
        small: 0,
    }))
}

/// Format the UTC instant with datetime.isoformat's auto microseconds.
pub(crate) fn iso(when: i64) -> String {
    let day = when / 86_400_000_000;
    let mut lo = 1_i64;
    let mut hi = 10_000_i64;
    let prior_days = |y: i64| {
        let y = y - 1;
        y * 365 + y / 4 - y / 100 + y / 400
    };
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if prior_days(mid) <= day {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let year = lo;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut remaining = day - prior_days(year);
    let mut month = 0;
    while remaining >= months[month] {
        remaining -= months[month];
        month += 1;
    }
    let clock = when % 86_400_000_000;
    let second = clock / 1_000_000;
    let micro = clock % 1_000_000;
    let fraction = if micro == 0 {
        String::new()
    } else {
        format!(".{micro:06}")
    };
    format!(
        "{year:04}-{:02}-{:02}T{:02}:{:02}:{:02}{fraction}Z",
        month + 1,
        remaining + 1,
        second / 3600,
        second / 60 % 60,
        second % 60
    )
}
