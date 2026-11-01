use super::*;
use kassiber_pytext::strip;
use kassiber_store::rusqlite::params_from_iter;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const RECIPE_KEYS: &[&str] = &[
    "target_amount",
    "report_purpose",
    "planned_destination",
    "planned_note",
    "reveal_mode",
    "max_depth",
    "recipient",
    "report_options",
];

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

pub(super) fn require_fingerprint(fingerprint: &Value) -> Result<()> {
    if !matches!(fingerprint, Value::Str(raw) if raw.as_str().is_some_and(|s| s.len()==64 && s.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))))
    {
        return Err(validation("A source-funds review fingerprint is required"));
    }
    Ok(())
}

fn recipe_options(recipe: &Value) -> Result<Value> {
    if matches!(recipe, Value::Null) {
        return Ok(dict::<&str>([]));
    }
    let Value::Dict(fields) = recipe else {
        return Err(validation("Unsupported source-funds review recipe"));
    };
    if fields.iter().any(|(key,_)| !matches!(key,kassiber_pyjson::Key::Str(raw) if raw.as_str().is_some_and(|s|RECIPE_KEYS.contains(&s)))) {
        return Err(validation("Unsupported source-funds review recipe"));
    }
    for key in RECIPE_KEYS
        .iter()
        .copied()
        .filter(|key| !["max_depth", "report_options"].contains(key))
    {
        if !matches!(value(recipe, key), Value::Null | Value::Str(_)) {
            return Err(validation(format!(
                "Source-funds recipe {key} must be text"
            )));
        }
    }
    if get(recipe, "max_depth")
        .is_some_and(|v| !matches!(v,Value::Int(n) if *n>=1.into() && *n<=32.into()))
    {
        return Err(validation(
            "Source-funds review depth must be between 1 and 32",
        ));
    }
    if get(recipe, "report_options").is_some_and(|v| !matches!(v, Value::Dict(_))) {
        return Err(validation("Source-funds report options must be an object"));
    }
    let encoded = kassiber_pyjson::dumps(recipe, &kassiber_pyjson::Options::new())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    if encoded.len() > 16_384 {
        return Err(validation("Source-funds review recipe is too large"));
    }
    Ok(recipe.clone())
}

