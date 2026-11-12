use super::source_funds_report as report;
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};

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

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.review-context"
            | "source-funds.request-input"
            | "ui.source_funds.review_context"
            | "ui.source_funds.request_input"
    )
}

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

fn list<'a>(row: &'a Value, key: &str) -> &'a [Value] {
    match value(row, key) {
        Value::List(v) => v,
        _ => &[],
    }
}

fn marks(count: usize) -> String {
    vec!["?"; count].join(",")
}

pub fn context(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    target: &Value,
    recipe: &Value,
) -> Result<Value> {
    let mut options = recipe_options(recipe)?;
    if !matches!(target, Value::Str(v) if kassiber_pytext::strip_py(v).code_points().next().is_some())
    {
        return Err(validation("A source-funds target transaction is required"));
    }
    let current = rows(
        conn,
        "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
        [&text(profile, "id"), &text(profile, "workspace_id")],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Source-funds book was not found"))?;
    let profile_id = text(&current, "id");
    let target = report::resolve_transaction(conn, &profile_id, &py_str(target))?;
    if truthy(value(&target, "excluded")) {
        return Err(validation(
            "Excluded transactions cannot anchor a source-funds investigation",
        ));
    }
    replace(
        &mut options,
        "target_transaction",
        value(&target, "id").clone(),
    );
    let report = report::build(conn, workspace, &current, &options, None)?;
    let msat: Decimal = super::source_funds_projection::integer(value(
        value(&report, "allocations"),
        "target_amount_msat",
    ))?
    .to_string()
    .parse()
    .map_err(arithmetic)?;
    let canonical = dict([
        (
            "target_amount",
            s(msat
                .div(&Decimal::from(100_000_000_000i64), &DECIMAL)
                .map_err(arithmetic)?
                .to_fixed_string()
                .map_err(arithmetic)?),
        ),
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
            get(recipe, "max_depth")
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
    for node in list(value(&report, "graph"), "nodes") {
        if text(node, "node_type") == "transaction" {
            transaction_ids.insert(text(node, "transaction_id"));
        }
    }
    let mut params = vec![profile_id.clone()];
    params.extend(transaction_ids.iter().cloned());
    let mut link_rows = rows(conn,&format!("SELECT * FROM source_funds_links WHERE profile_id=? AND to_transaction_id IN ({}) ORDER BY id LIMIT 201",marks(transaction_ids.len())),kassiber_store::rusqlite::params_from_iter(&params))?;
    let mut truncated = link_rows.len() > 200;
    link_rows.truncate(200);
    let links = link_rows
        .iter()
        .map(|row| super::source_funds_reads::link(conn, row))
        .collect::<Result<Vec<_>>>()?;
    let source_ids: BTreeSet<_> = link_rows
        .iter()
        .filter(|r| truthy(value(r, "from_source_id")))
        .map(|r| text(r, "from_source_id"))
        .collect();
    let mut sources = Vec::new();
    for source in source_ids {
        if let Some(row) = rows(
            conn,
            "SELECT * FROM source_funds_sources WHERE profile_id=? AND id=?",
            [&profile_id, &source],
        )?
        .first()
        {
            sources.push(super::source_funds_reads::source(conn, row)?);
        }
    }
    let attachment_ids: BTreeSet<_> = links
        .iter()
        .chain(&sources)
        .flat_map(|r| list(r, "attachments"))
        .map(|a| text(a, "id"))
        .collect();
    let mut predicate = format!("transaction_id IN ({})", marks(transaction_ids.len()));
    if !attachment_ids.is_empty() {
        predicate.push_str(&format!(" OR id IN ({})", marks(attachment_ids.len())));
        params.extend(attachment_ids);
    }
    let mut evidence = rows(conn,&format!("SELECT id,transaction_id,attachment_type,label,original_filename,media_type,size_bytes,sha256,created_at FROM attachments WHERE profile_id=? AND ({predicate}) ORDER BY id LIMIT 201"),kassiber_store::rusqlite::params_from_iter(&params))?;
    truncated |= evidence.len() > 200;
    evidence.truncate(200);
    let mut needs: BTreeSet<_> = list(&report, "findings")
        .iter()
        .map(|r| text(r, "code"))
        .collect();
    if sources.iter().any(|r| {
        matches!(
            text(r, "source_type").as_str(),
            "missing_history" | "opening_balance_attestation"
        )
    }) {
        needs.insert("documentary_origin_attested".into());
    }
    if sources.iter().any(|r| list(r, "attachments").is_empty()) || evidence.is_empty() {
        needs.insert("documentary_evidence_missing".into());
    }
    if let Some(row) = rows(conn,"SELECT t.*,w.config_json AS wallet_config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? AND t.id=? AND t.excluded=0",[&profile_id,&text(&target,"id")])?.first() {
        let row = kassiber_custody::codec::to_json(row);
        let authoritative = kassiber_custody::evidence::authority(&row).is_ok_and(|v|v["authoritative"]==true);
        let outer = kassiber_custody::confirmation::stored_mapping(&row["raw_json"],false).unwrap_or_default();
        if row["direction"]=="outbound" && authoritative && outer.get("observer_owned_scripts").is_none() && outer["vin"].as_array().is_some_and(|v|!v.is_empty()) {
            needs.insert("chain_attestation_missing".into());
        }
    }
    let mut target_fields = vec![("transaction_id", value(&target, "id").clone())];
    target_fields.extend(
        ["wallet_id", "direction", "asset", "occurred_at"].map(|k| (k, value(&target, k).clone())),
    );
    let mut packet = dict([
        ("schema_version", Value::Int(1.into())),
        ("domain", s("source_funds")),
        ("workspace_id", value(&current, "workspace_id").clone()),
        ("profile_id", s(profile_id.clone())),
        (
            "input_version",
            Value::Int(super::source_funds_projection::integer(value(
                &current,
                "journal_input_version",
            ))?),
        ),
        (
            "case_id",
            s(format!("source_funds:{}", text(&target, "id"))),
        ),
        ("target", dict(target_fields)),
        ("recipe", canonical),
        ("report", report),
        ("links", Value::List(links)),
        ("sources", Value::List(sources)),
        ("evidence", Value::List(evidence)),
        ("scope_truncated", Value::Bool(truncated)),
        (
            "input_needs",
            Value::List(needs.into_iter().map(s).collect()),
        ),
    ]);
    let fingerprint = s(digest(&packet)?);
    replace(&mut packet, "review_fingerprint", fingerprint);
    let eligible =
        super::source_funds_batch::eligible_ids(conn, &profile_id, &text(&target, "id"))?;
    let inspected: HashSet<_> = link_rows.iter().map(|r| text(r, "id")).collect();
    let beyond = eligible
        .iter()
        .filter(|id| !inspected.contains(*id))
        .count();
    replace(
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
}

fn request(conn: &Connection, workspace: &Value, profile: &Value, args: &Value) -> Result<Value> {
    if !matches!(
        text(args, "action").as_str(),
        "connect_wallet" | "import_history" | "attach_evidence"
    ) {
        return Err(validation("Unsupported source-funds input action"));
    }
    let fingerprint = value(args, "expected_review_fingerprint");
    if !matches!(fingerprint,Value::Str(v) if v.as_str().is_some_and(|v|v.len()==64 && v.bytes().all(|c|c.is_ascii_digit() || (b'a'..=b'f').contains(&c))))
    {
        return Err(validation("A source-funds review fingerprint is required"));
    }
    let explanation = value(args, "explanation");
    if !matches!(explanation, Value::Null)
        && !matches!(explanation,Value::Str(v) if {
            let trimmed=kassiber_pytext::strip_py(v);
            (1..=1000).contains(&trimmed.code_points().count()) && !v.code_points().any(|c|c<32 && c!=10 && c!=9)
        })
    {
        return Err(validation(
            "Source-funds input explanation must be bounded plain text",
        ));
    }
    let context = context(
        conn,
        workspace,
        profile,
        value(args, "target_transaction"),
        value(args, "recipe"),
    )?;
    if value(&context, "review_fingerprint") != fingerprint {
        return Err(AppError::new(
            "source_funds_review_stale",
            "Source-funds evidence changed; inspect the target again",
        )
        .retryable(true));
    }
    if truthy(value(&context, "scope_truncated")) {
        return Err(validation(
            "Source-funds review scope is truncated; narrow the investigation",
        ));
    }
    if list(&context, "input_needs").is_empty() {
        return Err(validation(
            "This source-funds investigation has no missing documentary input",
        ));
    }
    let mut packet = dict(
        [
            "schema_version",
            "domain",
            "workspace_id",
            "profile_id",
            "input_version",
            "review_fingerprint",
            "recipe",
        ]
        .map(|k| (k, value(&context, k).clone())),
    );
    replace(&mut packet, "action", value(args, "action").clone());
    replace(
        &mut packet,
        "target_transaction",
        value(value(&context, "target"), "transaction_id").clone(),
    );
    let mut case = value(&context, "target").clone();
    replace(&mut case, "case_id", value(&context, "case_id").clone());
    replace(&mut case, "reason", s("evidence_review"));
    replace(&mut packet, "cases", Value::List(vec![case]));
    replace(
        &mut packet,
        "explanation",
        if let Value::Str(v) = explanation {
            Value::Str(kassiber_pytext::strip_py(v))
        } else {
            Value::Null
        },
    );
    let request_id = s(digest(&packet)?);
    replace(&mut packet, "request_id", request_id);
    Ok(packet)
}

pub fn execute(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let input = command.ends_with("request-input") || command.ends_with("request_input");
    let empty = dict::<&str>([]);
    let args = if desktop && matches!(args, Value::Null) {
        &empty
    } else {
        args
    };
    let mut recipe = dict::<&str>([]);
    if desktop {
        let Value::Dict(fields) = args else {
            return Err(validation("daemon args must be an object")
                .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])));
        };
        let allowed = if input {
            vec![
                "action",
                "target_transaction",
                "expected_review_fingerprint",
                "recipe",
                "explanation",
            ]
        } else {
            RECIPE_KEYS
                .iter()
                .copied()
                .chain(["target_transaction"])
                .collect()
        };
        if fields
            .iter()
            .any(|(k, _)| !allowed.iter().any(|a| k == &kassiber_pyjson::Key::from(*a)))
            || (input
                && [
                    "action",
                    "target_transaction",
                    "expected_review_fingerprint",
                ]
                .iter()
                .any(|k| get(args, k).is_none()))
        {
            return Err(validation(if input {
                "Unsupported source-funds input arguments"
            } else {
                "Unsupported source-funds review arguments"
            }));
        }
        if !input {
            recipe = dict(
                RECIPE_KEYS
                    .iter()
                    .filter_map(|k| get(args, k).map(|v| (*k, v.clone()))),
            );
        }
    }
    let (workspace, profile) =
        kassiber_setup::references::scope(conn, if desktop { &empty } else { args })?;
    if !desktop {
        recipe = read_recipe(value(args, "recipe_file"))?;
    }
    if input {
        let mut options = args.clone();
        if !desktop {
            replace(&mut options, "recipe", recipe);
        }
        request(conn, &workspace, &profile, &options)
    } else {
        context(
            conn,
            &workspace,
            &profile,
            value(args, "target_transaction"),
            &recipe,
        )
    }
}

fn read_recipe(path: &Value) -> Result<Value> {
    use std::io::Read;
    if !truthy(path) {
        return Ok(dict::<&str>([]));
    }
    let raw = py_str(path);
    let path = kassiber_store::PathContext::from_env()
        .and_then(|context| context.expand_user(std::path::Path::new(&raw)))
        .map_err(|_| AppError::new("internal_error", "Could not determine home directory."))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(1_000_001).read_to_end(&mut bytes))
        .map_err(|_| validation("Unable to read the review input file"))?;
    if bytes.len() > 1_000_000 {
        return Err(validation("Review input file is too large")
            .with_details(dict([("maximum_bytes", Value::Int(1_000_000.into()))])));
    }
    let raw = decode_json_bytes(&bytes)
        .ok_or_else(|| validation("Review input file must contain valid JSON"))?;
    let mut recipe = kassiber_pytext::loads_py(&raw)
        .map_err(|_| validation("Review input file must contain valid JSON"))?;
    if matches!(recipe, Value::Dict(_)) {
        if matches!(value(&recipe, "data"), Value::Dict(_)) {
            recipe = value(&recipe, "data").clone();
        }
        if let Some(inner) = get(&recipe, "recipe") {
            recipe = inner.clone();
        }
    }
    Ok(recipe)
}

