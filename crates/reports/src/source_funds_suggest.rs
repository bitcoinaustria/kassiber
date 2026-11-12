use super::source_funds_projection::{integer, normalized_asset};
use super::source_funds_report as report;
use super::*;
use kassiber_pyjson::{BigInt, PyStr};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

const KEYS: [&str; 7] = [
    "trade_id",
    "order_id",
    "payment_id",
    "provider_trade_id",
    "exchange_order_id",
    "ledger_id",
    "provider_id",
];

fn pytext(v: &Value) -> PyStr {
    if let Value::Str(v) = v {
        v.clone()
    } else {
        py_str(v).into()
    }
}
fn safe_json(v: &Value) -> Value {
    if let Value::Str(v) = v {
        kassiber_pytext::loads_py(v).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}
fn utf8(v: &PyStr) -> Result<String> {
    if let Some(v) = v.as_str() {
        return Ok(v.into());
    }
    let points: Vec<_> = v.code_points().collect();
    let start = points
        .iter()
        .position(|v| (0xd800..=0xdfff).contains(v))
        .unwrap();
    let end = points[start..]
        .iter()
        .take_while(|v| (0xd800..=0xdfff).contains(*v))
        .count()
        + start;
    let message = if end == start + 1 {
        format!("'utf-8' codec can't encode character '\\u{:04x}' in position {start}: surrogates not allowed",points[start])
    } else {
        format!(
            "'utf-8' codec can't encode characters in position {start}-{}: surrogates not allowed",
            end - 1
        )
    };
    Err(AppError::new("internal_error", message))
}
fn bind(v: &Value) -> Result<Sql> {
    Ok(match v {
        Value::Null => Sql::Null,
        Value::Int(n) => Sql::Integer(n.to_string().parse().map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Str(v) => Sql::Text(utf8(v)?),
        Value::Bool(v) => Sql::Integer(i64::from(*v)),
        Value::Float(v) => Sql::Real(*v),
        _ => {
            return Err(AppError::new(
                "internal_error",
                "unsupported suggestion SQL value",
            ))
        }
    })
}
fn write(conn: &Connection, query: &str, values: &[Value]) -> Result<()> {
    let values = values.iter().map(bind).collect::<Result<Vec<_>>>()?;
    conn.execute(query, params_from_iter(values))
        .map_err(sql_error)?;
    Ok(())
}
fn rank(v: &str) -> u8 {
    match v {
        "weak" => 1,
        "strong" => 2,
        "exact" => 3,
        _ => 0,
    }
}
struct Candidate<'a> {
    from: &'a Value,
    to: &'a Value,
    kind: &'a str,
    method: &'a str,
    confidence: &'a str,
    allocation: BigInt,
    from_allocation: BigInt,
    explanation: PyStr,
}
struct Pass<'a> {
    conn: &'a Connection,
    sources: &'a dyn kassiber_setup::Sources,
    workspace: String,
    profile: String,
    target: Option<String>,
    scope: HashSet<String>,
    cap: BigInt,
    inserted: Vec<Value>,
}
impl Pass<'_> {
    fn in_scope(&self, from: &Value, to: &Value) -> bool {
        self.target.is_none()
            || self.scope.contains(&text(from, "id"))
            || self.scope.contains(&text(to, "id"))
    }
    fn insert(&mut self, c: Candidate<'_>, widen: bool) -> Result<()> {
        let from = text(c.from, "id");
        let to = text(c.to, "id");
        let existing=rows(self.conn,"SELECT * FROM source_funds_links WHERE profile_id=? AND from_transaction_id=? AND to_transaction_id=? AND method=? AND link_type=? LIMIT 1",[self.profile.as_str(),&from,&to,c.method,c.kind])?.into_iter().next();
        let stamp;
        let id;
        if let Some(existing) = existing {
            if text(&existing, "state") != "suggested" {
                return Ok(());
            }
            stamp = self.sources.now(self.conn)?;
            id = text(&existing, "id");
            write(self.conn,"UPDATE source_funds_links SET confidence=?,asset=?,allocation_amount=?,from_asset=?,from_allocation_amount=?,allocation_policy='heuristic',explanation=?,uses_chain_observation=0,chain_data_confirmed=1,updated_at=? WHERE id=?",&[
                s(c.confidence),value(c.to,"asset").clone(),Value::Int(c.allocation),value(c.from,"asset").clone(),Value::Int(c.from_allocation),s(c.explanation),s(stamp),s(id.as_str()),
            ])?;
        } else {
            let pairs=rows(self.conn,"SELECT id,state,confidence FROM source_funds_links WHERE profile_id=? AND from_transaction_id=? AND to_transaction_id=? AND state!='rejected'",[self.profile.as_str(),&from,&to])?;
            if pairs.iter().any(|r| text(r, "state") != "suggested")
                || pairs
                    .iter()
                    .map(|r| rank(&text(r, "confidence")))
                    .max()
                    .is_some_and(|r| rank(c.confidence) <= r)
            {
                return Ok(());
            }
            for pair in pairs {
                write(
                    self.conn,
                    "DELETE FROM source_funds_links WHERE id=?",
                    &[value(&pair, "id").clone()],
                )?;
            }
            stamp = self.sources.now(self.conn)?;
            id = self.sources.uuid(self.conn)?;
            write(self.conn,"INSERT INTO source_funds_links(id,workspace_id,profile_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at) VALUES(?,?,?,?,?,?,'suggested',?,?,?,?,?,?,'heuristic',?,0,1,?,?)",&[
                s(id.as_str()),s(self.workspace.as_str()),s(self.profile.as_str()),s(from.as_str()),s(to.as_str()),s(c.kind),s(c.confidence),s(c.method),value(c.to,"asset").clone(),Value::Int(c.allocation),value(c.from,"asset").clone(),Value::Int(c.from_allocation),s(c.explanation),s(stamp.as_str()),s(stamp),
            ])?;
        }
        let row = rows(
            self.conn,
            "SELECT * FROM source_funds_links WHERE id=?",
            [&id],
        )?
        .remove(0);
        self.inserted.push(row);
        if BigInt::from(self.inserted.len()) > self.cap {
            return Err(AppError::new("validation","source-funds suggestion write cap exceeded").with_hint("Run suggestions for a narrower --target-transaction, review or reject existing suggestions, then try again.").with_details(dict([("max_suggestions",Value::Int(self.cap.clone()))])));
        }
        if self.target.is_some() && widen {
            self.scope.extend([from, to]);
        }
        Ok(())
    }
}
fn samourai(row: &Value) -> Option<(PyStr, String)> {
    let raw = safe_json(value(row, "wallet_config_json"));
    let meta = value(&raw, "samourai");
    if text(meta, "role") != "child" {
        return None;
    }
    let group = kassiber_pytext::strip_py(&pytext(if truthy(value(meta, "group_id")) {
        value(meta, "group_id")
    } else {
        &Value::Null
    }));
    let section = kassiber_pytext::strip_py(&pytext(if truthy(value(meta, "section")) {
        value(meta, "section")
    } else {
        &Value::Null
    }));
    if !truthy(value(meta, "group_id"))
        || !truthy(value(meta, "section"))
        || group.code_points().next().is_none()
        || section.code_points().next().is_none()
    {
        return None;
    }
    Some((
        group,
        section
            .as_str()
            .map(kassiber_pytext::lower)
            .unwrap_or_default(),
    ))
}
fn candidate<'a>(
    from: &'a Value,
    to: &'a Value,
    kind: &'a str,
    method: &'a str,
    confidence: &'a str,
    explanation: impl Into<PyStr>,
) -> Result<Candidate<'a>> {
    Ok(Candidate {
        from,
        to,
        kind,
        method,
        confidence,
        allocation: integer(value(to, "amount"))?,
        from_allocation: integer(value(from, "amount"))?,
        explanation: explanation.into(),
    })
}
pub(super) fn scope(conn: &Connection, profile: &str, target: &str) -> Result<HashSet<String>> {
    let mut found = HashSet::from([target.into()]);
    let mut queue = VecDeque::from([target.to_owned()]);
    while let Some(id) = queue.pop_front() {
        for row in rows(conn,"SELECT from_transaction_id FROM source_funds_links WHERE profile_id=? AND to_transaction_id=? AND state!='rejected'",[profile,&id])? {
            if truthy(value(&row,"from_transaction_id")) {let id=text(&row,"from_transaction_id");if found.insert(id.clone()) {queue.push_back(id);}}
        }
    }
    Ok(found)
}

