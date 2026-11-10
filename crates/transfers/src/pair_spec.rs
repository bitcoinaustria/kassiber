//! Deterministic Python-compatible reviewed pair group construction.
use super::component_store::{hash, list};
use super::*;
use bitcoin_hashes::{sha1, Hash};
use kassiber_custody::{codec, components::review};
use kassiber_envelope::{set, truthy};
use std::collections::BTreeMap;

pub(super) const FIELDS: &[&str] = &[
    "id",
    "workspace_id",
    "profile_id",
    "out_transaction_id",
    "in_transaction_id",
    "kind",
    "policy",
    "notes",
    "swap_fee_msat",
    "swap_fee_kind",
    "confidence_at_pair",
    "pair_source",
    "out_amount",
    "deleted_at",
    "created_at",
];
pub(super) fn stable(parts: &[&str]) -> String {
    let mut bytes = vec![
        0x95, 0xed, 0x14, 0x8a, 0x74, 0x3a, 0x4d, 0xcc, 0x9b, 0x55, 0xca, 0x8c, 0xc2, 0x03, 0xd5,
        0x47,
    ];
    bytes.extend(parts.join("\u{1f}").as_bytes());
    let mut h = sha1::Hash::hash(&bytes).to_byte_array();
    h[6] = (h[6] & 15) | 80;
    h[8] = (h[8] & 63) | 128;
    let hex: String = h[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
pub(super) fn n(v: &Value) -> i128 {
    match v {
        Value::Int(n) => n.to_string().parse().expect("database integer arithmetic"),
        _ => 0,
    }
}
pub(super) fn num(v: i128) -> Value {
    Value::Int(v.into())
}
pub(super) fn principal(row: &Value) -> i128 {
    let amount = n(&value(row, "amount"));
    if kassiber_pytext::lower(kassiber_pytext::strip(&text(row, "direction"))) == "outbound"
        && truthy(&value(row, "amount_includes_fee"))
    {
        amount - n(&value(row, "fee"))
    } else {
        amount
    }
}
/// `row_boundary_amounts(row).wallet_movement_msat`: an outbound row's
/// complete debit (principal plus fee); any other row's observed amount.
fn wallet_movement(row: &Value) -> i128 {
    if kassiber_pytext::lower(kassiber_pytext::strip(&text(row, "direction"))) == "outbound" {
        principal(row) + n(&value(row, "fee"))
    } else {
        n(&value(row, "amount"))
    }
}
fn tx(row: &Value, prefix: &str) -> Value {
    dict([
        (
            "transaction_id",
            value(row, &format!("{prefix}_transaction_id")),
        ),
        ("wallet_id", value(row, &format!("{prefix}_wallet_id"))),
        ("wallet_kind", value(row, &format!("{prefix}_wallet_kind"))),
        (
            "config_json",
            value(row, &format!("{prefix}_wallet_config_json")),
        ),
        ("raw_json", value(row, &format!("{prefix}_raw_json"))),
        ("asset", value(row, &format!("{prefix}_asset"))),
    ])
}
fn boundary(row: &Value, prefix: &str) -> Value {
    dict([
        ("direction", value(row, &format!("{prefix}_direction"))),
        ("amount", value(row, &format!("{prefix}_tx_amount"))),
        ("fee", value(row, &format!("{prefix}_fee"))),
        (
            "amount_includes_fee",
            value(row, &format!("{prefix}_amount_includes_fee")),
        ),
    ])
}
pub(super) fn asset(v: &Value) -> String {
    let raw = if kassiber_envelope::truthy(v) {
        py_str(v)
    } else {
        String::new()
    };
    let text = kassiber_pytext::strip(&raw);
    let lower = kassiber_pytext::lower(text);
    if lower.len() == 64 && lower.bytes().all(|b| b.is_ascii_hexdigit()) {
        lower
    } else if ["l-btc", "lbtc"].contains(&lower.as_str()) {
        "LBTC".into()
    } else {
        kassiber_pytext::upper(text)
    }
}

pub(super) fn leg(
    row: &Value,
    id: &str,
    role: &str,
    amount: i128,
    when: Value,
    conversion: bool,
    valuation: i128,
) -> Value {
    let (rail, chain, network) = review::protocol_scope(&codec::to_json(row));
    let asset = asset(&value(row, "asset"));
    dict([
        ("id", s(id)),
        ("role", s(role)),
        ("rail", s(rail)),
        ("chain", chain.map(s).unwrap_or(Value::Null)),
        ("network", network.map(s).unwrap_or(Value::Null)),
        ("asset", s(&asset)),
        (
            "exposure",
            s(if ["BTC", "LBTC"].contains(&asset.as_str()) {
                String::from("bitcoin")
            } else {
                format!("asset:{}", kassiber_pytext::lower(&asset))
            }),
        ),
        ("conservation_unit", s("msat")),
        ("amount_msat", num(amount)),
        (
            "valuation_unit",
            if conversion {
                s("reviewed_source_msat")
            } else {
                Value::Null
            },
        ),
        (
            "valuation_amount",
            if conversion {
                num(valuation)
            } else {
                Value::Null
            },
        ),
        ("occurred_at", when),
        ("transaction_id", value(row, "transaction_id")),
        ("wallet_id", value(row, "wallet_id")),
        ("location_ref", Value::Null),
    ])
}
pub(super) fn allocation(id: String, source: &str, sink: &str, a: i128, b: i128) -> Value {
    dict([
        ("id", s(id)),
        ("source_leg_id", s(source)),
        ("sink_leg_id", s(sink)),
        ("source_amount_msat", num(a)),
        ("sink_amount_msat", num(b)),
    ])
}
pub(super) fn group(rows: &[Value], full: bool) -> Result<Value> {
    let mut ordered = rows.to_vec();
    ordered.sort_by_key(|r| (text(r, "created_at"), text(r, "id")));
    let hashes = ordered
        .iter()
        .map(|r| hash(&project(r, FIELDS), false))
        .collect::<Result<Vec<_>>>()?;
    group_hashed(&ordered, full, &hashes)
}

pub(super) fn group_hashed(ordered: &[Value], full: bool, hashes: &[String]) -> Result<Value> {
    let ids: Vec<_> = ordered.iter().map(|r| text(r, "id")).collect();
    let profile = text(&ordered[0], "profile_id");
    let mut parts = vec!["transaction_pair_group", &profile];
    parts.extend(hashes.iter().map(String::as_str));
    let id = stable(&parts);
    parts.truncate(2);
    parts.extend(ids.iter().map(String::as_str));
    let lineage = stable(&parts);
    let conversion = ordered.iter().any(|r| {
        asset(&value(r, "out_asset")) != asset(&value(r, "in_asset"))
            || text(r, "policy") != "carrying-value"
    });
    if conversion && ordered.len() != 1 {
        return Err(AppError::new(
            "custody_legacy_group_ambiguous",
            "a connected reviewed conversion group is ambiguous",
        )
        .with_details(dict([(
            "pair_ids",
            Value::List(ids.iter().map(s).collect()),
        )])));
    }
    let mut used = BTreeMap::<String, i128>::new();
    let mut targets = BTreeMap::<String, i128>::new();
    let mut totals = BTreeMap::new();
    let mut legs = Vec::new();
    let mut allocations = Vec::new();
    let mut terms = Vec::new();
    for (i, row) in ordered.iter().enumerate() {
        let source_id = text(row, "out_transaction_id");
        let target_id = text(row, "in_transaction_id");
        let source_boundary = boundary(row, "out");
        let total = principal(&source_boundary);
        // A full review conserves the complete debit, fee included.
        totals.insert(source_id.clone(), wallet_movement(&source_boundary));
        let available = total - used.get(&source_id).unwrap_or(&0);
        let target = principal(&boundary(row, "in")) - targets.get(&target_id).unwrap_or(&0);
        let explicit = value(row, "out_amount");
        let requested = if matches!(explicit, Value::Null) || explicit == s("") {
            available.into()
        } else if let Value::Int(n) = explicit {
            n
        } else {
            0.into()
        };
        if requested > available.into() {
            return Err(AppError::new(
                "custody_legacy_group_invalid",
                "a reviewed pair exceeds its available source quantity",
            )
            .with_details(dict([
                ("pair_id", value(row, "id")),
                ("requested_msat", Value::Int(requested.clone())),
                ("available_msat", num(available)),
            ])));
        }
        let requested = if requested <= 0.into() {
            0
        } else {
            requested.to_string().parse::<i128>().map_err(|_| {
                AppError::new(
                    "internal_error",
                    "reviewed quantity exceeds available integer range",
                )
            })?
        };
        let a = if conversion {
            requested.min(available)
        } else {
            requested.min(available).min(target)
        };
        let b = if conversion { target } else { a };
        if a <= 0 || b <= 0 {
            return Err(AppError::new(
                "custody_legacy_group_invalid",
                "a reviewed pair has no remaining positive component slice",
            )
            .with_details(dict([("pair_id", value(row, "id"))])));
        }
        *used.entry(source_id).or_default() += a;
        *targets.entry(target_id).or_default() += b;
        let ordinal = i.to_string();
        let source = stable(&[&id, "source", &ordinal]);
        let sink = stable(&[&id, "target", &ordinal]);
        legs.push(leg(
            &tx(row, "out"),
            &source,
            "source",
            a,
            value(row, "out_occurred_at"),
            conversion,
            a,
        ));
        legs.push(leg(
            &tx(row, "in"),
            &sink,
            "destination",
            b,
            value(row, "in_occurred_at"),
            conversion,
            a,
        ));
        allocations.push(allocation(
            stable(&[&id, "allocation", &ordinal]),
            &source,
            &sink,
            a,
            b,
        ));
        terms.push(dict([
            ("id", s(stable(&[&id, "term", &ordinal]))),
            ("source_leg_id", s(&source)),
            ("target_leg_id", s(&sink)),
            ("term_kind", s("transaction_pair")),
            ("legacy_source_id", value(row, "id")),
            ("source_row_hash", s(&hashes[i])),
            ("review_kind", value(row, "kind")),
            ("tax_policy", value(row, "policy")),
            ("reviewed_source_amount_msat", num(a)),
            ("swap_fee_msat", value(row, "swap_fee_msat")),
            ("swap_fee_kind", value(row, "swap_fee_kind")),
            ("confidence_at_review", value(row, "confidence_at_pair")),
            ("review_source", value(row, "pair_source")),
            ("review_notes", value(row, "notes")),
        ]));
    }
    let mut spec = dict([
        ("component_id", s(id)),
        ("lineage_id", s(lineage)),
        (
            "component_type",
            s(if conversion { "swap" } else { "manual_bridge" }),
        ),
        (
            "conservation_mode",
            s(if conversion { "conversion" } else { "quantity" }),
        ),
        (
            "conversion_policy",
            if conversion {
                value(&ordered[0], "policy")
            } else {
                Value::Null
            },
        ),
        ("conversion_reviewed", Value::Bool(conversion)),
        ("legs", Value::List(legs)),
        ("allocations", Value::List(allocations)),
        ("terms", Value::List(terms)),
    ]);
    if full {
        for (source, total) in totals {
            if used[&source] != total {
                let index = ordered
                    .iter()
                    .position(|r| text(r, "out_transaction_id") == source)
                    .unwrap();
                let source_leg = text(&list(&spec, "legs")[index * 2], "id");
                append_residual(&mut spec, &ordered[index], used[&source], Some(&source_leg))?;
            }
        }
    }
    Ok(spec)
}
pub(super) fn append_residual(
    spec: &mut Value,
    row: &Value,
    reviewed: i128,
    source_id: Option<&str>,
) -> Result<()> {
    if text(spec, "conservation_mode") == "conversion" {
        return Ok(());
    }
    let boundary = boundary(row, "out");
    let residual = wallet_movement(&boundary) - reviewed;
    if residual <= 0 {
        return Ok(());
    }
    // The observed debit includes the miner fee in addition to principal:
    // allocate that fee once and classify only the remaining principal
    // shortfall through the review (a refund's additional fee included).
    let fee = n(&value(&boundary, "fee"));
    let classification = if text(row, "kind") == "swap-refund" {
        "network_fee"
    } else {
        "suspense_continuation"
    };
    let mut parts = Vec::new();
    let mut principal_residual = residual;
    if fee > 0 && fee <= residual {
        parts.push(("network_fee", fee, true));
        principal_residual -= fee;
    }
    if principal_residual != 0 {
        parts.push((classification, principal_residual, false));
    }
    let index = list(spec, "legs")
        .iter()
        .position(|l| source_id.is_none_or(|id| text(l, "id") == id))
        .unwrap();
    let mut source = list(spec, "legs")[index].clone();
    let amount = n(&value(&source, "amount_msat")) + residual;
    set(&mut source, "amount_msat", num(amount));
    let source_id = text(&source, "id");
    let id = text(spec, "component_id");
    if let Some(Value::List(legs)) = get_mut(spec, "legs") {
        legs[index] = source.clone();
    }
    for (kind, amount, observed_fee) in parts {
        let key = if observed_fee {
            "observed_network_fee"
        } else {
            kind
        };
        let sink_id = stable(&[&id, "leg", "residual-sink", &source_id, key]);
        let mut sink = source.clone();
        set(&mut sink, "id", s(&sink_id));
        set(&mut sink, "amount_msat", num(amount));
        set(&mut sink, "valuation_unit", Value::Null);
        set(&mut sink, "valuation_amount", Value::Null);
        set(
            &mut sink,
            "role",
            s(if kind == "network_fee" {
                "fee"
            } else {
                "suspense"
            }),
        );
        set(&mut sink, "transaction_id", Value::Null);
        set(&mut sink, "wallet_id", Value::Null);
        set(
            &mut sink,
            "notes",
            s(if observed_fee {
                "observed_network_fee".to_owned()
            } else {
                format!("reviewed_residual:{kind}")
            }),
        );
        set(
            &mut sink,
            "location_ref",
            s(format!(
                "{}:{}",
                if kind == "network_fee" {
                    "network-fee"
                } else {
                    "reviewed-custody-suspense"
                },
                text(row, "id")
            )),
        );
        push(spec, "legs", sink);
        push(
            spec,
            "allocations",
            allocation(
                stable(&[&id, "allocation", "residual", &source_id, key]),
                &source_id,
                &sink_id,
                amount,
                amount,
            ),
        );
    }
    Ok(())
}
fn get_mut<'a>(v: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    if let Value::Dict(fields) = v {
        fields
            .iter_mut()
            .find(|(k, _)| *k == key.into())
            .map(|(_, v)| v)
    } else {
        None
    }
}
fn push(v: &mut Value, key: &str, row: Value) {
    if let Some(Value::List(rows)) = get_mut(v, key) {
        rows.push(row);
    }
}
pub(super) fn native_residual(
    c: &Connection,
    spec: &mut Value,
    row: &Value,
    reviewed: i128,
) -> Result<bool> {
    if text(spec, "conservation_mode") != "conversion" {
        return Ok(false);
    }
    let residual = principal(&boundary(row, "out")) - reviewed;
    let external = text(row, "out_external_id");
    let external = kassiber_pytext::strip(&external);
    if residual <= 0 || external.is_empty() {
        return Ok(false);
    }
    let found=rows(c,"SELECT t.id AS transaction_id,t.wallet_id,t.asset,t.occurred_at,t.raw_json,w.kind AS wallet_kind,w.config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 AND t.direction='inbound' AND upper(t.asset)=upper(?) AND lower(t.external_id)=lower(?) AND t.amount=? ORDER BY t.occurred_at,t.created_at,t.id LIMIT 2",kassiber_store::rusqlite::params![text(row,"profile_id"),text(row,"out_asset"),external,sql::bind(&num(residual),4)?])?;
    if found.len() != 1 {
        return Ok(false);
    }
    let id = text(spec, "component_id");
    let sink_id = stable(&[&id, "leg", "exact-native-residual"]);
    let mut source = list(spec, "legs")[0].clone();
    let amount = n(&value(&source, "amount_msat")) + residual;
    set(&mut source, "amount_msat", num(amount));
    let valuation = n(&value(&source, "valuation_amount")) + residual;
    set(&mut source, "valuation_amount", num(valuation));
    let source_id = text(&source, "id");
    if let Some(Value::List(legs)) = get_mut(spec, "legs") {
        legs[0] = source;
    }
    push(
        spec,
        "legs",
        leg(
            &found[0],
            &sink_id,
            "retained",
            residual,
            value(&found[0], "occurred_at"),
            true,
            residual,
        ),
    );
    push(
        spec,
        "allocations",
        allocation(
            stable(&[&id, "allocation", "exact-native-residual"]),
            &source_id,
            &sink_id,
            residual,
            residual,
        ),
    );
    Ok(true)
}
pub(super) fn retarget(spec: &mut Value, prior: &str) {
    let id = stable(&["legacy_review_revision", &text(spec, "component_id"), prior]);
    let mut leg_ids = BTreeMap::new();
    if let Some(Value::List(legs)) = get_mut(spec, "legs") {
        for (i, leg) in legs.iter_mut().enumerate() {
            let new = stable(&[&id, "leg", &i.to_string()]);
            leg_ids.insert(text(leg, "id"), new.clone());
            set(leg, "id", s(new));
        }
    }
    for (key, kind, endpoints) in [
        (
            "allocations",
            "allocation",
            ["source_leg_id", "sink_leg_id"],
        ),
        ("terms", "term", ["source_leg_id", "target_leg_id"]),
    ] {
        if let Some(Value::List(rows)) = get_mut(spec, key) {
            for (i, row) in rows.iter_mut().enumerate() {
                set(row, "id", s(stable(&[&id, kind, &i.to_string()])));
                for endpoint in endpoints {
                    set(row, endpoint, s(&leg_ids[&text(row, endpoint)]));
                }
            }
        }
    }
    set(spec, "component_id", s(id));
}

fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}
