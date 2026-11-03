//! BIP329 labels are preserved evidence; matching only projects optional tags.
use super::*;
use kassiber_envelope::{py_str, truthy};
use kassiber_pyjson::Key;
use kassiber_pytext::{loads, parse_float};
use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue};
use std::path::{Path, PathBuf};

mod matching;

pub enum Bip329 {
    DesktopExport {
        wallets: Option<Vec<String>>,
        mode: String,
    },
    Preview {
        file: String,
    },
    Import {
        file: String,
        apply_ambiguous: bool,
        source: String,
    },
    List {
        cursor: Option<String>,
        limit: Option<i64>,
    },
    Export {
        file: String,
        wallets: Option<Vec<String>>,
        mode: String,
    },
}

fn put(row: &mut Value, key: &str, val: Value) {
    let Value::Dict(fields) = row else {
        unreachable!("object")
    };
    if let Some((_, old)) = fields.iter_mut().find(|(k, _)| *k == Key::from(key)) {
        *old = val;
    } else {
        fields.push((key.into(), val));
    }
}
fn object(raw: &Value) -> Value {
    if matches!(raw, Value::Dict(_)) {
        raw.clone()
    } else {
        dict::<&str>([])
    }
}
fn decoded(raw: &str) -> Value {
    object(&loads(raw).unwrap_or(Value::Null))
}
fn update(target: &mut Value, source: &Value) {
    if let Value::Dict(fields) = source {
        for (k, v) in fields {
            let Value::Dict(target) = target else {
                unreachable!("object")
            };
            if let Some((_, old)) = target.iter_mut().find(|(key, _)| key == k) {
                *old = v.clone();
            } else {
                target.push((k.clone(), v.clone()));
            }
        }
    }
}
fn dump(value: &Value, sorted: bool) -> Result<String> {
    dumps(
        value,
        &Options {
            sort_keys: sorted,
            ..Options::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}
fn absolute(file: &str) -> Result<PathBuf> {
    let path = Path::new(file);
    // abspath normalizes dot segments without resolving symlinks.
    let full = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| AppError::new("internal_error", e.to_string()))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for part in full.components() {
        match part {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            part => normalized.push(part.as_os_str()),
        }
    }
    Ok(normalized)
}
fn io(e: std::io::Error, file: &Path) -> AppError {
    let message = std::io::Error::from_raw_os_error(e.raw_os_error().unwrap_or(5)).to_string();
    let message = message.split(" (os error").next().unwrap_or(&message);
    AppError::new(
        "internal_error",
        format!(
            "[Errno {}] {message}: {}",
            e.raw_os_error().unwrap_or(5),
            kassiber_envelope::py_repr(&s(file.to_string_lossy().as_ref()))
        ),
    )
}
fn optional_string(v: &Value) -> Value {
    if matches!(v, Value::Null) {
        return Value::Null;
    }
    let val = py_str(v);
    let val = strip(&val);
    if val.is_empty() {
        Value::Null
    } else {
        s(val)
    }
}
fn normalized(raw: Value) -> Result<Value> {
    let Value::Dict(fields) = &raw else {
        return Err(AppError::new(
            "app_error",
            "BIP329 records must be JSON objects",
        ));
    };
    for key in ["type", "ref", "label", "origin"] {
        if matches!(get(&raw,key),Some(Value::Str(t)) if t.has_surrogates()) {
            return Err(AppError::new(
                "not_implemented",
                "BIP329 surrogate code points in standard string fields are not implemented",
            ));
        }
    }
    let py_text = |k| {
        let v = value(&raw, k);
        if truthy(&v) {
            py_str(&v)
        } else {
            String::new()
        }
    };
    let kind = py_text("type");
    let kind = strip(&kind);
    let reference = py_text("ref");
    let reference = strip(&reference);
    if !["tx", "addr", "pubkey", "input", "output", "xpub", "spscan"].contains(&kind) {
        return Err(AppError::new(
            "app_error",
            format!("Unsupported BIP329 record type '{kind}'"),
        ));
    }
    if reference.is_empty() {
        return Err(AppError::new(
            "app_error",
            "BIP329 records require a non-empty ref",
        ));
    }
    let spendable = value(&raw, "spendable");
    if !matches!(spendable, Value::Null | Value::Bool(_)) {
        return Err(AppError::new(
            "app_error",
            "BIP329 spendable must be a boolean when present",
        ));
    }
    if !matches!(spendable, Value::Null) && kind != "output" {
        return Err(AppError::new(
            "app_error",
            "BIP329 spendable is only valid for output records",
        ));
    }
    let data = Value::Dict(
        fields
            .iter()
            .filter(|(k, _)| {
                !["type", "ref", "label", "origin", "spendable"]
                    .iter()
                    .any(|s| *k == Key::from(*s))
            })
            .cloned()
            .collect(),
    );
    Ok(dict([
        ("type", s(kind)),
        ("ref", s(reference)),
        ("label", optional_string(&value(&raw, "label"))),
        ("origin", optional_string(&value(&raw, "origin"))),
        ("spendable", spendable),
        ("data", data),
    ]))
}
fn load(file: &str) -> Result<Vec<Value>> {
    let bytes = std::fs::read(file).map_err(|e| io(e, Path::new(file)))?;
    let source = String::from_utf8(bytes).map_err(|error| {
        let error_info = error.utf8_error();
        let start = error_info.valid_up_to();
        let end = start
            + error_info
                .error_len()
                .unwrap_or(error.as_bytes().len() - start);
        let what = if end == start + 1 {
            format!("byte 0x{:02x} in position {start}", error.as_bytes()[start])
        } else {
            format!("bytes in position {start}-{}", end - 1)
        };
        let reason = if error_info.error_len().is_none() {
            "unexpected end of data"
        } else if matches!(error.as_bytes()[start], 0xc2..=0xf4) {
            "invalid continuation byte"
        } else {
            "invalid start byte"
        };
        AppError::new(
            "internal_error",
            format!("'utf-8' codec can't decode {what}: {reason}"),
        )
    })?;
    // TextIO's universal-newline translation occurs before iteration.
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut records = Vec::new();
    for (i, raw) in source.split('\n').enumerate() {
        let line = strip(raw);
        if line.is_empty() {
            continue;
        }
        let raw = loads(line).map_err(|e| {
            if e.is_decode_error() {
                AppError::new(
                    "app_error",
                    format!("Invalid BIP329 JSON on line {}", i + 1),
                )
            } else {
                AppError::new("internal_error", e.to_string())
            }
        })?;
        records.push(normalized(raw)?);
    }
    Ok(records)
}
fn existing(conn: &Connection, profile: &str, record: &Value) -> Result<Option<Value>> {
    Ok(rows(
        conn,
        "SELECT * FROM bip329_labels WHERE profile_id=? AND record_type=? AND ref=? LIMIT 1",
        [profile, text(record, "type"), text(record, "ref")],
    )?
    .into_iter()
    .next())
}
fn conflicts(old: Option<&Value>, record: &Value) -> Vec<Value> {
    let Some(old) = old else {
        return vec![];
    };
    let mut conflicts = Vec::new();
    for field in ["label", "origin", "spendable"] {
        let incoming = value(record, field);
        let stored = value(old, field);
        let differs = if field == "spendable" {
            truthy(&incoming) != truthy(&stored)
        } else {
            incoming != stored
        };
        if incoming != Value::Null && stored != Value::Null && differs {
            conflicts.push(s(field));
        }
    }
    let stored = decoded(text(old, "data_json"));
    if let Value::Dict(fields) = value(record, "data") {
        for (key, incoming) in fields {
            let Key::Str(key) = key else {
                continue;
            };
            let old = match &stored {
                Value::Dict(fields) => fields
                    .iter()
                    .find(|(k, _)| *k == Key::Str(key.clone()))
                    .map(|(_, v)| v),
                _ => None,
            };
            if old.is_some_and(|old| !cursor::equivalent(old, &incoming)) {
                conflicts.push(Value::Str(
                    kassiber_pyjson::PyStr::from_code_points(
                        "data.".chars().map(u32::from).chain(key.code_points()),
                    )
                    .expect("existing code points"),
                ));
            }
        }
    }
    conflicts
}
fn effects(
    conn: &Connection,
    profile: &str,
    record: &Value,
    matched: &Value,
    txs: &[Value],
    last: bool,
    ambiguous: bool,
) -> Result<Vec<Value>> {
    let label = text(record, "label");
    if label.is_empty() {
        return Ok(vec![]);
    }
    if strip(label).chars().count() > 128 {
        return Ok(vec![dict([
            ("action", s("skipped_label_too_long")),
            (
                "reason",
                s("BIP329 labels projected to tags cannot exceed 128 characters."),
            ),
        ])]);
    }
    if !last {
        return Ok(vec![dict([
            ("action", s("skipped_duplicate")),
            ("reason", s("A later duplicate record wins.")),
        ])]);
    }
    let status = text(matched, "status");
    if status != "exact" && status != "ambiguous" {
        return Ok(vec![]);
    }
    let normalized = code(label)?;
    let old = rows(
        conn,
        "SELECT label FROM tags WHERE profile_id=? AND code=? LIMIT 1",
        [profile, &normalized],
    )?;
    let mut result = Vec::new();
    for tx in txs {
        let mut effect = dict([
            ("transaction_id", value(tx, "id")),
            ("wallet", s(text(tx, "wallet_label"))),
            ("wallet_id", s(text(tx, "wallet_id"))),
        ]);
        if status == "ambiguous" && !ambiguous {
            put(&mut effect, "action", s("skipped_ambiguous"));
        } else {
            let current = tags(conn, text(tx, "id"))?;
            let present = current.iter().any(|t| text(t, "code") == normalized);
            put(&mut effect, "tag", s(normalized.as_str()));
            put(&mut effect, "label", s(label));
            put(
                &mut effect,
                "action",
                s(if present { "unchanged" } else { "add" }),
            );
            if let Some(old) = old.first().filter(|old| text(old, "label") != label) {
                put(&mut effect, "conflict", s("tag_label"));
                put(&mut effect, "existing_label", value(old, "label"));
            }
        }
        result.push(effect);
    }
    Ok(result)
}
struct Planned {
    record: Value,
    matched: Value,
    effects: Vec<Value>,
}
fn plan(
    conn: &Connection,
    scope: &Scope<'_>,
    file: &str,
    ambiguous: bool,
) -> Result<(Value, Vec<Planned>)> {
    let records = load(file)?;
    let index = matching::Index::build(conn, scope.profile_id)?;
    let mut keys = BTreeMap::<(String, String), (usize, usize)>::new();
    for (i, record) in records.iter().enumerate() {
        let item = keys
            .entry((text(record, "type").into(), text(record, "ref").into()))
            .or_default();
        item.0 += 1;
        item.1 = i;
    }
    let mut counts = dict(
        [
            "exact",
            "ambiguous",
            "unmatched",
            "preserved",
            "conflicts",
            "duplicate_refs",
            "duplicate_records",
            "tag_additions",
            "tag_unchanged",
            "tag_skipped_ambiguous",
            "tag_skipped_duplicate",
            "tag_skipped_label_too_long",
        ]
        .map(|k| (k, Value::Int(0.into()))),
    );
    put(
        &mut counts,
        "duplicate_refs",
        Value::Int(keys.values().filter(|v| v.0 > 1).count().into()),
    );
    let mut increment = |key: &str| {
        let n = int(&counts, key);
        put(&mut counts, key, Value::Int((n + 1).into()));
    };
    let mut ordinal = BTreeMap::<(String, String), usize>::new();
    let mut public = Vec::new();
    let mut planned = Vec::new();
    for (i, record) in records.into_iter().enumerate() {
        let key = (
            text(&record, "type").to_owned(),
            text(&record, "ref").to_owned(),
        );
        let (count, last) = keys[&key];
        let number = ordinal.entry(key).or_default();
        *number += 1;
        let (matched, txs) = index.matches(conn, scope.profile_id, &record)?;
        let old = existing(conn, scope.profile_id, &record)?;
        let conflicts = conflicts(old.as_ref(), &record);
        let tag_effects = if text(&record, "type") == "tx" {
            effects(
                conn,
                scope.profile_id,
                &record,
                &matched,
                &txs,
                i == last,
                ambiguous,
            )?
        } else {
            vec![]
        };
        increment(text(&matched, "status"));
        if count > 1 {
            increment("duplicate_records");
        }
        if !conflicts.is_empty() {
            increment("conflicts");
        }
        for e in &tag_effects {
            increment(match text(e, "action") {
                "add" => "tag_additions",
                "unchanged" => "tag_unchanged",
                "skipped_ambiguous" => "tag_skipped_ambiguous",
                "skipped_duplicate" => "tag_skipped_duplicate",
                _ => "tag_skipped_label_too_long",
            });
        }
        let redacted = ["pubkey", "xpub", "spscan"].contains(&text(&record, "type"));
        let reference = text(&record, "ref");
        let preview = if !redacted {
            reference.to_owned()
        } else if reference.chars().count() <= 16 {
            "[redacted]".into()
        } else {
            format!(
                "{}...{}",
                reference.chars().take(8).collect::<String>(),
                reference
                    .chars()
                    .skip(reference.chars().count() - 8)
                    .collect::<String>()
            )
        };
        let old = old.as_ref().map_or(Value::Null, |old| {
            dict([
                ("label", value(old, "label")),
                ("origin", value(old, "origin")),
                (
                    "spendable",
                    if value(old, "spendable") == Value::Null {
                        Value::Null
                    } else {
                        Value::Bool(truthy(&value(old, "spendable")))
                    },
                ),
            ])
        });
        public.push(dict([
            ("line", Value::Int((i + 1).into())),
            ("type", value(&record, "type")),
            ("ref", s(preview.as_str())),
            ("ref_preview", s(preview)),
            ("ref_redacted", Value::Bool(redacted)),
            ("label", s(text(&record, "label"))),
            ("origin", s(text(&record, "origin"))),
            ("match_status", value(&matched, "status")),
            ("match_confidence", value(&matched, "confidence")),
            ("wallet_ids", value(&matched, "wallet_ids")),
            ("wallets", value(&matched, "wallets")),
            ("match_source", value(&matched, "match_source")),
            ("duplicate", Value::Bool(count > 1)),
            (
                "duplicate_ordinal",
                if count > 1 {
                    Value::Int((*number).into())
                } else {
                    Value::Null
                },
            ),
            ("conflicts", Value::List(conflicts)),
            ("existing", old),
            ("tag_effects", Value::List(tag_effects.clone())),
        ]));
        planned.push(Planned {
            record,
            matched,
            effects: tag_effects,
        });
    }
    Ok((
        dict([
            ("file", s(absolute(file)?.to_string_lossy().as_ref())),
            ("records", Value::Int(planned.len().into())),
            ("counts", counts),
            ("warnings", Value::List(index.warnings)),
            (
                "apply_policy",
                s(if ambiguous {
                    "all_matches"
                } else {
                    "exact_only"
                }),
            ),
            ("rows", Value::List(public)),
        ]),
        planned,
    ))
}
fn imported_data(record: &Value, matched: &Value) -> Value {
    let mut data = value(record, "data");
    let imported = value(&data, "kassiber");
    let mut kassiber = object(&imported);
    if imported != Value::Null && !matches!(imported, Value::Dict(_)) {
        put(&mut kassiber, "imported_value", imported);
    }
    put(&mut kassiber, "wallet_match", matched.clone());
    put(&mut data, "kassiber", kassiber);
    data
}
fn upsert(conn: &Connection, scope: &Scope<'_>, row: &Planned) -> Result<bool> {
    let data = imported_data(&row.record, &row.matched);
    let old = existing(conn, scope.profile_id, &row.record)?;
    if let Some(old) = old {
        let effective = |field| {
            let v = value(&row.record, field);
            if v == Value::Null {
                value(&old, field)
            } else {
                v
            }
        };
        let mut merged = decoded(text(&old, "data_json"));
        let mut kassiber = value(&merged, "kassiber");
        update(&mut merged, &data);
        if matches!(kassiber, Value::Dict(_)) {
            update(&mut kassiber, &value(&data, "kassiber"));
            put(&mut merged, "kassiber", kassiber);
        }
        conn.execute("UPDATE bip329_labels SET wallet_id=NULL,label=?,origin=?,spendable=?,data_json=? WHERE id=?",params![cursor::bind(&effective("label"))?,cursor::bind(&effective("origin"))?,cursor::bind(&effective("spendable"))?,dump(&merged,true)?,text(&old,"id")]).map_err(error)?;
        Ok(false)
    } else {
        conn.execute("INSERT INTO bip329_labels(id,workspace_id,profile_id,wallet_id,record_type,ref,label,origin,spendable,data_json,created_at) VALUES(?,?,?,NULL,?,?,?,?,?,?,?)",params![uuid(conn)?,scope.workspace_id,scope.profile_id,text(&row.record,"type"),text(&row.record,"ref"),cursor::bind(&value(&row.record,"label"))?,cursor::bind(&value(&row.record,"origin"))?,cursor::bind(&value(&row.record,"spendable"))?,dump(&data,true)?,now(conn)?]).map_err(error)?;
        Ok(true)
    }
}
fn import(
    conn: &Connection,
    scope: &Scope<'_>,
    file: &str,
    ambiguous: bool,
    source: &str,
) -> Result<Value> {
    let (mut preview, planned) = plan(conn, scope, file, ambiguous)?;
    if let Value::Dict(fields) = &mut preview {
        fields.retain(|(k, _)| *k != Key::from("rows"));
    }
    let (imported, created, added) = atomic(conn, || {
        let mut imported = 0;
        let mut created = 0;
        let mut added = 0;
        for row in &planned {
            imported += usize::from(upsert(conn, scope, row)?);
        }
        for row in &planned {
            for effect in &row.effects {
                if text(effect, "action") != "add" {
                    continue;
                }
                let label = strip(text(effect, "label"));
                if label.is_empty() {
                    continue;
                }
                let tx = transaction(conn, scope.profile_id, text(effect, "transaction_id"))?;
                let normalized = code(label)?;
                created += usize::from(
                    rows(
                        conn,
                        "SELECT id FROM tags WHERE profile_id=? AND code=?",
                        [scope.profile_id, &normalized],
                    )?
                    .is_empty(),
                );
                ensure_tag(conn, scope, label)?;
                let current = tags(conn, text(&tx, "id"))?;
                if current.iter().any(|t| text(t, "code") == normalized) {
                    continue;
                }
                let before = Value::List(
                    canonical_tags(current.iter().map(|t| text(t, "label").to_owned()))
                        .into_iter()
                        .map(s)
                        .collect(),
                );
                let mut labels = current
                    .iter()
                    .map(|t| text(t, "label").to_owned())
                    .collect::<Vec<_>>();
                labels.push(label.into());
                let after = Value::List(canonical_tags(labels).into_iter().map(s).collect());
                // Python's grouped writer canonicalizes labels before replacing tag links.
                conn.execute(
                    "DELETE FROM transaction_tags WHERE transaction_id=?",
                    [text(&tx, "id")],
                )
                .map_err(error)?;
                if let Value::List(labels) = &after {
                    for label in labels {
                        let tag = ensure_tag(conn, scope, string(label).unwrap_or_default())?;
                        conn.execute("INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",[text(&tx,"id"),&tag]).map_err(error)?;
                    }
                }
                append_fields(
                    conn,
                    scope,
                    &tx,
                    &[("tags", before, after)],
                    source,
                    Some(&format!(
                        "Imported BIP329 label for {}",
                        text(&row.record, "ref")
                    )),
                )?;
                added += 1;
            }
        }
        if !planned.is_empty() {
            invalidate(conn, scope.profile_id)?;
        }
        Ok((imported, created, added))
    })?;
    Ok(dict([
        ("file", value(&preview, "file")),
        ("imported", Value::Int(imported.into())),
        ("updated", Value::Int((planned.len() - imported).into())),
        ("records", Value::Int(planned.len().into())),
        ("transaction_tags_created", Value::Int(created.into())),
        ("transaction_tags_added", Value::Int(added.into())),
        ("preview", preview),
    ]))
}
fn list(
    conn: &Connection,
    scope: &Scope<'_>,
    raw: Option<&str>,
    limit: Option<i64>,
) -> Result<Value> {
    let limit = limit.unwrap_or(100);
    if limit <= 0 {
        return Err(AppError::new("validation", "--limit must be positive"));
    }
    if limit > 1000 {
        return Err(AppError::new("validation", "--limit cannot exceed 1000")
            .with_hint("Use a smaller --limit; max page size is 1000."));
    }
    let filters = dict([
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
    ]);
    let boundary = cursor::decode(raw, &filters, &["created_at", "id"], true)?;
    let mut args = vec![SqlValue::Text(scope.profile_id.into())];
    let clause = if let Some(row) = boundary {
        args.push(cursor::bind(&value(&row, "created_at"))?);
        args.push(cursor::bind(&value(&row, "id"))?);
        "AND (created_at,id)<(?,?)"
    } else {
        ""
    };
    args.push(SqlValue::Integer(limit + 1));
    let mut page=rows(conn,&format!("SELECT id,record_type AS type,ref,COALESCE(label,'') AS label,COALESCE(origin,'') AS origin,CASE WHEN spendable IS NULL THEN '' WHEN spendable=1 THEN 'true' ELSE 'false' END AS spendable,created_at FROM bip329_labels WHERE profile_id=? {clause} ORDER BY created_at DESC,id DESC LIMIT ?"),params_from_iter(args))?;
    let more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let next = if more {
        cursor::encode(page.last().unwrap(), &filters, &["created_at", "id"])?
    } else {
        Value::Null
    };
    for row in &mut page {
        if let Value::Dict(fields) = row {
            fields.retain(|(k, _)| *k != Key::from("id"));
        }
    }
    Ok(dict([
        ("records", Value::List(page)),
        ("next_cursor", next),
        ("has_more", Value::Bool(more)),
        ("limit", Value::Int(limit.into())),
    ]))
}
fn stored(conn: &Connection, profile: &str) -> Result<Vec<Value>> {
    Ok(rows(conn,"SELECT record_type AS type,ref,label,origin,spendable,data_json FROM bip329_labels WHERE profile_id=? ORDER BY created_at ASC,record_type ASC,ref ASC,id ASC",[profile])?.into_iter().map(|row|dict([("type",value(&row,"type")),("ref",value(&row,"ref")),("label",value(&row,"label")),("origin",value(&row,"origin")),("spendable",if value(&row,"spendable")==Value::Null{Value::Null}else{Value::Bool(truthy(&value(&row,"spendable")))}),("data",decoded(text(&row,"data_json")))])).collect())
}
fn clip(raw: &str, max: usize) -> String {
    let joined = raw
        .split(|c| kassiber_pytext::is_space(c as u32))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() <= max {
        joined
    } else {
        format!(
            "{}...",
            joined
                .chars()
                .take(max - 3)
                .collect::<String>()
                .trim_end_matches(|c| kassiber_pytext::is_space(c as u32))
        )
    }
}
fn synthesized(conn: &Connection, profile: &str, wallet: Option<&str>) -> Result<Vec<Value>> {
    let txs=rows(conn,"SELECT t.*,w.label AS wallet_label FROM transactions t LEFT JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.external_id IS NOT NULL AND t.external_id!='' AND (? IS NULL OR t.wallet_id=?) ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC",params![profile,wallet,wallet])?;
    let mut result = Vec::new();
    for tx in txs {
        let labels = tags(conn, text(&tx, "id"))?
            .iter()
            .map(|t| text(t, "label").to_owned())
            .collect::<Vec<_>>();
        let mut parts = Vec::new();
        if truthy(&value(&tx, "counterparty")) {
            parts.push(clip(text(&tx, "counterparty"), 72));
        }
        if !labels.is_empty() {
            parts.push(format!("tags: {}", clip(&labels.join(", "), 90)));
        }
        if truthy(&value(&tx, "review_status")) {
            parts.push(format!("review: {}", clip(text(&tx, "review_status"), 32)));
        }
        if value(&tx, "fiat_value") != Value::Null && truthy(&value(&tx, "fiat_currency")) {
            parts.push(format!(
                "FMV {} {}",
                text(&tx, "fiat_currency"),
                clip(
                    &if truthy(&value(&tx, "fiat_value")) {
                        py_str(&value(&tx, "fiat_value"))
                    } else {
                        String::new()
                    },
                    32
                )
            ));
        }
        if truthy(&value(&tx, "note")) {
            parts.push(format!("note: {}", clip(text(&tx, "note"), 90)));
        }
        if parts.is_empty() {
            continue;
        }
        let mut data = dict([(
            "kassiber",
            dict([
                ("source", s("synthesized")),
                ("transaction_id", value(&tx, "id")),
                ("wallet_id", value(&tx, "wallet_id")),
                ("wallet", value(&tx, "wallet_label")),
                ("review_status", value(&tx, "review_status")),
                ("tags", Value::List(labels.into_iter().map(s).collect())),
            ]),
        )]);
        let time = if truthy(&value(&tx, "confirmed_at")) {
            value(&tx, "confirmed_at")
        } else {
            value(&tx, "occurred_at")
        };
        if truthy(&time) {
            put(&mut data, "time", time);
        }
        if ["BTC", "XBT"].contains(&text(&tx, "asset").to_uppercase().as_str()) {
            for (column, key) in [("amount", "value"), ("fee", "fee")] {
                if let Some(Value::Int(n)) = get(&tx, column) {
                    if n % 1000i64 == 0.into() && (column != "fee" || n > &0.into()) {
                        let mut n = n / 1000i64;
                        if column == "amount" && text(&tx, "direction") == "outbound" {
                            n = -n;
                        }
                        put(&mut data, key, Value::Int(n));
                    }
                }
            }
        }
        let rate = if truthy(&value(&tx, "fiat_rate_exact")) {
            value(&tx, "fiat_rate_exact")
        } else {
            value(&tx, "fiat_rate")
        };
        let rate = match rate {
            Value::Float(n) => Some(n),
            Value::Int(n) => n.to_string().parse().ok(),
            Value::Str(t) => t.as_str().and_then(|s| parse_float(s).ok()),
            _ => None,
        };
        if let Some(rate) = rate.filter(|_| truthy(&value(&tx, "fiat_currency"))) {
            put(
                &mut data,
                "rate",
                dict([(
                    text(&tx, "fiat_currency").to_uppercase(),
                    Value::Float(rate),
                )]),
            );
        }
        result.push(dict([
            ("type", s("tx")),
            ("ref", value(&tx, "external_id")),
            ("label", s(clip(&parts.join(" | "), 255))),
            ("origin", s("kassiber")),
            ("data", data),
        ]));
    }
    Ok(result)
}
fn select(
    conn: &Connection,
    scope: &Scope<'_>,
    refs: &Option<Vec<String>>,
    mode: &str,
) -> Result<Selected> {
    let mode = lower(strip(if mode.is_empty() { "stored" } else { mode }));
    if !["stored", "synthesized", "all"].contains(&mode.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported BIP329 export mode '{mode}'"),
        )
        .with_hint("Use stored, synthesized, or all."));
    }
    if refs.as_ref().is_some_and(Vec::is_empty) {
        return Err(AppError::new(
            "validation",
            "Select at least one wallet to export",
        ));
    }
    let mut wallets = Vec::new();
    if let Some(refs) = refs {
        for reference in refs {
            let id = records::wallet(conn, scope.profile_id, Some(reference))?;
            let wallet = rows(
                conn,
                "SELECT id,label FROM wallets WHERE profile_id=? AND id=?",
                [scope.profile_id, &id],
            )?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new("not_found", format!("Wallet '{reference}' not found")))?;
            wallets.push(wallet);
        }
    }
    let scopes = if refs.is_some() {
        wallets
            .iter()
            .map(|w| Some(text(w, "id")))
            .collect::<Vec<_>>()
    } else {
        vec![None]
    };
    let stored = if mode != "synthesized" {
        stored(conn, scope.profile_id)?
    } else {
        vec![]
    };
    let index = if mode != "synthesized" && refs.is_some() {
        Some(matching::Index::build(conn, scope.profile_id)?)
    } else {
        None
    };
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    let mut stored_count = 0;
    let mut synthesized_count = 0;
    for wallet in scopes {
        if mode != "synthesized" {
            for record in &stored {
                if let Some(wallet) = wallet {
                    let (matched, _) =
                        index
                            .as_ref()
                            .unwrap()
                            .matches(conn, scope.profile_id, record)?;
                    if text(&matched, "status") != "exact"
                        || !matches!(get(&matched,"wallet_ids"),Some(Value::List(ids)) if ids.contains(&s(wallet)))
                    {
                        continue;
                    }
                }
                if seen.insert((
                    text(record, "type").to_owned(),
                    text(record, "ref").to_owned(),
                )) {
                    output.push(record.clone());
                    stored_count += 1;
                }
            }
        }
        if mode != "stored" {
            for record in synthesized(conn, scope.profile_id, wallet)? {
                if seen.insert((
                    text(&record, "type").to_owned(),
                    text(&record, "ref").to_owned(),
                )) {
                    output.push(record);
                    synthesized_count += 1;
                }
            }
        }
    }
    Ok(Selected {
        mode,
        wallets,
        output,
        stored_count,
        synthesized_count,
    })
}