pub fn handles(command: &str) -> bool {
    matches!(command, "source-funds.suggest" | "ui.source_funds.suggest")
}

pub fn execute(
    conn: &Connection,
    command: &str,
    raw: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let args = arguments(command, raw)?;
    if !command.starts_with("ui.") && truthy(value(&args, "dry_run")) {
        kassiber_store::begin_write_savepoint(conn, "source_funds_preview").map_err(sql_error)?;
        let result = run(conn, &args, sources, None);
        conn.execute_batch("ROLLBACK TO source_funds_preview; RELEASE source_funds_preview")
            .map_err(sql_error)?;
        return result.map(|mut result| {
            replace(&mut result, "dry_run", Value::Bool(true));
            result
        });
    }
    committed(conn, || run(conn, &args, sources, None)).map(|mut result| {
        replace(&mut result, "committed", Value::Bool(true));
        result
    })
}

pub(super) fn arguments(command: &str, raw: &Value) -> Result<Value> {
    if !command.starts_with("ui.") {
        return Ok(raw.clone());
    }
    let empty = dict::<&str>([]);
    let raw = if matches!(raw, Value::Null) {
        &empty
    } else {
        raw
    };
    if !matches!(raw, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(raw)))])));
    }
    let mut args = dict::<&str>([]);
    let target = value(raw, "target_transaction");
    replace(
        &mut args,
        "target_transaction",
        if let Value::Str(v) = target {
            Value::Str(kassiber_pytext::strip_py(v))
        } else {
            Value::Null
        },
    );
    let broad = get(raw, "include_broad_hints").unwrap_or(&Value::Bool(false));
    if !matches!(broad, Value::Bool(_)) {
        return Err(
            AppError::new("validation", "include_broad_hints must be a boolean")
                .with_details(dict([("type", s(kassiber_envelope::type_name(broad)))])),
        );
    }
    replace(&mut args, "include_broad_hints", Value::Bool(truthy(broad)));
    let cap = get(raw, "max_suggestions")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(Value::Int(500.into()));
    replace(&mut args, "max_suggestions", Value::Int(integer(&cap)?));
    Ok(args)
}

