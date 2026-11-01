use super::*;
use kassiber_pytext::{lower, strip};
use kassiber_setup::Sources;
use kassiber_store::rusqlite::{params, params_from_iter, types::Value as SqlValue};
use source_funds_mutations::{
    optional, parameter, require_attachment, required_text, resolve_source, Amount,
};

const LINK_TYPES: &[&str] = &[
    "self_transfer",
    "exchange_transfer",
    "trade",
    "swap",
    "peg_in",
    "peg_out",
    "lightning_funding",
    "lightning_close",
    "lightning_routed",
    "lightning_swap",
    "coinjoin",
    "payjoin",
    "manual_source",
    "missing_history",
];

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.links.create"
            | "source-funds.links.review"
            | "source-funds.links.attach"
            | "ui.source_funds.links.create"
            | "ui.source_funds.links.review"
            | "ui.source_funds.links.attach"
    )
}

fn normalize(raw: &Value, key: &str) -> Result<String> {
    let (default, noun, choices) = match key {
        "link_type" => ("", "link type", LINK_TYPES),
        "state" => (
            "suggested",
            "source-funds link state",
            &["suggested", "reviewed", "rejected"][..],
        ),
        "confidence" => (
            "unknown",
            "confidence",
            &["exact", "strong", "weak", "unknown"][..],
        ),
        "allocation_policy" => (
            "unknown",
            "allocation policy",
            &["explicit", "heuristic", "unknown"][..],
        ),
        _ => unreachable!(),
    };
    let normalized = if truthy(raw) {
        py_str(raw)
    } else {
        default.into()
    };
    let mut normalized = lower(strip(&normalized));
    if key == "link_type" {
        normalized = normalized.replace('-', "_");
    }
    if key == "state" {
        normalized = match normalized.as_str() {
            "accept" | "accepted" => "reviewed".into(),
            "reject" => "rejected".into(),
            _ => normalized,
        };
    }
    if !choices.contains(&normalized.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported {noun} '{}'", py_str(raw)),
        )
        .with_hint(format!("Choose one of: {}", choices.join(", "))));
    }
    Ok(normalized)
}

fn amount(raw: &Value, label: &str) -> Result<Value> {
    match source_funds_mutations::amount(raw).map_err(|mut e| {
        if e.code == "validation" {
            e.message = format!("{label} must not be negative");
        }
        e
    })? {
        Amount::Missing => Ok(Value::Null),
        Amount::Msat(n) => Ok(Value::Int(n.into())),
        Amount::Overflow => Ok(Value::Int(
            kassiber_pytext::parse_int("9223372036854775808").unwrap(),
        )),
    }
}

fn transaction(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    Ok(rows(conn, "SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.id=?", params![profile, parameter(conn,id,2)?])?.into_iter().next().unwrap_or(Value::Null))
}

fn after(left: &Value, right: &Value) -> bool {
    let canonical = |v: &Value| {
        if optional(v) {
            return None;
        }
        kassiber_wallets::time::timestamp(&py_str(v))
            .ok()
            .filter(|v| v != "1970-01-01T00:00:00Z")
    };
    match (canonical(left), canonical(right)) {
        (Some(left), Some(right)) => left > right,
        _ => false,
    }
}

fn exceeds(amount: &Value, row: &Value) -> Result<bool> {
    Ok(!matches!(amount, Value::Null) && decimal(amount)? > decimal(value(row, "amount"))?)
}

pub(super) fn validate(candidate: &Value, from: &Value, source: &Value, to: &Value) -> Result<()> {
    let fail = |message| Err(AppError::new("validation", message));
    if exceeds(value(candidate, "allocation_amount"), to)? {
        return fail("A source-funds link allocation cannot exceed the target transaction amount.");
    }
    if truthy(source) && after(value(source, "acquired_at"), value(to, "occurred_at")) {
        return fail("A source-funds source cannot be acquired after the transaction it funds.");
    }
    if !truthy(from) {
        return Ok(());
    }
    if value(from, "id") == value(to, "id") {
        return fail("A source-funds link's from-transaction and to-transaction must differ.");
    }
    if after(value(from, "occurred_at"), value(to, "occurred_at")) {
        return fail("A source-funds link's parent transaction occurs after the child.");
    }
    let parent_amount = if matches!(value(candidate, "from_allocation_amount"), Value::Null) {
        value(candidate, "allocation_amount")
    } else {
        value(candidate, "from_allocation_amount")
    };
    if exceeds(parent_amount, from)? {
        return fail(
            "A source-funds link from-allocation cannot exceed the parent transaction amount.",
        );
    }
    if text(candidate, "link_type") != "self_transfer" {
        return Ok(());
    }
    let asset = |v: &Value| kassiber_wallets::config::asset_code(&py_str(v));
    let parent = asset(value(from, "asset"));
    let child = asset(value(to, "asset"));
    if parent != child {
        return Err(AppError::new(
            "validation",
            "Self-transfer source-funds links require the same asset on both transactions.",
        )
        .with_hint("Use a swap, peg-in, or peg-out link for cross-asset flows."));
    }
    if asset(value(candidate, "asset")) != child {
        return fail("A self-transfer link's asset must match the target transaction asset.");
    }
    let from_asset = value(candidate, "from_asset");
    if asset(if truthy(from_asset) {
        from_asset
    } else {
        value(from, "asset")
    }) != parent
    {
        return fail("A self-transfer link's from-asset must match the parent transaction asset.");
    }
    Ok(())
}