struct Selected {
    mode: String,
    wallets: Vec<Value>,
    output: Vec<Value>,
    stored_count: usize,
    synthesized_count: usize,
}

/// The records a BIP329 export writes, for the export dialog's preview.
pub fn export_records(
    conn: &Connection,
    scope: &Scope<'_>,
    refs: &Option<Vec<String>>,
    mode: &str,
) -> Result<Vec<Value>> {
    Ok(select(conn, scope, refs, mode)?.output)
}

fn export(
    conn: &Connection,
    scope: &Scope<'_>,
    file: &str,
    refs: &Option<Vec<String>>,
    mode: &str,
) -> Result<Value> {
    let Selected {
        mode,
        wallets,
        output,
        stored_count,
        synthesized_count,
    } = select(conn, scope, refs, mode)?;
    let mut bytes = String::new();
    for record in &output {
        let mut payload = dict([
            ("type", value(record, "type")),
            ("ref", value(record, "ref")),
        ]);
        for k in ["label", "origin", "spendable"] {
            let v = value(record, k);
            if v != Value::Null {
                put(&mut payload, k, v);
            }
        }
        update(&mut payload, &value(record, "data"));
        bytes.push_str(&dump(&payload, false)?);
        bytes.push('\n');
    }
    let path = absolute(file)?;
    std::fs::write(&path, bytes).map_err(|e| io(e, &path))?;
    Ok(dict([
        ("file", s(path.to_string_lossy().as_ref())),
        ("exported", Value::Int(output.len().into())),
        ("mode", s(mode)),
        (
            "wallet",
            s(wallets
                .iter()
                .map(|w| text(w, "label"))
                .collect::<Vec<_>>()
                .join(", ")),
        ),
        ("exported_stored", Value::Int(stored_count.into())),
        ("exported_synthesized", Value::Int(synthesized_count.into())),
    ]))
}

