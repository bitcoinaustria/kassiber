use super::source_funds_projection::{amount, integer};
use super::source_funds_report as report;
use super::*;
use kassiber_pyjson::BigInt;
use std::collections::BTreeMap;

const BUCKETS: [&str; 6] = [
    "fully_traced",
    "attested",
    "unattributed",
    "in_review",
    "untraced",
    "not_classified",
];
#[derive(Clone, Default)]
struct Bucket {
    amount: BigInt,
    count: usize,
}
type Breakdown = [Bucket; 6];

fn empty() -> Breakdown {
    std::array::from_fn(|_| Bucket::default())
}
fn materialize(buckets: &Breakdown) -> Result<Value> {
    Ok(dict(
        BUCKETS
            .iter()
            .zip(buckets)
            .map(|(name, b)| {
                Ok((
                    *name,
                    dict([
                        ("amount_msat", Value::Int(b.amount.clone())),
                        ("amount", amount(&b.amount)?),
                        ("tx_count", Value::Int(b.count.into())),
                    ]),
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    ))
}
fn add(buckets: &mut Breakdown, index: usize, quantity: &BigInt) {
    buckets[index].amount += quantity;
    buckets[index].count += 1;
}
fn total(buckets: &Breakdown) -> BigInt {
    buckets.iter().map(|b| b.amount.clone()).sum()
}
fn classify(report: &Value) -> usize {
    if !truthy(value(value(report, "explain_gates"), "exportable")) {
        return 3;
    }
    let mix = match value(report, "source_mix") {
        Value::List(v) => v.as_slice(),
        _ => &[],
    };
    let mut real = false;
    let mut attested = false;
    for row in mix {
        match text(row, "source_type").as_str() {
            "unknown" => return 2,
            "missing_history" | "opening_balance_attestation" => attested = true,
            _ => real = true,
        }
    }
    if attested {
        1
    } else if real {
        0
    } else {
        3
    }
}

pub fn execute(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let empty_args = dict::<&str>([]);
    let args = if desktop && matches!(args, Value::Null) {
        &empty_args
    } else {
        args
    };
    if desktop && !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])));
    }
    let raw_depth = value(args, "max_depth");
    let raw_count = value(args, "max_transactions");
    let (depth, cap) = if desktop {
        let positive = |v: &Value, default: i64, cap: i64| -> BigInt {
            match v {
                Value::Int(n) if *n > 0.into() => n.clone().min(cap.into()),
                Value::Bool(true) => 1.into(),
                _ => default.into(),
            }
        };
        (
            positive(raw_depth, 8, 32),
            positive(raw_count, 2000, 50_000),
        )
    } else {
        (
            if get(args, "max_depth").is_none() {
                8.into()
            } else {
                integer(raw_depth)?
            },
            if get(args, "max_transactions").is_none() {
                2000.into()
            } else {
                integer(raw_count)?
            },
        )
    };
    let (workspace, profile) =
        kassiber_setup::references::scope(conn, if desktop { &empty_args } else { args })?;
    let returned_depth = if desktop && matches!(raw_depth, Value::Bool(true)) {
        Value::Bool(true)
    } else {
        Value::Int(depth.clone())
    };
    let profile_id = text(&profile, "id");
    let inbound=rows(conn,"SELECT t.id,t.wallet_id,t.asset,t.amount,w.label AS wallet_label FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.direction='inbound' AND t.excluded=0 ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC",[&profile_id])?;
    let mut wallets: BTreeMap<(String, String, String), Breakdown> = BTreeMap::new();
    let mut assets: BTreeMap<String, Breakdown> = BTreeMap::new();
    let mut totals = empty();
    let mut shared = None;
    for (index, row) in inbound.iter().enumerate() {
        let quantity = integer(value(row, "amount"))?;
        let bucket = if BigInt::from(index) >= cap {
            5
        } else if rows(conn, "SELECT 1 FROM source_funds_links WHERE profile_id=? AND to_transaction_id=? AND state!='rejected' LIMIT 1", [&profile_id, &text(row,"id")])?.is_empty() {
            4
        } else {
            (|| {
                if shared.is_none() {
                    shared = Some(report::load_inputs(conn, &profile)?);
                }
                report::build(conn, &workspace, &profile, &dict([
                    ("target_transaction", value(row,"id").clone()),
                    ("reveal_mode", s("standard")),
                    ("max_depth", Value::Int(depth.clone())),
                ]), shared.as_ref()).map(|v| classify(&v))
            })().or_else(|error: AppError| {
                if error.code != "internal_error" || error.sqlite_origin {
                    Ok(3)
                } else {
                    Err(error)
                }
            })?
        };
        add(
            wallets
                .entry((
                    text(row, "wallet_label"),
                    text(row, "asset"),
                    text(row, "wallet_id"),
                ))
                .or_insert_with(empty),
            bucket,
            &quantity,
        );
        add(
            assets.entry(text(row, "asset")).or_insert_with(empty),
            bucket,
            &quantity,
        );
        add(&mut totals, bucket, &quantity);
    }
    let wallets = wallets
        .into_iter()
        .map(|((label, asset, id), b)| {
            let total = total(&b);
            Ok(dict([
                ("wallet_id", s(id)),
                ("wallet_label", s(label)),
                ("asset", s(asset)),
                ("buckets", materialize(&b)?),
                ("total_inbound_msat", Value::Int(total.clone())),
                ("total_inbound", amount(&total)?),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let assets = assets
        .into_iter()
        .map(|(asset, b)| {
            let total = total(&b);
            Ok(dict([
                ("asset", s(asset)),
                ("buckets", materialize(&b)?),
                ("total_inbound_msat", Value::Int(total.clone())),
                ("total_inbound", amount(&total)?),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let total = total(&totals);
    let missing = totals[5].amount.clone();
    let missing_count = totals[5].count;
    let result=dict([
        ("scope",s("historical_inbound")),
        ("scope_note",s("Counts every inbound transaction. Not a current-holdings view: self-transfers between owned wallets are counted at each hop, and already-spent inflows still appear in the totals.")),
        ("by_wallet",Value::List(wallets)),("by_asset",Value::List(assets)),
        ("totals",dict([("buckets",materialize(&totals)?),("tx_count",Value::Int(inbound.len().into())),("amount_msat",Value::Int(total.clone())),("amount",amount(&total)?)])),
        ("limits",dict([("max_depth",returned_depth),("max_transactions",Value::Int(cap))])),
        ("truncation",dict([("truncated",Value::Bool(missing_count>0)),("inbound_total_count",Value::Int(inbound.len().into())),("inbound_total_msat",Value::Int(total.clone())),("inbound_total",amount(&total)?),("not_classified_count",Value::Int(missing_count.into())),("not_classified_msat",Value::Int(missing.clone())),("not_classified",amount(&missing)?)])),
    ]);
    if !desktop && matches!(text(args, "format").as_str(), "table" | "plain") {
        summary(&result).map(s)
    } else {
        Ok(result)
    }
}
fn summary(coverage: &Value) -> Result<String> {
    let totals = value(coverage, "totals");
    let truncation = value(coverage, "truncation");
    let count = integer(value(totals, "tx_count"))?;
    let mut lines = Vec::new();
    if truthy(value(truncation, "truncated")) {
        lines.push(format!(
            "Coverage truncated: classified {count} of {} inbound tx; {} not classified.",
            py_str(value(truncation, "inbound_total_count")),
            py_str(value(truncation, "not_classified_count"))
        ));
    }
    lines.push(format!("Inbound transactions: {count}"));
    let mut append = |buckets: &Value, prefix: &str| {
        for name in BUCKETS {
            let bucket = value(buckets, name);
            let quantity = if let Value::Float(v) = value(bucket, "amount") {
                *v
            } else {
                0.0
            };
            lines.push(format!(
                "{prefix}{name}: {quantity:.8} ({} tx)",
                py_str(value(bucket, "tx_count"))
            ));
        }
    };
    append(value(totals, "buckets"), "  ");
    if let Value::List(assets) = value(coverage, "by_asset") {
        if !assets.is_empty() {
            lines.push(String::new());
            lines.push("By asset:".into());
            for row in assets {
                lines.push(format!("  {}:", text(row, "asset")));
                for name in BUCKETS {
                    let bucket = value(value(row, "buckets"), name);
                    let quantity = if let Value::Float(v) = value(bucket, "amount") {
                        *v
                    } else {
                        0.0
                    };
                    lines.push(format!(
                        "    {name}: {quantity:.8} ({} tx)",
                        py_str(value(bucket, "tx_count"))
                    ));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}