fn resolve_link(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM source_funds_links WHERE profile_id=? AND id=?",
        [profile, reference],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Source-funds link '{reference}' not found"),
        )
    })
}

fn response(conn: &Connection, id: &str) -> Result<Value> {
    let row = rows(conn, "SELECT * FROM source_funds_links WHERE id=?", [id])?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::new("internal_error", "'NoneType' object is not subscriptable"))?;
    source_funds_reads::link(conn, &row)
}

fn create(
    conn: &Connection,
    args: &Value,
    workspace: &Value,
    profile: &str,
    sources: &dyn Sources,
) -> Result<Value> {
    let from_ref = value(args, "from_transaction");
    let source_ref = value(args, "from_source");
    if truthy(from_ref) == truthy(source_ref) {
        return Err(AppError::new(
            "validation",
            "Provide exactly one of --from-transaction or --from-source",
        ));
    }
    let to_id =
        source_funds_reads::resolve_transaction(conn, profile, &text(args, "to_transaction"))?;
    let to = transaction(conn, profile, &s(to_id.as_str()))?;
    let from_id = if truthy(from_ref) {
        s(source_funds_reads::resolve_transaction(
            conn,
            profile,
            &py_str(from_ref),
        )?)
    } else {
        Value::Null
    };
    let from = transaction(conn, profile, &from_id)?;
    let source = if truthy(source_ref) {
        resolve_source(conn, profile, &py_str(source_ref))?
    } else {
        Value::Null
    };
    let mut candidate = dict::<&str>([]);
    for key in ["link_type", "state", "confidence", "allocation_policy"] {
        kassiber_envelope::set(&mut candidate, key, s(normalize(value(args, key), key)?));
    }
    for (key, fallback) in [
        ("asset", &to),
        ("from_asset", if truthy(&from) { &from } else { &source }),
    ] {
        let raw = value(args, key);
        let raw = if truthy(raw) {
            raw
        } else {
            value(fallback, "asset")
        };
        kassiber_envelope::set(
            &mut candidate,
            key,
            s(kassiber_wallets::config::asset_code(&py_str(raw))),
        );
    }
    for (key, label) in [
        ("allocation_amount", "--allocation-amount"),
        ("from_allocation_amount", "--from-amount"),
    ] {
        kassiber_envelope::set(&mut candidate, key, amount(value(args, key), label)?);
    }
    validate(&candidate, &from, &source, &to)?;
    let source_id = value(&source, "id");
    let endpoint = if truthy(&source) {
        "from_source_id"
    } else {
        "from_transaction_id"
    };
    let endpoint_id = if truthy(&source) { source_id } else { &from_id };
    let found = rows(conn,&format!("SELECT * FROM source_funds_links WHERE profile_id=? AND {endpoint}=? AND to_transaction_id=? AND method=? AND link_type=? LIMIT 1"),params![profile,parameter(conn,endpoint_id,2)?,to_id,parameter(conn,value(args,"method"),4)?,text(&candidate,"link_type")])?;
    if let Some(existing) = found.first() {
        return source_funds_reads::link(conn, existing);
    }
    let id = sources.uuid(conn)?;
    let now = sources.now(conn)?;
    conn.execute("INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",params![id,text(workspace,"id"),profile,parameter(conn,source_id,4)?,parameter(conn,&from_id,5)?,to_id,text(&candidate,"link_type"),text(&candidate,"state"),text(&candidate,"confidence"),parameter(conn,value(args,"method"),10)?,text(&candidate,"asset"),parameter(conn,value(&candidate,"allocation_amount"),12)?,text(&candidate,"from_asset"),parameter(conn,value(&candidate,"from_allocation_amount"),14)?,text(&candidate,"allocation_policy"),parameter(conn,value(args,"explanation"),16)?,i64::from(truthy(value(args,"uses_chain_observation"))),i64::from(truthy(value(args,"chain_data_confirmed"))),now,now]).map_err(sql_error)?;
    if let Value::List(attachments) = value(args, "attachment_ids") {
        for attachment in attachments {
            let attachment = py_str(attachment);
            require_attachment(conn, profile, &attachment)?;
            conn.execute("INSERT OR IGNORE INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES(?,?,?)",params![id,attachment,now]).map_err(sql_error)?;
        }
    }
    response(conn, &id)
}