pub(super) fn execute(conn: &Connection, scope: &Scope<'_>, command: &Bip329) -> Result<Value> {
    match command {
        Bip329::DesktopExport { .. } => Err(AppError::new(
            "not_implemented",
            "managed BIP329 export requires the book paths adapter",
        )),
        Bip329::Preview { file } => plan(conn, scope, file, false).map(|v| v.0),
        Bip329::Import {
            file,
            apply_ambiguous,
            source,
        } => import(conn, scope, file, *apply_ambiguous, source),
        Bip329::List { cursor, limit } => list(conn, scope, cursor.as_deref(), *limit),
        Bip329::Export {
            file,
            wallets,
            mode,
        } => export(conn, scope, file, wallets, mode),
    }
}

/// Only the admitted book adapter supplies this managed root.
pub fn managed_export(
    conn: &Connection,
    scope: &Scope<'_>,
    exports_root: &Path,
    wallets: &Option<Vec<String>>,
    mode: &str,
) -> Result<Value> {
    let root = exports_root.join("reports");
    std::fs::create_dir_all(&root).map_err(|e| io(e, &root))?;
    let timestamp: String = conn
        .query_row("SELECT strftime('%Y%m%d-%H%M%S','now')", [], |r| r.get(0))
        .map_err(error)?;
    let base = format!("kassiber-bip329-labels-{timestamp}");
    let mut path = root.join(format!("{base}.jsonl"));
    let mut counter = 2;
    while path.exists() {
        path = root.join(format!("{base}-{counter}.jsonl"));
        counter += 1;
    }
    let mut payload = export(
        conn,
        scope,
        path.to_str()
            .ok_or_else(|| AppError::new("validation", "export path is not valid UTF-8"))?,
        wallets,
        mode,
    )?;
    put(&mut payload, "format", s("jsonl"));
    put(&mut payload, "scope", s("bip329"));
    put(
        &mut payload,
        "filename",
        s(path.file_name().unwrap().to_string_lossy().as_ref()),
    );
    Ok(payload)
}
