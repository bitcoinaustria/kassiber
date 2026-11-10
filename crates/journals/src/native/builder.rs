use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct State<'a> {
    pub ledger: Ledger,
    custody: &'a Value,
    quantity_differences: Value,
    ownership_review_counts: Value,
    ownership_review_candidates: Value,
}

impl State<'_> {
    pub fn snapshot(&self) -> Value {
        let mut state = self
            .ledger
            .snapshot(field(self.custody, "finalized_tax_projection"));
        let Value::Dict(items) = &mut state else {
            unreachable!()
        };
        items.retain(|(k, _)| !matches!(k, Key::Str(s) if s.is("intra_audit")));
        for (key, value) in [
            ("ownership_review_counts", &self.ownership_review_counts),
            (
                "ownership_review_candidates",
                &self.ownership_review_candidates,
            ),
            ("quantity_differences", &self.quantity_differences),
            (
                "custody_component_blockers",
                field(self.custody, "component_blockers"),
            ),
            ("custody_quantity", field(self.custody, "quantity_state")),
            (
                "custody_transfers",
                field(self.custody, "custody_transfers"),
            ),
            ("latest_rates", field(self.custody, "rates")),
            ("warnings", field(self.custody, "warnings")),
            (
                "custody_gap_ignored_transaction_ids",
                field(self.custody, "gap_ignored_transaction_ids"),
            ),
            (
                "custody_gap_accounting_ignored_transaction_ids",
                field(self.custody, "gap_accounting_ignored_transaction_ids"),
            ),
        ] {
            set(&mut state, key, scalar::snapshot(value));
        }
        fn asdict(value: &Value) -> Value {
            match value {
                Value::Dict(items) => Value::Dict(
                    items
                        .iter()
                        .filter(|(k, _)| !matches!(k, Key::Str(s) if s.is("$type")))
                        .map(|(k, v)| (k.clone(), asdict(v)))
                        .collect(),
                ),
                Value::List(items) => Value::List(items.iter().map(asdict).collect()),
                _ => value.clone(),
            }
        }
        set(
            &mut state,
            "custody_native_reconciliations",
            scalar::snapshot(&asdict(field(
                field(self.custody, "interpretation"),
                "native_component_reconciliations",
            ))),
        );
        state
    }
}

pub fn build<'a>(
    profile: &Value,
    custody: &'a Value,
    response: &Value,
    ids: impl FnMut() -> String,
) -> Result<State<'a>> {
    let ledger = read_ledger(
        profile,
        required(custody, "finalized_tax_projection")?,
        required(custody, "wallet_refs_by_id")?,
        &iterable(required(custody, "direct_payout_records")?)?,
        response,
        ids,
    )?;
    finish(custody, ledger)
}

pub fn finish(custody: &Value, ledger: Ledger) -> Result<State<'_>> {
    let quantity_differences = differences(custody, &ledger.wallet_holdings)?;
    let (ownership_review_counts, ownership_review_candidates) =
        super::ownership::ownership_review(custody, &ledger.quarantines)?;
    Ok(State {
        ledger,
        custody,
        quantity_differences,
        ownership_review_counts,
        ownership_review_candidates,
    })
}

fn items(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => &[],
    }
}

fn int(value: &Value) -> Result<BigInt> {
    let Value::Int(value) = int_zero(value)? else {
        unreachable!()
    };
    Ok(value)
}

