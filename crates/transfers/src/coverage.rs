//! `technical_coverage_snapshot`: imported-policy scan diagnostics, redacted to
//! structural classes. Coverage never claims a complete ownership universe and
//! never clears a custody gap.
use super::*;
use kassiber_envelope::{s, set, truthy};

const BRANCHES: [&str; 2] = ["receive", "change"];
const SCRIPT_FAMILIES: [&str; 4] = ["p2pkh", "p2sh-p2wpkh", "p2wpkh", "p2tr"];
const SAMOURAI_SECTIONS: [&str; 5] = ["deposit", "badbank", "premix", "postmix", "ricochet"];

/// `_public_policy_source_name`.
fn source_name(key: &Value) -> String {
    let key = if truthy(key) {
        py_str(key)
    } else {
        String::new()
    };
    let parts: Vec<&str> = key.split(':').collect();
    match parts[0] {
        "descriptor" => "descriptor-policy".into(),
        "xpub" => match parts.get(1) {
            Some(family) if SCRIPT_FAMILIES.contains(family) => format!("extended-key:{family}"),
            _ => "extended-key-policy".into(),
        },
        "samourai" => match parts.get(1) {
            Some(section) if SAMOURAI_SECTIONS.contains(section) => format!("samourai:{section}"),
            _ => "samourai:source".into(),
        },
        _ => "imported-policy".into(),
    }
}
/// `_public_observer_name`.
fn observer_name(kind: &Value) -> &'static str {
    let kind = if truthy(kind) {
        py_str(kind).to_lowercase()
    } else {
        String::new()
    };
    match kind.as_str() {
        "bdk" => "bdk",
        "lwk" => "lwk",
        "bitcoinrpc" => "bitcoinrpc",
        _ if kind.starts_with("compatibility") => "compatibility",
        _ => "observer",
    }
}
fn empty_branch(branch: &str) -> Value {
    dict([
        ("branch", s(branch)),
        ("scanned_to_exclusive", Value::Null),
        ("highest_used", Value::Null),
        ("observed_at", Value::Null),
    ])
}
/// `_declared_coverage_branches`.
fn declared(raw: &Value) -> Vec<Value> {
    let text = if truthy(raw) {
        py_str(raw)
    } else {
        "[]".into()
    };
    let Ok(Value::List(declared)) = kassiber_pytext::loads(&text) else {
        return Vec::new();
    };
    BRANCHES
        .iter()
        .filter(|branch| declared.contains(&s(**branch)))
        .map(|branch| empty_branch(branch))
        .collect()
}
fn list_mut<'a>(row: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    let Value::Dict(pairs) = row else {
        unreachable!()
    };
    match &mut pairs
        .iter_mut()
        .find(|(k, _)| *k == key.into())
        .expect("list field")
        .1
    {
        Value::List(items) => items,
        _ => unreachable!(),
    }
}