pub(super) fn digest(packet: &Value) -> Result<String> {
    let encoded = kassiber_pyjson::dumps(
        packet,
        &kassiber_pyjson::Options::new()
            .sort_keys(true)
            .compact()
            .allow_nan(false),
    )
    .map_err(|error| {
        AppError::new(
            "internal_error",
            if error == kassiber_pyjson::Error::OutOfRangeFloat {
                "Out of range float values are not JSON compliant".into()
            } else {
                error.to_string()
            },
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(encoded.as_bytes())))
}

pub fn review_context(
    conn: &Connection,
    profile: &Value,
    target_reference: &str,
    recipe: &Value,
) -> Result<Value> {
    let options = recipe_options(recipe)?;
    if strip(target_reference).is_empty() {
        return Err(validation("A source-funds target transaction is required"));
    }
    let owns = conn.is_autocommit();
    if owns {
        conn.execute_batch("BEGIN").map_err(sql_error)?;
    }
    let result = (|| {
        let current = rows(
            conn,
            "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
            [text(profile, "id"), text(profile, "workspace_id")],
        )?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::new("not_found", "Source-funds book was not found"))?;
        let profile_id = text(&current, "id");
        let target =
            super::source_funds_report::resolve_transaction(conn, &profile_id, target_reference)?;
        if truthy(value(&target, "excluded")) {
            return Err(validation(
                "Excluded transactions cannot anchor a source-funds investigation",
            ));
        }
        let workspace = rows(
            conn,
            "SELECT * FROM workspaces WHERE id=?",
            [text(&current, "workspace_id")],
        )?
        .remove(0);
        let mut report_options = options.clone();
        kassiber_envelope::set(
            &mut report_options,
            "target_transaction",
            value(&target, "id").clone(),
        );
        let report =
            super::source_funds_report::build(conn, &workspace, &current, &report_options, None)?;
        let target_amount = super::source_funds_projection::integer(value(
            value(&report, "allocations"),
            "target_amount_msat",
        ))?;
        let target_amount: Decimal = target_amount.to_string().parse().map_err(arithmetic)?;
        let target_amount = target_amount
            .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
            .map_err(arithmetic)?
            .to_fixed_string()
            .map_err(arithmetic)?;
        let canonical_recipe = dict([
            ("target_amount", s(target_amount)),
            (
                "report_purpose",
                value(value(&report, "purpose"), "type").clone(),
            ),
            (
                "planned_destination",
                value(value(&report, "purpose"), "planned_destination").clone(),
            ),
            (
                "planned_note",
                value(value(&report, "purpose"), "planned_note").clone(),
            ),
            ("reveal_mode", value(&report, "reveal_mode").clone()),
            (
                "max_depth",
                get(&options, "max_depth")
                    .cloned()
                    .unwrap_or(Value::Int(8.into())),
            ),
            (
                "recipient",
                value(value(&report, "recipient"), "id").clone(),
            ),
            ("report_options", value(&report, "report_options").clone()),
        ]);
        let mut transaction_ids = BTreeSet::from([text(&target, "id")]);
        if let Value::List(nodes) = value(value(&report, "graph"), "nodes") {
            for node in nodes
                .iter()
                .filter(|node| text(node, "node_type") == "transaction")
            {
                transaction_ids.insert(text(node, "transaction_id"));
            }
        }
        let transaction_ids: Vec<_> = transaction_ids.into_iter().collect();
        let marks = vec!["?"; transaction_ids.len()].join(",");
        let mut bindings = vec![profile_id.clone()];
        bindings.extend(transaction_ids.iter().cloned());
        bindings.push("201".into());
        let mut link_rows=rows(conn,&format!("SELECT * FROM source_funds_links WHERE profile_id=? AND to_transaction_id IN ({marks}) ORDER BY id LIMIT ?"),params_from_iter(bindings))?;
        let mut truncated = link_rows.len() > 200;
        link_rows.truncate(200);
        let links = link_rows
            .iter()
            .map(|r| super::source_funds_reads::link(conn, r))
            .collect::<Result<Vec<_>>>()?;
        let source_ids: BTreeSet<_> = link_rows
            .iter()
            .filter(|row| truthy(value(row, "from_source_id")))
            .map(|row| text(row, "from_source_id"))
            .collect();
        let mut sources = Vec::new();
        for id in source_ids {
            for row in rows(
                conn,
                "SELECT * FROM source_funds_sources WHERE profile_id=? AND id=?",
                [&profile_id, &id],
            )? {
                sources.push(super::source_funds_reads::source(conn, &row)?);
            }
        }
        let mut attachment_ids = BTreeSet::new();
        for record in links.iter().chain(sources.iter()) {
            if let Value::List(attachments) = value(record, "attachments") {
                attachment_ids.extend(attachments.iter().map(|r| text(r, "id")));
            }
        }
        let mut evidence_where = format!("transaction_id IN ({marks})");
        let mut bindings = vec![profile_id.clone()];
        bindings.extend(transaction_ids.iter().cloned());
        if !attachment_ids.is_empty() {
            evidence_where.push_str(&format!(
                " OR id IN ({})",
                vec!["?"; attachment_ids.len()].join(",")
            ));
            bindings.extend(attachment_ids);
        }
        bindings.push("201".into());
        let mut evidence=rows(conn,&format!("SELECT id,transaction_id,attachment_type,label,original_filename,media_type,size_bytes,sha256,created_at FROM attachments WHERE profile_id=? AND ({evidence_where}) ORDER BY id LIMIT ?"),params_from_iter(bindings))?;
        truncated |= evidence.len() > 200;
        evidence.truncate(200);
        let mut needs = BTreeSet::new();
        if let Value::List(findings) = value(&report, "findings") {
            needs.extend(findings.iter().map(|r| text(r, "code")));
        }
        if sources.iter().any(|row| {
            ["missing_history", "opening_balance_attestation"]
                .contains(&text(row, "source_type").as_str())
        }) {
            needs.insert("documentary_origin_attested".into());
        }
        if sources.iter().any(|row| !truthy(value(row, "attachments"))) || evidence.is_empty() {
            needs.insert("documentary_evidence_missing".into());
        }
        let target_id = text(&target, "id");
        let target_row=rows(conn,"SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS wallet_config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? AND t.id=? AND t.excluded=0",[&profile_id,&target_id])?.into_iter().next();
        if let Some(row) = target_row {
            let raw = super::source_funds_assembly::mapping(value(&row, "raw_json"));
            let authoritative =
                kassiber_custody::evidence::authority(&kassiber_custody::codec::to_json(&row))
                    .ok()
                    .and_then(|v| v["authoritative"].as_bool())
                    .unwrap_or(false);
            if text(&row, "direction") == "outbound"
                && authoritative
                && get(&raw, "observer_owned_scripts").is_none()
                && matches!(value(&raw,"vin"),Value::List(v) if !v.is_empty())
            {
                needs.insert("chain_attestation_missing".into());
            }
        }
        let target_projection = dict([
            ("transaction_id", s(target_id.as_str())),
            ("wallet_id", value(&target, "wallet_id").clone()),
            ("direction", value(&target, "direction").clone()),
            ("asset", value(&target, "asset").clone()),
            ("occurred_at", value(&target, "occurred_at").clone()),
        ]);
        let mut packet = dict([
            ("schema_version", Value::Int(1.into())),
            ("domain", s("source_funds")),
            ("workspace_id", value(&current, "workspace_id").clone()),
            ("profile_id", s(profile_id.as_str())),
            (
                "input_version",
                Value::Int(super::source_funds_assembly::integer(
                    get(&current, "journal_input_version")
                        .filter(|v| truthy(v))
                        .unwrap_or(&Value::Int(0.into())),
                )?),
            ),
            ("case_id", s(format!("source_funds:{target_id}"))),
            ("target", target_projection),
            ("recipe", canonical_recipe),
            ("report", report),
            ("links", Value::List(links.clone())),
            ("sources", Value::List(sources)),
            ("evidence", Value::List(evidence)),
            ("scope_truncated", Value::Bool(truncated)),
            (
                "input_needs",
                Value::List(needs.into_iter().map(s).collect()),
            ),
        ]);
        let fingerprint = digest(&packet)?;
        kassiber_envelope::set(&mut packet, "review_fingerprint", s(fingerprint));
        let eligible = super::source_funds_batch::eligible_ids(conn, &profile_id, &target_id)?;
        let beyond = eligible
            .iter()
            .filter(|id| !links.iter().any(|link| text(link, "id") == **id))
            .count();
        kassiber_envelope::set(
            &mut packet,
            "bulk_review",
            dict([
                (
                    "eligible_link_ids",
                    Value::List(eligible.into_iter().map(s).collect()),
                ),
                ("eligible_beyond_inspection", Value::Int(beyond.into())),
            ]),
        );
        Ok(packet)
    })();
    if owns {
        conn.execute_batch("ROLLBACK").map_err(sql_error)?;
    }
    result
}

