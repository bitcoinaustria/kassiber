//! Pair authoring is reviewed custody, never an ownership inference.
use super::*;
use super::{component_store as store, pair_spec as spec};
use kassiber_custody::{codec, components::review};
use kassiber_envelope::{set, truthy};
use kassiber_pydecimal::{Context, Rounding};
use kassiber_store::rusqlite::params;
use sql::{atomic, error};
use std::collections::BTreeSet;

pub(crate) const KINDS: &[&str] = &[
    "manual",
    "coinjoin",
    "whirlpool",
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];
pub(crate) const POLICIES: &[&str] = &["carrying-value", "taxable"];
const PUBLIC: &[&str] = &[
    "id",
    "component_id",
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
fn present<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    get(args, key).filter(|v| !matches!(v, Value::Null))
}
fn validate_choice(v: &Value, field: &str, choices: &[&str]) -> Result<()> {
    if !choices.iter().any(|s| v == &kassiber_envelope::s(*s)) {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported {field} '{}'. Supported: {}",
                py_str(v),
                choices.join(", ")
            ),
        ));
    }
    Ok(())
}
pub(super) fn resolve(
    c: &Connection,
    scope: &Scope<'_>,
    reference: &Value,
    direction: &str,
) -> Result<Value> {
    let found = kassiber_transactions::lookup::resolve_transaction(
        c,
        scope.profile_id,
        reference,
        Some(direction),
    )?;
    tx(c, &text(&found, "id"))
}
fn tx(c: &Connection, id: &str) -> Result<Value> {
    rows(c,"SELECT t.*,w.kind AS wallet_kind,w.config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.id=?",[id])?.into_iter().next().ok_or_else(||AppError::new("not_found","Custody review boundary was not found"))
}
pub(super) fn boundary(c: &Connection, base: &Value) -> Result<Value> {
    let mut row = base.clone();
    for side in ["out", "in"] {
        if matches!(value(base, &format!("{side}_transaction_id")), Value::Null) {
            continue;
        }
        let t = tx(c, &text(base, &format!("{side}_transaction_id")))?;
        for (dest, src) in [
            ("asset", "asset"),
            ("tx_amount", "amount"),
            ("fee", "fee"),
            ("amount_includes_fee", "amount_includes_fee"),
            ("direction", "direction"),
            ("occurred_at", "occurred_at"),
            ("raw_json", "raw_json"),
            ("wallet_id", "wallet_id"),
            ("wallet_kind", "wallet_kind"),
            ("wallet_config_json", "config_json"),
        ] {
            set(&mut row, &format!("{side}_{dest}"), value(&t, src));
        }
        if side == "out" {
            set(&mut row, "out_external_id", value(&t, "external_id"));
        }
    }
    Ok(row)
}
fn network(out: &Value, inbound: &Value, policy: &str) -> Result<()> {
    if policy != "carrying-value" {
        return Ok(());
    }
    let (od, ov) = review::network_domain_evidence(&codec::to_json(out));
    let (id, iv) = review::network_domain_evidence(&codec::to_json(inbound));
    if !ov || !iv {
        return Err(AppError::new("transfer_network_mismatch","A carrying-value pair has contradictory Bitcoin network metadata.").with_hint("Correct the wallet/transaction chain and network metadata before pairing; conflicting observations cannot carry basis.").with_details(dict([("out_transaction_id",value(out,"id")),("in_transaction_id",value(inbound,"id")),("out_network_valid",Value::Bool(ov)),("in_network_valid",Value::Bool(iv))])));
    }
    if let (Some(od), Some(id)) = (od, id) {
        if od != id {
            return Err(AppError::new("transfer_network_mismatch",format!("A carrying-value pair cannot cross Bitcoin network boundaries ({od} -> {id}).")).with_hint("Correct the wallet/transaction network metadata or leave these transactions unpaired; mainnet, testnet, signet, and regtest are distinct physical value domains.").with_details(dict([("out_transaction_id",value(out,"id")),("in_transaction_id",value(inbound,"id")),("out_network_domain",s(od)),("in_network_domain",s(id))])));
        }
    }
    Ok(())
}
fn policy(c: &Connection, scope: &Scope<'_>, out: &str, inbound: &str, policy: &str) -> Result<()> {
    let same = kassiber_pytext::upper(out) == kassiber_pytext::upper(inbound);
    if same && policy == "taxable" {
        return Err(AppError::new("validation",format!("Same-asset taxable pairs are not supported yet (asset={out}). Leave the legs unpaired to keep normal SELL + BUY treatment, or use --policy carrying-value for a self-transfer.")).with_hint("Re-run with --policy carrying-value, or omit the pair entirely to preserve taxable SELL + BUY behavior."));
    }
    if !same && policy == "carrying-value" {
        let profile = rows(c, "SELECT * FROM profiles WHERE id=?", [scope.profile_id])?.remove(0);
        let country =
            kassiber_pytext::lower(kassiber_pytext::strip(&text(&profile, "tax_country")));
        let rail = |asset: &str| kassiber_pytext::upper(kassiber_pytext::strip(asset));
        let pair = BTreeSet::from([rail(out), rail(inbound)]);
        if !kassiber_tax_countries::cross_asset_carry(&country)
            && pair != BTreeSet::from(["BTC".into(), "LBTC".into()])
        {
            return Err(AppError::new("validation",format!("Cross-asset carrying-value pairs are only supported for Austrian profiles or BTC/LBTC rail swaps right now (out={out}, in={inbound}). Use --policy taxable for other cross-asset swaps.")).with_hint("Re-run with --policy taxable, or pair only BTC/LBTC rail changes as carrying-value outside Austrian profiles."));
        }
    }
    Ok(())
}
pub(super) fn active_refs(c: &Connection, scope: &Scope<'_>) -> Result<Vec<Value>> {
    let mut refs=rows(c,"SELECT term.legacy_source_id AS id,term.component_id,term.term_kind,COALESCE(source.anchor_transaction_id,source.transaction_id) AS out_transaction_id,CASE WHEN term.term_kind='transaction_pair' THEN COALESCE(target.anchor_transaction_id,target.transaction_id) ELSE NULL END AS in_transaction_id,term.review_kind AS kind,term.tax_policy AS policy FROM custody_component_economic_terms term JOIN custody_component_legs source ON source.id=term.source_leg_id JOIN custody_component_legs target ON target.id=term.target_leg_id WHERE term.profile_id=? AND term.component_id IN (SELECT DISTINCT component_id FROM custody_component_transaction_memberships WHERE profile_id=?) ORDER BY term.created_at,term.component_id,term.ordinal",[scope.profile_id,scope.profile_id])?;
    let claimed: BTreeSet<_> = refs
        .iter()
        .flat_map(|r| [text(r, "out_transaction_id"), text(r, "in_transaction_id")])
        .collect();
    for leg in rows(c,"SELECT component_id,role,COALESCE(transaction_id,anchor_transaction_id) AS transaction_id FROM custody_component_legs WHERE profile_id=? AND component_id IN (SELECT DISTINCT component_id FROM custody_component_transaction_memberships WHERE profile_id=?) AND COALESCE(transaction_id,anchor_transaction_id) IS NOT NULL ORDER BY component_id,ordinal,id",[scope.profile_id,scope.profile_id])? {
        if !claimed.contains(&text(&leg,"transaction_id")) {
            let outbound=text(&leg,"role")=="source";
            refs.push(dict([("id",value(&leg,"component_id")),("component_id",value(&leg,"component_id")),("out_transaction_id",if outbound{value(&leg,"transaction_id")}else{Value::Null}),("in_transaction_id",if outbound{Value::Null}else{value(&leg,"transaction_id")}),("kind",s("custody-component")),("policy",Value::Null),("term_kind",s("custody_component"))]));
        }
    }
    for r in &mut refs {
        for side in ["out", "in"] {
            let asset = rows(
                c,
                "SELECT asset FROM transactions WHERE id=?",
                [text(r, &format!("{side}_transaction_id"))],
            )?
            .first()
            .map(|r| value(r, "asset"))
            .unwrap_or(Value::Null);
            set(r, &format!("{side}_asset"), asset);
        }
    }
    Ok(refs)
}
fn reusable(r: &Value) -> bool {
    kassiber_pytext::upper(&text(r, "out_asset")) == kassiber_pytext::upper(&text(r, "in_asset"))
        && text(r, "policy") == "carrying-value"
        && ["manual", "coinjoin", "whirlpool"].contains(&text(r, "kind").as_str())
}
fn reuse(refs: &[Value], row: &Value, exclude: Option<&str>) -> Result<()> {
    for r in refs {
        if text(r, "term_kind") != "transaction_pair"
            || exclude.is_some_and(|id| text(r, "id") == id)
            || (value(r, "out_transaction_id") != value(row, "out_transaction_id")
                && value(r, "in_transaction_id") != value(row, "in_transaction_id"))
        {
            continue;
        }
        if !(reusable(r) && reusable(row)) {
            let leg = if value(r, "out_transaction_id") == value(row, "out_transaction_id") {
                "--tx-out"
            } else {
                "--tx-in"
            };
            let id = text(r, "id");
            return Err(AppError::new("conflict",format!("{leg} already belongs to active pair id={id}. Only same-asset manual, coinjoin, or whirlpool carrying-value links may reuse a transaction leg; cross-asset and layer-transition pairs must remain one-to-one.")).with_hint(format!("Unpair `{id}` first, or use a same-asset privacy/manual pair kind.")));
        }
    }
    Ok(())
}
fn fee(out: &Value, inbound: &Value, kind: &str, amount: &Value) -> (Value, Value) {
    if value(out, "asset") == value(inbound, "asset")
        && ["manual", "coinjoin", "whirlpool"].contains(&kind)
    {
        return (Value::Null, Value::Null);
    }
    let split = !matches!(amount, Value::Null);
    let a = if split {
        spec::n(amount)
    } else {
        spec::n(&value(out, "amount"))
    };
    let network = if split || truthy(&value(out, "amount_includes_fee")) {
        0
    } else {
        spec::n(&value(out, "fee")).max(0)
    };
    (
        spec::num(a - spec::n(&value(inbound, "amount")) + network),
        s("combined"),
    )
}
pub(super) fn amount(v: &Value) -> Result<Value> {
    let decimal = kassiber_pydecimal::finite_price(&py_str(v))
        .map_err(|e| AppError::new("validation", e.message("--out-amount")))?;
    if decimal.signum() != std::cmp::Ordering::Greater {
        return Err(AppError::new("validation", "--out-amount must be positive"));
    }
    let scaled = decimal
        .mul(
            &"100000000000".parse().unwrap(),
            &Context::new(32, Rounding::HalfEven),
        )
        .map_err(|_| AppError::new("internal_error", "[<class 'decimal.Overflow'>]"))?;
    let rounded = scaled
        .to_fixed_places(0, Rounding::HalfUp)
        .map_err(|_| AppError::new("internal_error", "decimal conversion failed"))?;
    Ok(Value::Int(rounded.parse().map_err(|_| {
        AppError::new("internal_error", "decimal conversion failed")
    })?))
}
pub(super) fn finish_residual(c: &Connection, row: &Value, plan: &mut Value) -> Result<()> {
    let reviewed = spec::n(&value(&store::list(plan, "legs")[0], "amount_msat"));
    if !spec::native_residual(c, plan, row, reviewed)? {
        spec::append_residual(plan, row, reviewed, None)?;
    }
    Ok(())
}

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    atomic(c, || match operation {
        "transfers.pair" => create(c, scope, args, sources),
        "transfers.update" => update(c, scope, args, sources),
        _ => delete(c, scope, args, sources),
    })
}
fn create(c: &Connection, scope: &Scope<'_>, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let out = resolve(c, scope, &value(args, "tx_out"), "outbound")?;
    let inbound = resolve(c, scope, &value(args, "tx_in"), "inbound")?;
    let kind = present(args, "kind")
        .cloned()
        .unwrap_or_else(|| s("manual"));
    validate_choice(&kind, "pair kind", KINDS)?;
    let mut chosen = value(args, "policy");
    if !matches!(chosen, Value::Null) {
        validate_choice(&chosen, "pair policy", POLICIES)?;
    }
    let source = present(args, "pair_source")
        .cloned()
        .unwrap_or_else(|| s("manual"));
    validate_choice(
        &source,
        "pair_source",
        &["manual", "bulk_exact", "bulk_selected", "rule_auto"],
    )?;
    if value(&out, "id") == value(&inbound, "id") {
        return Err(AppError::new(
            "validation",
            "--tx-out and --tx-in must reference different transactions",
        ));
    }
    if matches!(chosen, Value::Null) {
        chosen = s(super::rules::recommended_policy(
            c,
            scope,
            &dict([
                ("out_asset", value(&out, "asset")),
                ("in_asset", value(&inbound, "asset")),
            ]),
        )?);
    }
    let policy_text = py_str(&chosen);
    network(&out, &inbound, &policy_text)?;
    policy(
        c,
        scope,
        &text(&out, "asset"),
        &text(&inbound, "asset"),
        &policy_text,
    )?;
    let mut portion = Value::Null;
    if let Some(raw) = present(args, "out_amount") {
        if value(&out, "asset") == value(&inbound, "asset") {
            return Err(AppError::new("validation","--out-amount only applies to cross-asset swap pairs: it is the portion of the outbound that was swapped, with the remainder treated as a same-asset self-transfer."));
        }
        portion = amount(raw)?;
        let full = spec::principal(&out);
        if let Value::Int(p) = &portion {
            if p > &full.into() {
                if p.to_string().len() > 4300 {
                    return Err(AppError::new("internal_error","Exceeds the limit (4300 digits) for integer string conversion; use sys.set_int_max_str_digits() to increase the limit"));
                }
                return Err(AppError::new(
                    "validation",
                    format!("--out-amount exceeds the outbound amount ({p} > {full} msat)."),
                ));
            }
        }
    }
    let refs = active_refs(c, scope)?;
    if let Some(old) = refs.iter().find(|r| {
        text(r, "term_kind") == "transaction_pair"
            && value(r, "out_transaction_id") == value(&out, "id")
            && value(r, "in_transaction_id") == value(&inbound, "id")
    }) {
        let id = text(old, "id");
        return Err(AppError::new("conflict",format!("Those transactions are already paired (pair id={id}). Run `kassiber transfers unpair --pair-id {id}` first.")));
    }
    if let Some(old) = refs.iter().find(|r| {
        text(r, "term_kind") != "transaction_pair"
            && [
                value(r, "out_transaction_id"),
                value(r, "in_transaction_id"),
            ]
            .iter()
            .any(|id| id == &value(&out, "id") || id == &value(&inbound, "id"))
    }) {
        if text(old, "term_kind") == "direct_swap_payout" {
            let id = text(old, "id");
            return Err(AppError::new("conflict",format!("One of the transactions already has an active direct swap payout (id={id}). Delete that payout review before pairing.")).with_hint(format!("Run `kassiber transfers payouts delete --payout-id {id}` first.")));
        }
        return Err(AppError::new(
            "conflict",
            format!(
                "One of the transactions belongs to active custody component {}.",
                text(old, "component_id")
            ),
        )
        .with_hint("Reopen or supersede that custody review before creating a pair.")
        .with_details(dict([("component_id", value(old, "component_id"))])));
    }
    let mut row = dict([
        ("out_transaction_id", value(&out, "id")),
        ("in_transaction_id", value(&inbound, "id")),
        ("kind", kind.clone()),
        ("policy", chosen),
        ("out_asset", value(&out, "asset")),
        ("in_asset", value(&inbound, "asset")),
    ]);
    reuse(&refs, &row, None)?;
    let (fee, fee_kind) = fee(&out, &inbound, &py_str(&kind), &portion);
    for (key, v) in [
        ("id", s(sources.uuid(c)?)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("notes", value(args, "notes")),
        ("swap_fee_msat", fee),
        ("swap_fee_kind", fee_kind),
        ("confidence_at_pair", value(args, "confidence_at_pair")),
        ("pair_source", source),
        ("out_amount", portion),
        ("deleted_at", Value::Null),
        ("created_at", s(sources.now(c)?)),
    ] {
        set(&mut row, key, v);
    }
    let row = boundary(c, &row)?;
    let related: Vec<_> = records::pair_records(c, scope, false)?
        .into_iter()
        .filter(|r| {
            value(r, "out_transaction_id") == value(&row, "out_transaction_id")
                || value(r, "in_transaction_id") == value(&row, "in_transaction_id")
        })
        .collect();
    let mut plan = if related.is_empty() {
        let mut plan = spec::group(std::slice::from_ref(&row), false)?;
        finish_residual(c, &row, &mut plan)?;
        plan
    } else {
        let ids: BTreeSet<_> = related.iter().map(|r| text(r, "component_id")).collect();
        if ids.len() != 1 || ids.contains("") {
            return Err(AppError::new(
                "custody_component_membership_conflict",
                "Connected reviewed pair terms do not have one active component",
            ));
        }
        let active = rows(
            c,
            "SELECT lineage_id FROM custody_components WHERE id=?",
            [ids.first().unwrap()],
        )?
        .remove(0);
        let mut group = related
            .iter()
            .map(|r| boundary(c, r))
            .collect::<Result<Vec<_>>>()?;
        group.push(row.clone());
        let mut plan = spec::group(&group, true)?;
        set(&mut plan, "lineage_id", value(&active, "lineage_id"));
        plan
    };
    let id = store::activate(c, scope, &row, &mut plan, &text_source(args), None)?;
    let mut out = project(&row, PUBLIC);
    set(&mut out, "component_id", s(id));
    store::invalidate(c, scope.profile_id)?;
    Ok(out)
}
fn text_source(args: &Value) -> String {
    present(args, "authored_source")
        .map(py_str)
        .unwrap_or_else(|| "cli".into())
}
fn update(c: &Connection, scope: &Scope<'_>, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let id = text(args, "pair_id");
    let current = records::pair_records(c, scope, false)?;
    let old = current
        .iter()
        .find(|r| text(r, "id") == id)
        .ok_or_else(|| AppError::new("not_found", format!("Pair '{id}' not found")))?;
    let mut revised = old.clone();
    for key in ["kind", "policy"] {
        if let Some(v) = present(args, key) {
            set(&mut revised, key, v.clone());
        }
    }
    validate_choice(&value(&revised, "kind"), "pair kind", KINDS)?;
    validate_choice(&value(&revised, "policy"), "pair policy", POLICIES)?;
    policy(
        c,
        scope,
        &text(old, "out_asset"),
        &text(old, "in_asset"),
        &text(&revised, "policy"),
    )?;
    let out = tx(c, &text(old, "out_transaction_id"))?;
    let inbound = tx(c, &text(old, "in_transaction_id"))?;
    network(&out, &inbound, &text(&revised, "policy"))?;
    reuse(&active_refs(c, scope)?, &revised, Some(&id))?;
    if let Some(notes) = get(args, "notes") {
        set(&mut revised, "notes", notes.clone());
    }
    if ["kind", "policy", "notes"]
        .iter()
        .all(|k| value(old, k) == value(&revised, k))
    {
        return Ok(project(old, PUBLIC));
    }
    if value(old, "kind") != value(&revised, "kind") {
        let portion = if value(old, "out_asset") != value(old, "in_asset") {
            value(old, "out_amount")
        } else {
            Value::Null
        };
        let (fee, kind) = fee(&out, &inbound, &text(&revised, "kind"), &portion);
        set(&mut revised, "swap_fee_msat", fee);
        set(&mut revised, "swap_fee_kind", kind);
    }
    set(&mut revised, "created_at", s(sources.now(c)?));
    let row = boundary(c, &revised)?;
    let group = current
        .iter()
        .filter(|r| value(r, "component_id") == value(old, "component_id"))
        .map(|r| {
            if text(r, "id") == id {
                Ok(row.clone())
            } else {
                boundary(c, r)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut plan = spec::group(&group, group.len() > 1)?;
    if group.len() == 1 {
        finish_residual(c, &row, &mut plan)?;
    }
    let active = rows(
        c,
        "SELECT lineage_id FROM custody_components WHERE id=?",
        [text(old, "component_id")],
    )?
    .remove(0);
    set(&mut plan, "lineage_id", value(&active, "lineage_id"));
    let component = store::activate(c, scope, &row, &mut plan, &text_source(args), None)?;
    let mut result = project(&row, PUBLIC);
    set(&mut result, "component_id", s(component));
    store::invalidate(c, scope.profile_id)?;
    Ok(result)
}
fn delete(c: &Connection, scope: &Scope<'_>, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let id = text(args, "pair_id");
    let result = dict([("deleted", s(&id))]);
    let records = records::pair_records(c, scope, true)?;
    let Some(row) = records.iter().find(|r| text(r, "id") == id) else {
        if !rows(c,"SELECT 1 FROM custody_component_economic_terms WHERE profile_id=? AND legacy_source_id=? AND term_kind='transaction_pair' LIMIT 1",[scope.profile_id,&id])?.is_empty(){return Ok(result);}
        return Err(AppError::new("not_found", format!("Pair '{id}' not found")));
    };
    if truthy(&value(row, "deleted_at")) {
        return Ok(result);
    }
    let active = rows(
        c,
        "SELECT id,lineage_id FROM custody_components WHERE id=?",
        [text(row, "component_id")],
    )?
    .remove(0);
    let remaining = records::pair_records(c, scope, false)?
        .into_iter()
        .filter(|r| value(r, "component_id") == value(row, "component_id") && text(r, "id") != id)
        .map(|r| boundary(c, &r))
        .collect::<Result<Vec<_>>>()?;
    let now = sources.now(c)?;
    if !remaining.is_empty() {
        let mut plan = spec::group(&remaining, false)?;
        set(&mut plan, "lineage_id", value(&active, "lineage_id"));
        let mut activation = remaining[0].clone();
        set(&mut activation, "created_at", s(&now));
        store::activate(
            c,
            scope,
            &activation,
            &mut plan,
            &text_source(args),
            Some(format!("transaction_pair review {id} deleted")),
        )?;
    } else {
        c.execute("UPDATE custody_components SET state='superseded',superseded_at=?,change_reason=COALESCE(?,change_reason) WHERE id=? AND state!='superseded'",params![now,format!("transaction_pair review deleted by {}",text_source(args)),text(&active,"id")]).map_err(error)?;
        store::invalidate(c, scope.profile_id)?;
        let snapshot = store::snapshot(c, scope)?;
        let computed = kassiber_custody::components::compute(&snapshot)
            .map_err(|e| AppError::new("internal_error", e.message))?;
        c.execute(
            "DELETE FROM custody_component_transaction_memberships WHERE profile_id=?",
            [scope.profile_id],
        )
        .map_err(error)?;
        for component in computed["components"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["effective_state"] == "active")
        {
            let ids: BTreeSet<_> = component["legs"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|l| l["transaction_id"].as_str())
                .collect();
            for tx in ids {
                c.execute("INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES(?,?,?,?)",params![component["id"].as_str().unwrap(),scope.profile_id,tx,now]).map_err(error)?;
            }
        }
    }
    store::invalidate(c, scope.profile_id)?;
    // The removed pair's legs and terms, so it can be paired again.
    let mut pair = project(row, PUBLIC);
    for key in ["swap_fee_msat", "out_amount"] {
        let amount = value(&pair, key);
        if !matches!(amount, Value::Null) {
            set(&mut pair, key, Value::Int(integer(&amount)?));
        }
    }
    let mut result = result;
    set(&mut result, "pair", pair);
    Ok(result)
}

fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}
