use super::*;

fn next_revision(revision: &kassiber_pyjson::BigInt) -> Result<i64> {
    i64::try_from(revision.clone())
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })
}

fn subject(value: &str, chain: &str, network: &str) -> Result<String> {
    let normalized = match (chain, network) {
        ("liquid", "main") => "liquidv1",
        ("liquid", "test") => "liquidtestnet",
        ("liquid", "regtest") => "elementsregtest",
        _ => network,
    };
    let prefix = format!("{chain}:{normalized}:");
    let physical = value.strip_prefix(&prefix).unwrap_or(value);
    let physical = physical
        .strip_prefix("tx:")
        .or_else(|| physical.strip_prefix("out:"))
        .unwrap_or(physical);
    let (txid, index) = physical
        .split_once(':')
        .map_or((physical, None), |(a, b)| (a, Some(b)));
    let outpoint = index.is_none_or(|v| {
        v == "0"
            || (!v.starts_with('0')
                && (1..=10).contains(&v.len())
                && v.bytes().all(|b| b.is_ascii_digit()))
    });
    if txid.len() == 64 && txid.bytes().all(|b| b.is_ascii_hexdigit()) && outpoint {
        return Ok(lower(value));
    }
    let invalid = || {
        super::invalid(
            "Subject must be a transaction, outpoint or valid address in the selected domain",
        )
    };
    if chain == "bitcoin" {
        let address = value
            .parse::<bitcoin::Address<_>>()
            .map_err(|_| invalid())?;
        let bitcoin_network = match network {
            "main" => bitcoin::Network::Bitcoin,
            "regtest" => bitcoin::Network::Regtest,
            "signet" => bitcoin::Network::Signet,
            _ => bitcoin::Network::Testnet,
        };
        address
            .require_network(bitcoin_network)
            .map_err(|_| invalid())?;
        if ["bc1", "tb1", "bcrt1"]
            .iter()
            .any(|p| lower(value).starts_with(p))
        {
            return Ok(lower(value));
        }
    } else {
        let hrp = lower(value)
            .rsplit_once('1')
            .map(|(hrp, _)| hrp.to_owned())
            .unwrap_or_default();
        if ["ex", "lq", "tex", "tlq", "ert", "el", "bc", "tb", "bcrt"].contains(&hrp.as_str()) {
            value
                .parse::<elements_miniscript::elements::Address>()
                .map_err(|_| invalid())?;
        } else {
            let data = bitcoin::base58::decode_check(value).map_err(|_| invalid())?;
            if data.len() >= 2 && [[12, 39], [4, 75], [23, 19]].contains(&[data[0], data[1]]) {
                bitcoin::secp256k1::PublicKey::from_slice(data.get(2..35).ok_or_else(invalid)?)
                    .map_err(|_| invalid())?;
            } else if !data
                .first()
                .is_some_and(|b| [39, 75, 19, 5, 196].contains(b))
            {
                return Err(AppError::new("internal_error", "Invalid address"));
            }
        }
        let prefixes: &[&str] = match network {
            "main" => &["ex1", "lq1", "V", "G", "H"],
            "test" => &["tex1", "tlq1", "Q", "8", "v"],
            _ => &["ert1", "el1", "2", "X", "Az"],
        };
        if !prefixes.iter().any(|p| value.starts_with(p)) {
            return Err(invalid());
        }
    }
    Ok(value.to_owned())
}

fn label(mut row: Value) -> Value {
    let cluster = truthy(field(&row, "cluster_defining"));
    let deleted = truthy(field(&row, "deleted"));
    set(&mut row, "cluster_defining", Value::Bool(cluster));
    set(&mut row, "deleted", Value::Bool(deleted));
    remove(&mut row, "profile_id");
    row
}

fn load(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    first(
        conn,
        "SELECT * FROM chain_analysis_labels WHERE id=? AND profile_id=? AND deleted=0",
        params![sql(id, 1)?, profile],
    )?
    .ok_or_else(|| AppError::new("not_found", "Label not found in this book"))
}

fn current(conn: &Connection, id: &str) -> Result<Value> {
    Ok(label(
        first(conn, "SELECT * FROM chain_analysis_labels WHERE id=?", [id])?.unwrap(),
    ))
}

fn history(conn: &Connection, id: &str, revision: i64, timestamp: &str) -> Result<Value> {
    let result = current(conn, id)?;
    conn.execute(
        "INSERT INTO chain_analysis_label_history VALUES(?,?,?,?)",
        params![id, revision, canonical(&result)?, timestamp],
    )
    .map_err(database_error)?;
    Ok(result)
}