pub(super) fn committed<T>(conn: &Connection, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let result = crate::export::admitted(conn, operation)?;
    if !conn.is_autocommit() {
        if let Err(error) = conn.execute_batch("COMMIT") {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(sql_error(error));
        }
    }
    Ok(result)
}

pub(super) struct Inputs<'a> {
    pub report: report::Inputs<'a>,
    pub structural: Vec<kassiber_custody::source_funds::StructuralPair>,
}

pub(super) fn load_inputs<'a>(conn: &'a Connection, profile: &Value) -> Result<Inputs<'a>> {
    let report = report::load_inputs(conn, profile)?;
    let owned = rows(
        conn,
        "SELECT * FROM wallet_utxos WHERE profile_id=?",
        [text(profile, "id")],
    )?;
    let structural = kassiber_custody::source_funds::structural_pair_details(
        &report
            .active
            .iter()
            .map(kassiber_custody::codec::to_json)
            .collect::<Vec<_>>(),
        &owned
            .iter()
            .map(kassiber_custody::codec::to_json)
            .collect::<Vec<_>>(),
    );
    Ok(Inputs { report, structural })
}

pub(super) fn run(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
    frozen: Option<&Inputs<'_>>,
) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let profile_id = text(&profile, "id");
    let target = if truthy(value(args, "target_transaction")) {
        Some(text(
            &report::resolve_transaction(conn, &profile_id, &text(args, "target_transaction"))?,
            "id",
        ))
    } else {
        None
    };
    let cap = if get(args, "max_suggestions").is_none() {
        500.into()
    } else {
        integer(value(args, "max_suggestions"))?
    };
    if cap <= 0.into() {
        return Err(AppError::new(
            "validation",
            "--max-suggestions must be positive",
        ));
    }
    let cap = cap.min(500.into());
    let active = if let Some(inputs) = frozen {
        inputs.report.active.clone()
    } else {
        report::active_rows(conn, &profile_id)?
    };
    let scope = if let Some(id) = &target {
        scope(conn, &profile_id, id)?
    } else {
        active.iter().map(|r| text(r, "id")).collect()
    };
    let mut pass = Pass {
        conn,
        sources,
        workspace: text(&workspace, "id"),
        profile: profile_id.clone(),
        target: target.clone(),
        scope,
        cap,
        inserted: Vec::new(),
    };
    type SamouraiKey = (PyStr, String, String, String, String);
    let mut groups: Vec<(SamouraiKey, Vec<(usize, String)>)> = Vec::new();
    let mut group_positions = HashMap::new();
    for (i, row) in active.iter().enumerate() {
        if let (Some((group, section)), Some((chain, network, txid, asset))) = (
            samourai(row),
            kassiber_custody::source_funds::scope(&kassiber_custody::codec::to_json(row)),
        ) {
            let key = (group, chain, network, txid, asset);
            let position = *group_positions.entry(key.clone()).or_insert_with(|| {
                groups.push((key, Vec::new()));
                groups.len() - 1
            });
            groups[position].1.push((i, section));
        }
    }
    for (_, group) in groups {
        let outs: Vec<_> = group
            .iter()
            .filter(|(i, _)| text(&active[*i], "direction") == "outbound")
            .collect();
        let ins: Vec<_> = group
            .iter()
            .filter(|(i, _)| text(&active[*i], "direction") == "inbound")
            .collect();
        let boundary = outs.iter().any(|(_, section)| match section.as_str() {
            "deposit" => ins
                .iter()
                .any(|(_, s)| ["premix", "badbank"].contains(&s.as_str())),
            "premix" | "postmix" => ins.iter().any(|(_, s)| s == "postmix"),
            _ => false,
        });
        if !boundary {
            continue;
        }
        for (o, _) in &outs {
            for (i, section) in &ins {
                let from = &active[*o];
                let to = &active[*i];
                if section == "badbank"
                    || text(from, "id") == text(to, "id")
                    || !pass.in_scope(from, to)
                {
                    continue;
                }
                pass.insert(candidate(from,to,"coinjoin","coinjoin","unknown","Coinjoin section transition in the same imported wallet group; review as a privacy boundary, not exact participant lineage.")?,true)?;
            }
        }
    }
    let loaded;
    let inputs = if let Some(inputs) = frozen {
        &inputs.report
    } else {
        loaded = report::load_inputs_with_active(conn, &profile, active)?;
        &loaded
    };
    let active = &inputs.active;
    let rows_by_id: BTreeMap<_, _> = active.iter().map(|r| (text(r, "id"), r)).collect();
    let reviewed_targets: HashSet<_> = inputs
        .allocation_details
        .iter()
        .filter(|(_, (_, reviewed))| *reviewed)
        .map(|(k, _)| k.1.as_str())
        .collect();
    let mut allocations: Vec<_> = inputs.allocations.iter().collect();
    allocations.sort_by_key(|(k, _)| (&k.1, &k.0, &k.2));
    for (key, (amount, from_amount)) in allocations {
        let (projection_ids, reviewed) = &inputs.allocation_details[key];
        if reviewed_targets.contains(key.1.as_str()) && !reviewed {
            continue;
        }
        let from = rows_by_id[&key.0];
        let to = rows_by_id[&key.1];
        if !pass.in_scope(from, to) {
            continue;
        }
        let c = Candidate {
            from,
            to,
            kind: &key.2,
            method: "custody_component",
            confidence: "exact",
            allocation: amount.clone(),
            from_allocation: from_amount.clone(),
            explanation: format!(
                "Current stored custody projection links these transaction rows ({}).",
                projection_ids
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into(),
        };
        pass.insert(c, true)?;
    }
    if target.is_some() {
        let loaded;
        let pairs = if let Some(inputs) = frozen {
            &inputs.structural
        } else {
            let owned = rows(
                conn,
                "SELECT * FROM wallet_utxos WHERE profile_id=?",
                [&profile_id],
            )?;
            loaded = kassiber_custody::source_funds::structural_pair_details(
                &active
                    .iter()
                    .map(kassiber_custody::codec::to_json)
                    .collect::<Vec<_>>(),
                &owned
                    .iter()
                    .map(kassiber_custody::codec::to_json)
                    .collect::<Vec<_>>(),
            );
            &loaded
        };
        for pair in pairs {
            if inputs
                .allocations
                .keys()
                .any(|k| k.0 == pair.from && k.1 == pair.to)
                || !pass.scope.contains(pair.to.as_str())
                || pair.allocation <= 0
            {
                continue;
            }
            let explanation = if pair.passthrough {
                "This spend's observed inputs trace back to this transaction through intermediate transactions in the same wallet. Value is attributed across those hops in proportion to the amounts they carried, which conserves the total but does not identify which individual coins moved."
            } else {
                "This spend's observed inputs include outputs created by this transaction in the same wallet."
            };
            let mut c = candidate(
                rows_by_id[&pair.from],
                rows_by_id[&pair.to],
                "self_transfer",
                "utxo_spend",
                "exact",
                explanation,
            )?;
            c.allocation = pair.allocation.into();
            c.from_allocation = pair.from_allocation.into();
            pass.insert(c, false)?;
        }
    }
    let broad = truthy(value(args, "include_broad_hints"));
    let mut providers: Vec<((&str, PyStr), Vec<usize>)> = Vec::new();
    let mut provider_positions = HashMap::new();
    for (i, row) in active.iter().enumerate() {
        let raw = safe_json(value(row, "raw_json"));
        if !matches!(raw, Value::Dict(_)) {
            continue;
        }
        for key in KEYS {
            let v = value(&raw, key);
            if matches!(v, Value::Null) || matches!(v,Value::Str(s) if s.is("")) {
                continue;
            }
            let key = (key, pytext(v));
            let position = *provider_positions.entry(key.clone()).or_insert_with(|| {
                providers.push((key, Vec::new()));
                providers.len() - 1
            });
            providers[position].1.push(i);
        }
    }
    for ((key, v), group) in providers {
        let unique = key != "provider_id";
        if !unique && !broad {
            continue;
        }
        let outs: Vec<_> = group
            .iter()
            .map(|i| &active[*i])
            .filter(|r| text(r, "direction") == "outbound")
            .collect();
        let ins: Vec<_> = group
            .iter()
            .map(|i| &active[*i])
            .filter(|r| text(r, "direction") == "inbound")
            .collect();
        let method = if key.starts_with("provider_") {
            key.to_owned()
        } else {
            format!("provider_{key}")
        };
        for from in &outs {
            for to in &ins {
                if text(from, "id") == text(to, "id") || !pass.in_scope(from, to) {
                    continue;
                }
                let a = integer(value(from, "amount"))?;
                let b = integer(value(to, "amount"))?;
                let abs = |n: BigInt| if n < 0.into() { -n } else { n };
                let close = normalized_asset(value(from, "asset"))
                    == normalized_asset(value(to, "asset"))
                    && abs(abs(a.clone()) - abs(b.clone()))
                        <= (abs(a).max(abs(b)) / BigInt::from(100)).max(1000.into());
                let confidence = if unique && outs.len() == 1 && ins.len() == 1 && close {
                    "strong"
                } else {
                    "weak"
                };
                let prefix = format!("Both imports carry {key}=");
                let suffix = if confidence == "strong" {
                    ". One-to-one amount match."
                } else {
                    ". Manual review required."
                };
                let explanation = PyStr::from_code_points(
                    prefix
                        .chars()
                        .map(u32::from)
                        .chain(v.code_points())
                        .chain(suffix.chars().map(u32::from)),
                )
                .unwrap();
                pass.insert(
                    candidate(
                        from,
                        to,
                        if value(from, "asset") == value(to, "asset") {
                            "trade"
                        } else {
                            "swap"
                        },
                        &method,
                        confidence,
                        explanation,
                    )?,
                    true,
                )?;
            }
        }
    }
    if broad {
        for from in active.iter().filter(|r| text(r, "direction") == "outbound") {
            for to in active.iter().filter(|r| {
                text(r, "direction") == "inbound" && value(r, "asset") == value(from, "asset")
            }) {
                if value(from, "wallet_id") == value(to, "wallet_id")
                    || value(from, "id") == value(to, "id")
                    || !pass.in_scope(from, to)
                    || py_str(value(from, "occurred_at"))
                        .chars()
                        .take(10)
                        .ne(py_str(value(to, "occurred_at")).chars().take(10))
                {
                    continue;
                }
                let difference = integer(value(from, "amount"))? - integer(value(to, "amount"))?;
                let difference = if difference < 0.into() {
                    -difference
                } else {
                    difference
                };
                if difference > (integer(value(to, "amount"))? / BigInt::from(100)).max(1000.into())
                {
                    continue;
                }
                pass.insert(candidate(from,to,"self_transfer","tight_time_amount_match","weak","Same-day same-asset amount match across owned wallets; review before using as evidence.")?,true)?;
            }
        }
    }
    let links = pass
        .inserted
        .iter()
        .map(|r| super::source_funds_reads::link(conn, r))
        .collect::<Result<Vec<_>>>()?;
    Ok(dict([("inserted",Value::Int(links.len().into())),("committed",Value::Bool(false)),("target_transaction_id",target.map(s).unwrap_or(Value::Null)),("links",Value::List(links)),("privacy_warning",s("No chain backend was queried. If public Esplora/Electrum observations are added later, the queried txids reveal the target path to that backend."))]))
}