pub(super) fn save_reviewed_case(
    conn: &Connection,
    profile: &Value,
    args: &Value,
    label: Option<&str>,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let expected = value(args, "expected_review_fingerprint");
    require_fingerprint(expected)?;
    if !conn.is_autocommit() {
        return Err(validation(
            "A reviewed source-funds save requires an idle database transaction",
        ));
    }
    crate::export::admitted(conn, || {
        let recipe = dict(
            RECIPE_KEYS
                .iter()
                .filter_map(|key| get(args, key).map(|v| (*key, v.clone()))),
        );
        let context = review_context(conn, profile, &text(args, "target_transaction"), &recipe)?;
        if value(&context, "review_fingerprint") != expected {
            return Err(AppError::new(
                "source_funds_review_stale",
                "Source-funds evidence changed; inspect before saving",
            )
            .retryable(true));
        }
        if truthy(value(&context, "scope_truncated")) {
            return Err(validation(
                "Source-funds review scope is truncated; narrow the investigation before saving",
            ));
        }
        let mut recipe = value(&context, "recipe").clone();
        kassiber_envelope::set(
            &mut recipe,
            "target_transaction",
            value(value(&context, "target"), "transaction_id").clone(),
        );
        kassiber_envelope::set(&mut recipe, "include_diagrams", Value::Bool(true));
        let workspace = rows(
            conn,
            "SELECT * FROM workspaces WHERE id=?",
            [text(&context, "workspace_id")],
        )?
        .remove(0);
        let current = rows(
            conn,
            "SELECT * FROM profiles WHERE id=?",
            [text(&context, "profile_id")],
        )?
        .remove(0);
        let report = super::source_funds_report::build(conn, &workspace, &current, &recipe, None)?;
        super::source_funds_cases::save_report(conn, &workspace, &current, report, label, sources)
    })
}

pub(super) fn execute(conn: &Connection, args: &Value) -> Result<Value> {
    let empty = dict::<&str>([]);
    let args = if matches!(args, Value::Null) {
        &empty
    } else {
        args
    };
    let Value::Dict(fields) = args else {
        return Err(validation("daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])));
    };
    if fields.iter().any(|(key,_)| !matches!(key,kassiber_pyjson::Key::Str(raw) if raw.as_str().is_some_and(|key|key=="target_transaction" || RECIPE_KEYS.contains(&key)))) {
        return Err(validation("Unsupported source-funds review arguments"));
    }
    let (_, profile) = kassiber_setup::references::scope(conn, &empty)?;
    let recipe = dict(
        RECIPE_KEYS
            .iter()
            .filter_map(|key| get(args, key).map(|v| (*key, v.clone()))),
    );
    let Value::Str(target) = value(args, "target_transaction") else {
        return Err(validation("A source-funds target transaction is required"));
    };
    review_context(conn, &profile, target.as_str().unwrap_or_default(), &recipe)
}