pub(super) fn upsert(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    const FIELDS: &[&str] = &[
        "subject",
        "chain",
        "network",
        "label",
        "category",
        "source",
        "confidence",
        "cluster_defining",
        "id",
        "expected_revision",
    ];
    arguments(args, FIELDS, &FIELDS[..7])?;
    let chain = field(args, "chain").as_str().unwrap_or_default();
    let network = field(args, "network").as_str().unwrap_or_default();
    if !["bitcoin", "liquid"].contains(&chain)
        || !["main", "test", "signet", "regtest"].contains(&network)
        || (chain == "liquid" && network == "signet")
    {
        return Err(invalid("A valid explicit chain and network are required"));
    }
    let mut value = dict(
        FIELDS[..7]
            .iter()
            .map(|key| {
                Ok((
                    *key,
                    Value::Str(text_value(
                        field(args, key),
                        key,
                        if *key == "source" { 512 } else { 256 },
                    )?),
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    let canonical_subject = subject(
        field(&value, "subject").as_str().unwrap_or_default(),
        chain,
        network,
    )?;
    set(&mut value, "subject", s(canonical_subject));
    if !["exchange", "merchant", "mixer", "service", "self", "other"]
        .contains(&field(&value, "category").as_str().unwrap_or_default())
        || !["user_confirmed", "imported", "unverified"]
            .contains(&field(&value, "confidence").as_str().unwrap_or_default())
    {
        return Err(invalid("Unsupported label category or confidence"));
    }
    let cluster = match get(args, "cluster_defining") {
        None => false,
        Some(Value::Bool(b)) => *b,
        _ => return Err(invalid("cluster_defining must be a boolean")),
    };
    atomic(conn, || {
        let timestamp = sources.now();
        let (id, revision, created) = if truthy(field(args, "id")) {
            let old = load(conn, profile, field(args, "id"))?;
            let Value::Int(revision) = field(&old, "revision") else {
                return Err(stale("Label changed; reload before editing"));
            };
            if !matches!(get(args, "expected_revision"), Some(Value::Int(n)) if n == revision) {
                return Err(stale("Label changed; reload before editing"));
            }
            (
                field(&old, "id").as_str().unwrap().to_owned(),
                next_revision(revision)?,
                field(&old, "created_at").as_str().unwrap().to_owned(),
            )
        } else {
            if get(args, "expected_revision").is_some() {
                return Err(invalid(
                    "expected_revision is only valid when updating a label",
                ));
            }
            (sources.id(), 1, timestamp.clone())
        };
        let text = |key, index| sql(field(&value, key), index);
        conn.execute("INSERT INTO chain_analysis_labels VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET chain=excluded.chain,network=excluded.network,subject=excluded.subject,label=excluded.label,category=excluded.category,source=excluded.source,confidence=excluded.confidence,cluster_defining=excluded.cluster_defining,revision=excluded.revision,updated_at=excluded.updated_at",
            params![id, profile, text("chain",3)?, text("network",4)?, text("subject",5)?, text("label",6)?, text("category",7)?, text("source",8)?, text("confidence",9)?, i64::from(cluster), revision, 0, created, timestamp]).map_err(database_error)?;
        history(conn, &id, revision, &timestamp)
    })
}

pub(super) fn delete(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(
        args,
        &["id", "expected_revision"],
        &["id", "expected_revision"],
    )?;
    atomic(conn, || {
        let old = load(conn, profile, field(args, "id"))?;
        let Value::Int(revision) = field(&old, "revision") else {
            return Err(stale("Label changed; reload before deleting"));
        };
        if !matches!(get(args, "expected_revision"), Some(Value::Int(n)) if n == revision) {
            return Err(stale("Label changed; reload before deleting"));
        }
        let revision = next_revision(revision)?;
        let timestamp = sources.now();
        let id = field(&old, "id").as_str().unwrap();
        conn.execute(
            "UPDATE chain_analysis_labels SET deleted=1,revision=?,updated_at=? WHERE id=?",
            params![revision, timestamp, id],
        )
        .map_err(database_error)?;
        history(conn, id, revision, &timestamp)
    })
}

pub(super) fn import_labels(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(args, &["items"], &["items"])?;
    let items = match field(args, "items") {
        Value::List(items) if (1..=5000).contains(&items.len()) => items,
        _ => {
            return Err(invalid(
                "A label import requires between 1 and 5000 structured labels",
            ))
        }
    };
    atomic(conn, || {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            if !matches!(item, Value::Dict(_))
                || get(item, "id").is_some()
                || get(item, "expected_revision").is_some()
            {
                return Err(invalid(
                    "Label imports create new claims; use revisioned edits for existing labels",
                ));
            }
            results.push(upsert(conn, profile, item, sources)?);
        }
        Ok(dict([
            ("imported_count", Value::from(results.len() as i64)),
            ("items", Value::List(results)),
        ]))
    })
}