fn review(
    conn: &Connection,
    args: &Value,
    link: &Value,
    profile: &str,
    sources: &dyn Sources,
) -> Result<Value> {
    let raw_state = value(args, "state");
    if !matches!(raw_state, Value::Null)
        && normalize(raw_state, "state")? == "rejected"
        && (!optional(value(args, "allocation_amount"))
            || !optional(value(args, "from_allocation_amount")))
    {
        return Err(AppError::new(
            "validation",
            "A rejected source-funds link cannot carry an allocation edit",
        )
        .with_hint("Reject the link on its own, or correct the allocation in a separate review."));
    }
    for (key, column, label) in [
        (
            "expected_allocation_amount_msat",
            "allocation_amount",
            "allocation amount",
        ),
        (
            "expected_from_allocation_amount_msat",
            "from_allocation_amount",
            "source amount",
        ),
    ] {
        let raw = value(args, key);
        if optional(raw) {
            continue;
        }
        let expected = source_funds_reads::integer(raw).map_err(|message| {
            if matches!(raw, Value::Float(number) if number.is_infinite()) {
                AppError::new("internal_error", message)
            } else {
                AppError::new(
                    "validation",
                    format!("Expected {label} must be an exact millisatoshi integer"),
                )
            }
        })?;
        let expected = kassiber_pytext::parse_int(&expected)
            .map_err(|_| AppError::new("internal_error", "invalid integer"))?;
        let current = if truthy(value(link, column)) {
            value(link, column).clone()
        } else {
            Value::Int(0.into())
        };
        if current != Value::Int(expected) {
            return Err(AppError::new(
                "source_funds_link_stale",
                "This source-funds link changed since it was inspected",
            )
            .with_hint("Reload the investigation, check the updated amounts, and review it again.")
            .with_details(dict([
                ("link", value(link, "id").clone()),
                ("field", s(column)),
            ])));
        }
    }
    let mut updates = Vec::new();
    for key in ["state", "link_type", "confidence"] {
        if !matches!(value(args, key), Value::Null) {
            updates.push((key, s(normalize(value(args, key), key)?)));
        }
    }
    for (key, label) in [
        ("allocation_amount", "--allocation-amount"),
        ("from_allocation_amount", "--from-amount"),
    ] {
        if !optional(value(args, key)) {
            updates.push((key, amount(value(args, key), label)?));
        }
    }
    if !matches!(value(args, "allocation_policy"), Value::Null) {
        updates.push((
            "allocation_policy",
            s(normalize(
                value(args, "allocation_policy"),
                "allocation_policy",
            )?),
        ));
    }
    if !matches!(value(args, "explanation"), Value::Null) {
        updates.push(("explanation", value(args, "explanation").clone()));
    }
    for key in ["uses_chain_observation", "chain_data_confirmed"] {
        match value(args, key) {
            Value::Null => (),
            Value::Bool(v) => updates.push((key, Value::Int(i64::from(*v).into()))),
            _ => {
                return Err(AppError::new(
                    "validation",
                    format!("{key} must be a boolean"),
                ))
            }
        }
    }
    if updates.is_empty() {
        return Err(AppError::new(
            "validation",
            "source-funds links review requires at least one update",
        ));
    }
    let mut candidate = link.clone();
    for (key, v) in &updates {
        kassiber_envelope::set(&mut candidate, key, v.clone());
    }
    if text(&candidate, "state") == "reviewed"
        || updates
            .iter()
            .any(|(key, _)| ["allocation_amount", "from_allocation_amount"].contains(key))
    {
        let from = transaction(conn, profile, value(link, "from_transaction_id"))?;
        let to = transaction(conn, profile, value(link, "to_transaction_id"))?;
        let source = if truthy(value(link, "from_source_id")) {
            rows(
                conn,
                "SELECT * FROM source_funds_sources WHERE id=?",
                [text(link, "from_source_id")],
            )?
            .into_iter()
            .next()
            .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        if truthy(&to) {
            validate(&candidate, &from, &source, &to)?;
        }
    }
    updates.push(("updated_at", s(sources.now(conn)?)));
    let assignments = updates
        .iter()
        .map(|(key, _)| format!("{key}=?"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut parameters = updates
        .iter()
        .enumerate()
        .map(|(index, (_, v))| parameter(conn, v, index + 1))
        .collect::<Result<Vec<SqlValue>>>()?;
    parameters.push(SqlValue::Text(text(link, "id")));
    conn.execute(
        &format!("UPDATE source_funds_links SET {assignments} WHERE id=?"),
        params_from_iter(parameters),
    )
    .map_err(sql_error)?;
    response(conn, &text(link, "id"))
}

pub fn execute(
    conn: &Connection,
    command: &str,
    raw: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let mut args = match raw {
        Value::Null if desktop => dict::<&str>([]),
        Value::Dict(_) => raw.clone(),
        _ if desktop => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(kassiber_envelope::type_name(raw)))])))
        }
        _ => raw.clone(),
    };
    let creating = command.ends_with(".create");
    if creating {
        if desktop {
            let attachments = match value(&args, "attachment_ids") {
                Value::Null => match value(&args, "attachment_id") {
                    Value::Str(v) if !v.is("") => vec![Value::Str(v.clone())],
                    _ => vec![],
                },
                Value::List(v) => v.iter().map(|v| s(py_str(v))).collect(),
                _ => {
                    return Err(AppError::new(
                        "validation",
                        "ui.source_funds.links.create attachment_ids must be a list",
                    ))
                }
            };
            kassiber_envelope::set(&mut args, "attachment_ids", Value::List(attachments));
            for (key, default) in [
                ("to_transaction", ""),
                ("link_type", "self_transfer"),
                ("state", "reviewed"),
                ("confidence", "strong"),
                ("method", "manual"),
                ("allocation_policy", "explicit"),
            ] {
                let v = value(&args, key);
                let v = if truthy(v) { py_str(v) } else { default.into() };
                kassiber_envelope::set(&mut args, key, s(v));
            }
            for key in ["uses_chain_observation", "chain_data_confirmed"] {
                if let Some(v) = get(&args, key) {
                    if !matches!(v, Value::Bool(_)) {
                        return Err(AppError::new(
                            "validation",
                            format!("{key} must be a boolean"),
                        )
                        .with_details(dict([("type", s(kassiber_envelope::type_name(v)))])));
                    }
                }
            }
        } else {
            let attachments = value(&args, "attachments").clone();
            kassiber_envelope::set(&mut args, "attachment_ids", attachments);
        }
    }
    if desktop {
        for key in if creating {
            &[
                "from_transaction",
                "from_source",
                "asset",
                "from_asset",
                "explanation",
            ][..]
        } else {
            &[
                "state",
                "link_type",
                "confidence",
                "allocation_policy",
                "explanation",
            ][..]
        } {
            if !matches!(value(&args, key), Value::Str(_)) {
                kassiber_envelope::set(&mut args, key, Value::Null);
            }
        }
        if !creating {
            for key in ["uses_chain_observation", "chain_data_confirmed"] {
                if !matches!(value(&args, key), Value::Bool(_)) {
                    kassiber_envelope::set(&mut args, key, Value::Null);
                }
            }
        }
    }
    let empty = dict::<&str>([]);
    let link_ref = if !creating && desktop {
        required_text(&args, "link", command)?
    } else {
        text(&args, "link")
    };
    let attachment = if command.ends_with(".attach") && desktop {
        required_text(&args, "attachment_id", command)?
    } else {
        text(&args, "attachment")
    };
    let (workspace, profile) =
        kassiber_setup::references::scope(conn, if desktop { &empty } else { &args })?;
    let profile = text(&profile, "id");
    crate::export::admitted(conn, || {
        if creating {
            return create(conn, &args, &workspace, &profile, sources);
        }
        let link = resolve_link(conn, &profile, &link_ref)?;
        if command.ends_with(".review") {
            return review(conn, &args, &link, &profile, sources);
        }
        require_attachment(conn, &profile, &attachment)?;
        conn.execute("INSERT OR IGNORE INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES(?,?,?)",params![text(&link,"id"),attachment,sources.now(conn)?]).map_err(sql_error)?;
        source_funds_reads::link(conn, &link)
    })
}