fn decode_json_bytes(bytes: &[u8]) -> Option<kassiber_pyjson::PyStr> {
    let (bytes, width, little) = if bytes.starts_with(&[0xff, 0xfe, 0, 0]) {
        (&bytes[4..], 4, true)
    } else if bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        (&bytes[4..], 4, false)
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        (&bytes[2..], 2, true)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (&bytes[2..], 2, false)
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (&bytes[3..], 1, false)
    } else if bytes.len() >= 4 && bytes[..3] == [0, 0, 0] {
        (bytes, 4, false)
    } else if bytes.len() >= 4 && bytes[1..4] == [0, 0, 0] {
        (bytes, 4, true)
    } else if bytes.len() >= 2 && bytes[0] == 0 {
        (bytes, 2, false)
    } else if bytes.len() >= 2 && bytes[1] == 0 {
        (bytes, 2, true)
    } else {
        (bytes, 1, false)
    };
    if bytes.len() % width != 0 {
        return None;
    }
    let points: Vec<_> = if width == 1 {
        let mut remaining = bytes;
        let mut points = Vec::new();
        loop {
            match std::str::from_utf8(remaining) {
                Ok(text) => {
                    points.extend(text.chars().map(u32::from));
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    points.extend(
                        std::str::from_utf8(&remaining[..valid])
                            .ok()?
                            .chars()
                            .map(u32::from),
                    );
                    let tail = &remaining[valid..];
                    if tail.len() < 3
                        || tail[0] != 0xed
                        || !(0xa0..=0xbf).contains(&tail[1])
                        || !(0x80..=0xbf).contains(&tail[2])
                    {
                        return None;
                    }
                    points.push(
                        ((tail[0] as u32 & 15) << 12)
                            | ((tail[1] as u32 & 63) << 6)
                            | (tail[2] as u32 & 63),
                    );
                    remaining = &tail[3..];
                }
            }
        }
        points
    } else if width == 2 {
        char::decode_utf16(bytes.as_chunks::<2>().0.iter().map(|b| {
            if little {
                u16::from_le_bytes(*b)
            } else {
                u16::from_be_bytes(*b)
            }
        }))
        .map(|c| {
            c.map(u32::from)
                .unwrap_or_else(|e| u32::from(e.unpaired_surrogate()))
        })
        .collect()
    } else {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u32::from_le_bytes(*b)
                } else {
                    u32::from_be_bytes(*b)
                }
            })
            .collect()
    };
    kassiber_pyjson::PyStr::from_code_points(points).ok()
}

pub fn review_context(
    conn: &Connection,
    profile: &Value,
    target: &str,
    recipe: &Value,
) -> Result<Value> {
    let workspace = rows(
        conn,
        "SELECT * FROM workspaces WHERE id=?",
        [text(profile, "workspace_id")],
    )?
    .remove(0);
    let owns = conn.is_autocommit();
    if owns {
        conn.execute_batch("BEGIN").map_err(sql_error)?;
    }
    let result = context(conn, &workspace, profile, &s(target), recipe);
    kassiber_store::debug_assert_no_write(conn, owns, "source_funds_review_context");
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
