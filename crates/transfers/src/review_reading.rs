use super::*;
use kassiber_envelope::{s, set, truthy};
use kassiber_pyjson::{dumps, Options};

fn int_or_zero(value: &Value) -> Result<kassiber_pyjson::BigInt> {
    integer(if truthy(value) {
        value
    } else {
        &Value::Bool(false)
    })
}

fn json(value: &Value, compact: bool) -> Result<String> {
    dumps(
        value,
        &Options {
            sort_keys: true,
            separators: compact.then(|| (",".into(), ":".into())),
            ..Options::default()
        },
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// Bind the review, observed legs and allocations by economic content.
pub fn pair_fingerprint(conn: &Connection, pair: &Value) -> Result<String> {
    let mut reading = project(
        pair,
        &[
            "kind",
            "policy",
            "out_amount",
            "swap_fee_msat",
            "swap_fee_kind",
        ],
    );
    set(&mut reading, "pair_id", s(text(pair, "id")));
    for side in ["out", "in"] {
        for field in [
            "transaction_id",
            "asset",
            "full_amount_msat",
            "amount_msat",
            "external_id",
            "occurred_at",
        ] {
            let key = format!("{side}_{field}");
            set(&mut reading, &key, value(pair, &key));
        }
    }
    let mut allocations = Vec::new();
    if truthy(&value(pair, "component_id")) {
        for row in rows(
            conn,
            "SELECT source.role AS source_role,
                    COALESCE(source.anchor_transaction_id,source.transaction_id) AS source_tx,
                    sink.role AS sink_role,
                    COALESCE(sink.anchor_transaction_id,sink.transaction_id) AS sink_tx,
                    allocation.source_amount_msat,allocation.sink_amount_msat
             FROM custody_component_allocations allocation
             JOIN custody_component_legs source ON source.id=allocation.source_leg_id
             JOIN custody_component_legs sink ON sink.id=allocation.sink_leg_id
             WHERE allocation.component_id=?",
            [text(pair, "component_id")],
        )? {
            allocations.push(Value::List(vec![
                value(&row, "source_role"),
                value(&row, "source_tx"),
                value(&row, "sink_role"),
                value(&row, "sink_tx"),
                Value::Int(int_or_zero(&value(&row, "source_amount_msat"))?),
                Value::Int(int_or_zero(&value(&row, "sink_amount_msat"))?),
            ]));
        }
    }
    // Python sorts by json.dumps(item), which uses spaces, not compact JSON.
    let mut keyed = allocations
        .into_iter()
        .map(|row| Ok((json(&row, false)?, row)))
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    set(
        &mut reading,
        "allocations",
        Value::List(keyed.into_iter().map(|(_, row)| row).collect()),
    );
    component_store::hash(&reading, true)
}

pub fn reading(conn: &Connection, scope: &Scope<'_>, out_id: &str, in_id: &str) -> Result<Value> {
    let Value::List(pairs) = records::pairs(conn, scope)? else {
        unreachable!()
    };
    let mut matches = pairs.into_iter().filter(|pair| {
        text(pair, "out_transaction_id") == out_id && text(pair, "in_transaction_id") == in_id
    });
    let first = matches.next();
    if first.is_none() || matches.next().is_some() {
        return Ok(dict([
            ("reviewPairId", Value::Null),
            ("pairFingerprint", Value::Null),
        ]));
    }
    let mut pair = first.unwrap();
    for side in ["out", "in"] {
        let leg = value(&pair, side);
        for field in [
            "transaction_id",
            "asset",
            "full_amount_msat",
            "amount_msat",
            "external_id",
            "occurred_at",
        ] {
            set(&mut pair, &format!("{side}_{field}"), value(&leg, field));
        }
    }
    for side in ["out", "in"] {
        let observed = rows(
            conn,
            "SELECT external_id FROM transactions WHERE id=?",
            [text(&pair, &format!("{side}_transaction_id"))],
        )?;
        set(
            &mut pair,
            &format!("{side}_external_id"),
            observed
                .first()
                .map(|row| value(row, "external_id"))
                .unwrap_or(Value::Null),
        );
    }
    Ok(dict([
        ("reviewPairId", value(&pair, "id")),
        ("pairFingerprint", s(pair_fingerprint(conn, &pair)?)),
    ]))
}
