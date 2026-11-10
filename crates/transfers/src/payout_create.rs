//! Explicit external payout reviews; no automatic matching or ownership inference.
use super::*;
use super::{component_store as store, pair_authoring as pair, pair_spec as spec};
use kassiber_custody::{codec, components::review};
use kassiber_envelope::{set, truthy};
use kassiber_pydecimal::Decimal;

const FIELDS: &[&str] = &[
    "id",
    "workspace_id",
    "profile_id",
    "out_transaction_id",
    "kind",
    "policy",
    "payout_asset",
    "payout_amount",
    "payout_occurred_at",
    "payout_fiat_value",
    "payout_external_id",
    "counterparty",
    "notes",
    "swap_fee_msat",
    "swap_fee_kind",
    "out_amount",
    "deleted_at",
    "created_at",
];
fn present<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    get(args, key).filter(|v| !matches!(v, Value::Null))
}
fn amount(v: &Value, name: &str) -> Result<Value> {
    pair::amount(v).map_err(|mut e| {
        e.message = e.message.replace("--out-amount", name);
        e
    })
}
pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    sql::atomic(c, || create(c, scope, args, sources))
}
fn create(c: &Connection, scope: &Scope<'_>, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let out = pair::resolve(c, scope, &value(args, "tx_out"), "outbound")?;
    let kind = present(args, "kind")
        .cloned()
        .unwrap_or_else(|| s("direct-swap-payout"));
    if kind != s("direct-swap-payout") {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported direct payout kind '{}'. Supported: direct-swap-payout",
                py_str(&kind)
            ),
        ));
    }
    let mut policy = value(args, "policy");
    if !matches!(policy, Value::Null) && policy != s("taxable") && policy != s("carrying-value") {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported direct payout policy '{}'. Supported: carrying-value, taxable",
                py_str(&policy)
            ),
        ));
    }
    let asset = spec::asset(&value(args, "payout_asset"));
    if asset.is_empty() {
        return Err(AppError::new("validation", "--payout-asset is required"));
    }
    let payout = amount(&value(args, "payout_amount"), "--payout-amount")?;
    let portion = match present(args, "out_amount") {
        Some(v) => amount(v, "--out-amount")?,
        None => Value::Null,
    };
    let principal = spec::principal(&out);
    if let Value::Int(n) = &portion {
        if n > &principal.into() {
            if n.to_string().len() > 4300 {
                return Err(AppError::new("internal_error","Exceeds the limit (4300 digits) for integer string conversion; use sys.set_int_max_str_digits() to increase the limit"));
            }
            return Err(AppError::new(
                "validation",
                format!("--out-amount exceeds the outbound amount ({n} > {principal} msat)."),
            ));
        }
    }
    let (fiat, fiat_exact) = match present(args, "payout_fiat_value") {
        None => (Value::Null, Value::Null),
        Some(v) => {
            let d = kassiber_pydecimal::finite_price(&py_str(v))
                .map_err(|e| AppError::new("validation", e.message("--payout-fiat-value")))?;
            for quantity in [&payout, &portion] {
                let quantity = if matches!(quantity, Value::Null) {
                    principal.to_string()
                } else {
                    py_str(quantity)
                };
                let quantity: Decimal = quantity
                    .parse()
                    .map_err(|_| AppError::new("internal_error", "decimal conversion failed"))?;
                if quantity.is_positive() {
                    let btc = quantity
                        .div(
                            &Decimal::from(100_000_000_000_i64),
                            &kassiber_pydecimal::Context::RP2,
                        )
                        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
                    let rate = d
                        .div(&btc, &kassiber_pydecimal::Context::RP2)
                        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
                    kassiber_pydecimal::finite_price(&rate.to_string())
                        .map_err(|e| AppError::new("validation", e.message("payout_fiat_rate")))?;
                }
            }
            if d.signum() == std::cmp::Ordering::Less {
                return Err(AppError::new(
                    "validation",
                    "--payout-fiat-value must not be negative",
                ));
            }
            // The authoring boundary supplies format(value, "f"), then the
            // component canonicalizer uses str(Decimal(text)). Positive
            // exponents therefore become zero; negative scale is retained.
            let exact: Decimal = d
                .to_fixed_string()
                .map_err(|e| AppError::new("internal_error", e.to_string()))?
                .parse()
                .expect("bounded fixed decimal");
            (
                Value::Float(
                    d.to_string().parse::<f64>().map_err(|_| {
                        AppError::new("internal_error", "decimal conversion failed")
                    })?,
                ),
                s(exact.to_string()),
            )
        }
    };
    let when = match present(args, "payout_occurred_at") {
        Some(v) if !kassiber_pytext::strip(&py_str(v)).is_empty() => s(
            review::canonical_timestamp(&codec::to_json(&s(kassiber_pytext::strip(&py_str(v)))))
                .map_err(|_| {
                    AppError::new(
                        "validation",
                        format!("Invalid --payout-occurred-at value '{}'", py_str(v)),
                    )
                    .with_hint("Use an RFC3339 timestamp like 2025-01-01T00:00:00Z.")
                })?,
        ),
        _ => Value::Null,
    };
    if matches!(policy, Value::Null) {
        policy = s(if kassiber_pytext::upper(&text(&out, "asset")) == asset {
            "carrying-value"
        } else {
            super::rules::recommended_policy(
                c,
                scope,
                &dict([("out_asset", value(&out, "asset")), ("in_asset", s(&asset))]),
            )?
        });
    }
    let same = kassiber_pytext::upper(&text(&out, "asset")) == kassiber_pytext::upper(&asset);
    if !same && policy == s("carrying-value") {
        let profile = rows(
            c,
            "SELECT tax_country FROM profiles WHERE id=?",
            [scope.profile_id],
        )?
        .remove(0);
        let rails = std::collections::BTreeSet::from([
            kassiber_pytext::upper(kassiber_pytext::strip(&text(&out, "asset"))),
            asset.clone(),
        ]);
        if !kassiber_tax_countries::cross_asset_carry(&kassiber_pytext::lower(
            kassiber_pytext::strip(&text(&profile, "tax_country")),
        )) && rails != std::collections::BTreeSet::from(["BTC".into(), "LBTC".into()])
        {
            return Err(AppError::new("validation",format!("Cross-asset carrying-value pairs are only supported for Austrian profiles or BTC/LBTC rail swaps right now (out={}, in={asset}). Use --policy taxable for other cross-asset swaps.",text(&out,"asset"))).with_hint("Re-run with --policy taxable, or pair only BTC/LBTC rail changes as carrying-value outside Austrian profiles."));
        }
    }
    if let Some(old) = pair::active_refs(c, scope)?.iter().find(|r| {
        [
            value(r, "out_transaction_id"),
            value(r, "in_transaction_id"),
        ]
        .contains(&value(&out, "id"))
    }) {
        let id = text(old, "id");
        return Err(match text(old,"term_kind").as_str() {
            "transaction_pair"=>AppError::new("conflict",format!("Transaction is already paired (pair id={id}). Run `kassiber transfers unpair --pair-id {id}` first.")),
            "direct_swap_payout"=>AppError::new("conflict",format!("Transaction already has an active direct swap payout (id={id}).")).with_hint("Delete the existing payout review before creating a replacement."),
            _=>AppError::new("conflict",format!("Transaction belongs to active custody component {}.",text(old,"component_id"))).with_hint("Reopen or supersede that custody review before creating a direct payout.").with_details(dict([("component_id",value(old,"component_id"))])),
        });
    }
    if let Value::Int(n) = &payout {
        if n.to_string().len() > 4300 {
            return Err(AppError::new("internal_error","Exceeds the limit (4300 digits) for integer string conversion; use sys.set_int_max_str_digits() to increase the limit"));
        }
        if n > &i64::MAX.into() {
            return Err(AppError::new(
                "custody_component_validation",
                "amount_msat must fit SQLite's non-negative integer range",
            )
            .with_details(dict([
                ("field", s("amount_msat")),
                ("value", payout.clone()),
            ])));
        }
    }
    let reviewed = if matches!(portion, Value::Null) {
        principal
    } else {
        spec::n(&portion)
    };
    let physical = reviewed.max(0);
    let fee = if matches!(portion, Value::Null) && !truthy(&value(&out, "amount_includes_fee")) {
        spec::n(&value(&out, "fee")).max(0)
    } else {
        0
    };
    let mut base = dict([
        ("id", s(sources.uuid(c)?)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("out_transaction_id", value(&out, "id")),
        ("kind", kind),
        ("policy", policy),
        ("payout_asset", s(&asset)),
        ("payout_amount", payout),
        ("payout_occurred_at", when),
        ("payout_fiat_value", fiat),
        ("payout_external_id", value(args, "payout_external_id")),
        ("counterparty", value(args, "counterparty")),
        ("notes", value(args, "notes")),
        ("swap_fee_msat", spec::num(reviewed + fee)),
        ("swap_fee_kind", s("combined")),
        ("out_amount", portion),
        ("deleted_at", Value::Null),
        ("created_at", s(sources.now(c)?)),
    ]);
    let swap = reviewed - spec::n(&value(&base, "payout_amount")) + fee;
    set(&mut base, "swap_fee_msat", spec::num(swap));
    let row = pair::boundary(c, &base)?;
    let mut canonical = project(&row, FIELDS);
    set(&mut canonical, "payout_fiat_value", fiat_exact.clone());
    let hash = store::hash(&canonical, false)?;
    let id = spec::stable(&[
        "direct_swap_payout",
        scope.profile_id,
        &text(&base, "id"),
        &hash,
    ]);
    let source = spec::stable(&[&id, "leg", "source"]);
    let target = spec::stable(&[&id, "leg", "target"]);
    let tx = dict([
        ("transaction_id", value(&out, "id")),
        ("wallet_id", value(&out, "wallet_id")),
        ("wallet_kind", value(&out, "wallet_kind")),
        ("config_json", value(&out, "config_json")),
        ("raw_json", value(&out, "raw_json")),
        ("asset", value(&out, "asset")),
    ]);
    let mut sink = spec::leg(
        &dict([("asset", s(&asset))]),
        &target,
        "retained",
        spec::n(&value(&base, "payout_amount")),
        present(&base, "payout_occurred_at")
            .cloned()
            .unwrap_or_else(|| value(&out, "occurred_at")),
        true,
        physical,
    );
    set(&mut sink, "rail", s("untracked"));
    set(&mut sink, "chain", Value::Null);
    set(&mut sink, "network", Value::Null);
    set(
        &mut sink,
        "location_ref",
        s(format!("legacy-payout:{}", text(&base, "id"))),
    );
    let mut term = dict([
        (
            "id",
            s(spec::stable(&[
                &id,
                "term",
                "direct_swap_payout",
                &text(&base, "id"),
            ])),
        ),
        ("source_leg_id", s(&source)),
        ("target_leg_id", s(&target)),
        ("term_kind", s("direct_swap_payout")),
        ("legacy_source_id", value(&base, "id")),
        ("source_row_hash", s(hash)),
        ("review_kind", value(&base, "kind")),
        ("tax_policy", value(&base, "policy")),
        ("reviewed_source_amount_msat", spec::num(reviewed)),
        ("swap_fee_msat", value(&base, "swap_fee_msat")),
        ("swap_fee_kind", s("combined")),
        ("confidence_at_review", Value::Null),
        ("review_source", Value::Null),
        ("review_notes", value(&base, "notes")),
        ("payout_fiat_value_exact", fiat_exact),
        ("payout_amount_msat", value(&base, "payout_amount")),
    ]);
    for key in [
        "payout_asset",
        "payout_occurred_at",
        "payout_external_id",
        "counterparty",
    ] {
        set(&mut term, key, value(&base, key));
    }
    let mut plan = dict([
        ("component_id", s(&id)),
        (
            "lineage_id",
            s(spec::stable(&[
                "direct_swap_payout",
                scope.profile_id,
                &text(&base, "id"),
                "lineage",
            ])),
        ),
        ("component_type", s("swap")),
        ("conservation_mode", s("conversion")),
        ("conversion_policy", value(&base, "policy")),
        ("conversion_reviewed", Value::Bool(true)),
        (
            "legs",
            Value::List(vec![
                spec::leg(
                    &tx,
                    &source,
                    "source",
                    physical,
                    value(&out, "occurred_at"),
                    true,
                    physical,
                ),
                sink,
            ]),
        ),
        (
            "allocations",
            Value::List(vec![spec::allocation(
                spec::stable(&[&id, "allocation", "0"]),
                &source,
                &target,
                physical,
                spec::n(&value(&base, "payout_amount")),
            )]),
        ),
        ("terms", Value::List(vec![term])),
    ]);
    pair::finish_residual(c, &row, &mut plan)?;
    let source = present(args, "authored_source")
        .map(py_str)
        .unwrap_or_else(|| "cli".into());
    store::activate(c, scope, &row, &mut plan, &source, None)?;
    store::invalidate(c, scope.profile_id)?;
    let mut result = dict(
        [
            ("invalid", Value::Bool(false)),
            ("validation_error", Value::Null),
        ]
        .into_iter()
        .chain(FIELDS.iter().map(|key| (*key, value(&base, key)))),
    );
    let mut fields = match result {
        Value::Dict(v) => v,
        _ => unreachable!(),
    };
    fields.insert(3, ("component_id".into(), value(&plan, "component_id")));
    result = Value::Dict(fields);
    let payout = value(&base, "payout_amount");
    set(&mut result, "payout_amount", super::records::btc(&payout));
    if let Value::Dict(fields) = &mut result {
        let i = fields
            .iter()
            .position(|(k, _)| *k == "payout_amount".into())
            .unwrap();
        fields.insert(i + 1, ("payout_amount_msat".into(), payout));
    }
    Ok(result)
}

fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}