pub(crate) fn snapshot(conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    let found = rows(
        conn,
        "SELECT epoch.id AS epoch_id,epoch.wallet_id,wallet.label AS wallet_label,epoch.chain,epoch.network,epoch.status,epoch.created_at AS epoch_created_at,epoch.retired_at,source.id AS source_id,source.source_key,source.observer_kind,source.branch_keys_json,coverage.branch_key,coverage.scanned_to_exclusive,coverage.highest_used,coverage.observed_at FROM wallet_policy_epochs epoch JOIN wallets wallet ON wallet.id=epoch.wallet_id LEFT JOIN wallet_policy_sources source ON source.epoch_id=epoch.id LEFT JOIN wallet_policy_coverage_witnesses coverage ON coverage.source_id=source.id WHERE epoch.profile_id=? ORDER BY wallet.label,epoch.created_at,epoch.id,source.source_key,coverage.branch_key",
        [scope.profile_id],
    )?;
    // Wallets keep first-seen order; epochs and sources are addressed by index.
    let mut wallets: Vec<(String, Value)> = Vec::new();
    let mut epochs: Vec<(String, usize, usize)> = Vec::new();
    let mut sources: Vec<(String, usize, usize, usize)> = Vec::new();
    let mut statuses = Vec::new();
    let mut covered = 0;
    for row in &found {
        let wallet_id = text(row, "wallet_id");
        let w = match wallets.iter().position(|(id, _)| *id == wallet_id) {
            Some(w) => w,
            None => {
                let label = value(row, "wallet_label");
                let label = if truthy(&label) {
                    py_str(&label)
                } else {
                    "Wallet".into()
                };
                wallets.push((
                    wallet_id,
                    dict([("wallet_label", s(label)), ("epochs", Value::List(vec![]))]),
                ));
                wallets.len() - 1
            }
        };
        let epoch_id = text(row, "epoch_id");
        let (ew, e) = match epochs.iter().find(|(id, _, _)| *id == epoch_id) {
            Some((_, ew, e)) => (*ew, *e),
            None => {
                let list = list_mut(&mut wallets[w].1, "epochs");
                list.push(dict([
                    ("epoch_id", s(epoch_id.as_str())),
                    ("status", s(text(row, "status"))),
                    ("chain", s(text(row, "chain"))),
                    ("network", s(text(row, "network"))),
                    ("created_at", value(row, "epoch_created_at")),
                    ("retired_at", value(row, "retired_at")),
                    ("sources", Value::List(vec![])),
                ]));
                statuses.push(text(row, "status"));
                epochs.push((epoch_id, w, list.len() - 1));
                (w, list.len() - 1)
            }
        };
        if value(row, "source_id") == Value::Null {
            continue;
        }
        let source_id = text(row, "source_id");
        let (sw, se, si) = match sources.iter().find(|(id, ..)| *id == source_id) {
            Some((_, sw, se, si)) => (*sw, *se, *si),
            None => {
                let epoch = &mut list_mut(&mut wallets[ew].1, "epochs")[e];
                let list = list_mut(epoch, "sources");
                list.push(dict([
                    ("source", s(source_name(&value(row, "source_key")))),
                    (
                        "observer_kind",
                        s(observer_name(&value(row, "observer_kind"))),
                    ),
                    (
                        "branches",
                        Value::List(declared(&value(row, "branch_keys_json"))),
                    ),
                ]));
                sources.push((source_id, ew, e, list.len() - 1));
                (ew, e, list.len() - 1)
            }
        };
        let branch_key = value(row, "branch_key");
        let Some(key) = BRANCHES.into_iter().find(|branch| branch_key == s(*branch)) else {
            continue;
        };
        let epoch = &mut list_mut(&mut wallets[sw].1, "epochs")[se];
        let source = &mut list_mut(epoch, "sources")[si];
        let branches = list_mut(source, "branches");
        let index = match branches
            .iter()
            .position(|b| get(b, "branch") == Some(&s(key)))
        {
            Some(index) => index,
            None => {
                branches.push(empty_branch(key));
                branches.len() - 1
            }
        };
        if value(row, "scanned_to_exclusive") != Value::Null {
            let branch = &mut branches[index];
            set(
                branch,
                "scanned_to_exclusive",
                value(row, "scanned_to_exclusive"),
            );
            set(branch, "highest_used", value(row, "highest_used"));
            set(branch, "observed_at", value(row, "observed_at"));
            covered += 1;
        }
    }
    let epoch_count = epochs.len();
    let active = statuses.iter().filter(|s| *s == "active").count();
    Ok(dict([
        ("schema_version", Value::from(1)),
        ("scope", s("imported_policy_technical_coverage")),
        ("ownership_universe_known", Value::Bool(false)),
        ("coverage_can_clear_custody_gaps", Value::Bool(false)),
        (
            "summary",
            dict([
                ("wallet_count", Value::from(wallets.len() as i64)),
                ("epoch_count", Value::from(epoch_count as i64)),
                ("active_epoch_count", Value::from(active as i64)),
                (
                    "retired_epoch_count",
                    Value::from((epoch_count - active) as i64),
                ),
                ("source_count", Value::from(sources.len() as i64)),
                ("covered_branch_count", Value::from(covered as i64)),
            ]),
        ),
        (
            "wallets",
            Value::List(wallets.into_iter().map(|(_, w)| w).collect()),
        ),
    ]))
}