fn differences(c: &Value, holdings: &Value) -> Result<Value> {
    let mut current = BTreeMap::new();
    for pair in items(field(holdings, "$pairs")) {
        let pair = items(pair);
        let key = items(&pair[0]);
        let Value::Decimal(quantity) = field(&pair[1], "quantity") else {
            unreachable!()
        };
        let Value::Int(amount) = msat(quantity)? else {
            unreachable!()
        };
        current.insert((text(&key[0]), text(&key[3])), amount);
    }
    let mut reasons = BTreeMap::new();
    for leg in iterable(required(c, "loan_legs")?)? {
        let id = required(&leg, "transaction_id")?;
        if !matches!(id, Value::Null) {
            reasons.insert(
                text(id),
                join(&[
                    "loan_".into(),
                    text(required(&leg, "role")?),
                    "_non_event".into(),
                ]),
            );
        }
    }
    let roles = required(c, "channel_roles")?;
    let Value::Dict(roles) = roles else {
        return Err(error(
            "AttributeError",
            format!("'{}' object has no attribute 'items'", kind(roles)),
        ));
    };
    for (id, role) in roles {
        reasons.insert(
            text(&scalar::key_value(id)),
            join(&["lightning_".into(), text(role), "_non_event".into()]),
        );
    }
    for component in iterable(required(c, "active_components")?)? {
        let empty = dict([]);
        let validation = fallback(get(&component, "validation")?, &empty);
        if int(get(validation, "suspense_msat")?)? <= BigInt::from(0) {
            continue;
        }
        for leg in items(get(&component, "legs")?) {
            if is(get(leg, "role")?, "source") && truth(get(leg, "transaction_id")?) {
                reasons.insert(
                    text(field(leg, "transaction_id")),
                    "custody_component_residual_suspense".into(),
                );
            }
        }
    }
    let projection = required(required(c, "quantity_state")?, "projection")?;
    let observations: HashMap<_, _> = items(required(projection, "observations")?)
        .iter()
        .map(|o| (text(field(o, "quantity_hash")), o))
        .collect();
    let mut canonical = BTreeMap::<_, Value>::new();
    let mut named = BTreeMap::<_, BTreeSet<PyStr>>::new();
    for posting in items(required(projection, "postings")?) {
        if !is(field(posting, "location_kind"), "wallet") {
            continue;
        }
        let key = (
            text(required(posting, "location_id")?),
            text(required(posting, "asset")?),
        );
        let amount = required(posting, "amount_msat")?;
        let total = canonical
            .entry(key.clone())
            .or_insert_with(|| Value::from(0));
        match (&mut *total, amount) {
            (Value::Int(total), Value::Int(amount)) => *total += amount,
            (Value::Int(total), Value::Bool(amount)) => *total += BigInt::from(u8::from(*amount)),
            (Value::Int(_) | Value::Float(_), Value::Float(_) | Value::Int(_) | Value::Bool(_)) => {
                *total = Value::Float(float(total)? + float(amount)?);
            }
            _ => {
                return Err(error(
                    "TypeError",
                    format!(
                        "unsupported operand type(s) for +: '{}' and '{}'",
                        kind(total),
                        kind(amount)
                    ),
                ));
            }
        }
        if let Some(observation) = observations.get(&text(field(posting, "observation_hash"))) {
            if let Some(reason) = reasons.get(&text(field(observation, "transaction_id"))) {
                named.entry(key).or_default().insert(reason.clone());
            }
        }
    }
    let keys: BTreeSet<_> = canonical.keys().chain(current.keys()).cloned().collect();
    let mut result = vec![];
    for key in keys {
        let a = canonical
            .get(&key)
            .cloned()
            .unwrap_or_else(|| Value::from(0));
        let b = current.get(&key).cloned().unwrap_or_default();
        if equal(&a, &Value::Int(b.clone())) {
            continue;
        }
        let reason = named
            .get(&key)
            .filter(|r| r.len() == 1)
            .and_then(|r| r.first())
            .cloned();
        result.push(dict([
            ("$type", Value::from("WalletBalanceDifference")),
            ("wallet_id", Value::Str(key.0)),
            ("asset", Value::Str(key.1)),
            ("canonical_msat", a),
            ("current_msat", Value::Int(b)),
            ("reason", reason.map(Value::Str).unwrap_or(Value::Null)),
        ]));
    }
    Ok(Value::List(result))
}
