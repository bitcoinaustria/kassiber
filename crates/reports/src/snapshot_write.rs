//! Phase-4 append-only markers. A saved reference copies sealed facts, never journals.
use super::*;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::params;

fn invalid(message: impl Into<String>, field: Option<&str>) -> AppError {
    let error = AppError::new("filed_report_snapshot_validation", message);
    match field {
        Some(field) => error.with_details(dict([("field", s(field))])),
        None => error,
    }
}
fn stripped_py(value: &Value) -> kassiber_pyjson::PyStr {
    let raw = if !truthy(value) {
        "".into()
    } else if let Value::Str(t) = value {
        t.clone()
    } else {
        py_str(value).into()
    };
    kassiber_pytext::strip_py(&raw)
}
fn stripped(value: &Value) -> String {
    stripped_py(value)
        .code_points()
        .map(|c| char::from_u32(c).unwrap_or('�'))
        .collect()
}
pub(super) fn token(value: &Value, field: &str) -> Result<String> {
    let text = stripped(value).to_lowercase();
    if text.is_empty()
        || text.len() > 64
        || !text.as_bytes()[0].is_ascii_alphanumeric()
        || !text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
    {
        return Err(invalid(
            format!("{field} must be a bounded lowercase token"),
            Some(field),
        ));
    }
    Ok(text)
}
pub(super) fn year(value: &Value, field: &str) -> Result<i64> {
    let parsed = match value {
        Value::Int(n) => Some(n.clone()),
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc())).ok()
        }
        Value::Str(t) => t.as_str().and_then(|s| kassiber_pytext::parse_int(s).ok()),
        _ => None,
    }
    .ok_or_else(|| invalid(format!("{field} must be a calendar year"), Some(field)))?;
    if parsed < 1900.into() || parsed > 9999.into() {
        return Err(invalid(
            format!("{field} must be between 1900 and 9999"),
            Some(field),
        ));
    }
    Ok(i64::try_from(parsed).unwrap())
}
fn fields<'a>(
    v: &'a Value,
    message: &str,
    field: &str,
    max: usize,
    allowed: Option<&[&str]>,
) -> Result<&'a [(kassiber_pyjson::Key, Value)]> {
    if let Value::Dict(items) = v {
        if items.len() <= max
            && allowed.is_none_or(|allowed| {
                items
                    .iter()
                    .all(|(key, _)| allowed.iter().any(|a| key == &(*a).into()))
            })
        {
            return Ok(items);
        }
    }
    Err(invalid(message, Some(field)))
}
pub(super) fn classifications(v: &Value) -> Result<Value> {
    if matches!(v, Value::Null) {
        return Ok(dict::<&str>([]));
    }
    let items = fields(
        v,
        "classification_summary must contain at most 32 categories",
        "classification_summary",
        32,
        None,
    )?;
    let mut result = dict::<&str>([]);
    for (key, summary) in items {
        let category = token(
            &match key {
                kassiber_pyjson::Key::Str(t) => Value::Str(t.clone()),
                _ => unreachable!("JSON keys are strings"),
            },
            "classification_summary.category",
        )?;
        if !matches!(summary, Value::Dict(_)) {
            return Err(invalid(
                "each classification summary must contain count and amount_msat",
                Some("classification_summary"),
            ));
        }
        fields(
            summary,
            "classification summary contains unsupported fields",
            "classification_summary",
            usize::MAX,
            Some(&["count", "amount_msat"]),
        )?;
        let zero = Value::Int(0.into());
        let count = get(summary, "count").unwrap_or(&zero);
        let amount = get(summary, "amount_msat").unwrap_or(&zero);
        if !matches!(count, Value::Int(n) if n >= &0.into())
            || !matches!(amount, Value::Int(n) if n >= &0.into())
        {
            return Err(invalid(
                "classification count and amount_msat must be non-negative integers",
                Some("classification_summary"),
            ));
        }
        replace(
            &mut result,
            &category,
            dict([("count", count.clone()), ("amount_msat", amount.clone())]),
        );
    }
    Ok(result)
}
#[cfg(test)]
fn key_string(k: &kassiber_pyjson::Key) -> String {
    match k {
        kassiber_pyjson::Key::Str(s) => py_str(&Value::Str(s.clone())),
        _ => unreachable!("JSON keys are strings"),
    }
}
pub(super) fn gains(v: &Value) -> Result<Value> {
    if matches!(v, Value::Null) {
        return Ok(dict::<&str>([]));
    }
    fields(
        v,
        "gain_summary contains unsupported fields",
        "gain_summary",
        usize::MAX,
        Some(&[
            "fiat_currency",
            "proceeds_exact",
            "cost_basis_exact",
            "gain_loss_exact",
            "status",
        ]),
    )?;
    let mut result = dict::<&str>([]);
    if let Some(raw) = get(v, "fiat_currency") {
        let currency = stripped(raw).to_uppercase();
        if !(3..=12).contains(&currency.len()) || !currency.bytes().all(|b| b.is_ascii_uppercase())
        {
            return Err(invalid(
                "gain_summary.fiat_currency is invalid",
                Some("gain_summary.fiat_currency"),
            ));
        }
        replace(&mut result, "fiat_currency", s(currency));
    }
    for field in ["proceeds_exact", "cost_basis_exact", "gain_loss_exact"] {
        if let Some(raw) = get(v, field) {
            let text = stripped(raw);
            let name = format!("gain_summary.{field}");
            let special = text
                .strip_prefix(['+', '-'])
                .unwrap_or(&text)
                .replace('_', "")
                .to_ascii_lowercase();
            if ["infinity", "inf"].contains(&special.as_str())
                || ["nan", "snan"].iter().any(|prefix| {
                    special.strip_prefix(prefix).is_some_and(|rest| {
                        rest.chars()
                            .all(|c| kassiber_pytext::decimal_value(u32::from(c)).is_some())
                    })
                })
            {
                return Err(invalid(format!("{name} must be finite"), Some(&name)));
            }
            if text.parse::<Decimal>().is_err() {
                return Err(invalid(
                    format!("{name} must be an exact decimal string"),
                    Some(&name),
                ));
            }
            replace(&mut result, field, s(text));
        }
    }
    if let Some(raw) = get(v, "status") {
        let status = stripped(raw).to_lowercase();
        if !["final", "pending_journal_rebuild"].contains(&status.as_str()) {
            return Err(invalid(
                "gain_summary.status is invalid",
                Some("gain_summary.status"),
            ));
        }
        replace(&mut result, "status", s(status));
    }
    Ok(result)
}
pub(super) fn scope(v: &Value) -> Result<Value> {
    if matches!(v, Value::Null) {
        return Ok(dict::<&str>([]));
    }
    fields(
        v,
        "report_scope contains unsupported fields",
        "report_scope",
        usize::MAX,
        Some(&["wallet_ids", "occurred_at_start", "occurred_at_end"]),
    )?;
    let mut result = dict::<&str>([]);
    if let Some(raw) = get(v, "wallet_ids") {
        let Value::List(wallets) = raw else {
            return Err(invalid(
                "report_scope.wallet_ids must contain at most 256 ids",
                Some("report_scope.wallet_ids"),
            ));
        };
        if wallets.len() > 256 {
            return Err(invalid(
                "report_scope.wallet_ids must contain at most 256 ids",
                Some("report_scope.wallet_ids"),
            ));
        }
        let mut wallets: Vec<_> = wallets.iter().map(stripped_py).collect();
        wallets.sort();
        wallets.dedup();
        if wallets
            .iter()
            .any(|w| w.code_points().count() == 0 || w.code_points().count() > 128)
        {
            return Err(invalid(
                "report_scope.wallet_ids is invalid",
                Some("report_scope.wallet_ids"),
            ));
        }
        if !wallets.is_empty() {
            replace(
                &mut result,
                "wallet_ids",
                Value::List(wallets.into_iter().map(Value::Str).collect()),
            );
        }
    }
    for field in ["occurred_at_start", "occurred_at_end"] {
        if let Some(raw) = get(v, field).filter(|v| !matches!(v, Value::Null) && **v != s("")) {
            // Python checks the shape only here, deliberately not calendar validity.
            let timestamp = stripped(raw);
            let chars: Vec<_> = timestamp.chars().collect();
            let pattern = "dddd-dd-ddTdd:dd:ddZ";
            if chars.len() != pattern.len()
                || !chars.iter().zip(pattern.chars()).all(|(c, p)| {
                    if p == 'd' {
                        kassiber_pytext::parse_int(&c.to_string()).is_ok()
                    } else {
                        *c == p
                    }
                })
            {
                let name = format!("report_scope.{field}");
                return Err(invalid(
                    format!("{name} must use whole-second RFC3339 UTC"),
                    Some(&name),
                ));
            }
            replace(&mut result, field, s(timestamp));
        }
    }
    if truthy(value(&result, "occurred_at_start"))
        && truthy(value(&result, "occurred_at_end"))
        && text(&result, "occurred_at_end") < text(&result, "occurred_at_start")
    {
        return Err(invalid(
            "report_scope.occurred_at_end must not precede occurred_at_start",
            Some("report_scope.occurred_at_end"),
        ));
    }
    Ok(result)
}
pub(super) fn json(v: &Value) -> Result<String> {
    kassiber_pyjson::dumps(
        v,
        &kassiber_pyjson::Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}

pub(super) fn create_cli(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let mut args = args.clone();
    for field in ["classification_summary", "gain_summary", "report_scope"] {
        let raw = value(&args, &format!("{field}_json"));
        let parsed = if matches!(raw, Value::Null) {
            Value::Null
        } else {
            kassiber_pytext::loads(&py_str(raw)).map_err(|error| {
                if error.is_decode_error() {
                    AppError::new(
                        "validation",
                        "filed snapshot summaries must be JSON objects",
                    )
                } else {
                    AppError::new("internal_error", error.to_string())
                }
            })?
        };
        replace(&mut args, field, parsed);
    }
    if ["classification_summary", "gain_summary", "report_scope"]
        .iter()
        .any(|field| !matches!(value(&args, field), Value::Null | Value::Dict(_)))
    {
        return Err(AppError::new(
            "validation",
            "filed snapshot summaries must be JSON objects",
        ));
    }
    let state = value(&args, "state").clone();
    replace(&mut args, "report_state", state);
    replace(&mut args, "authored_source", s("cli"));
    create(
        conn,
        &text(workspace, "id"),
        &text(profile, "id"),
        &args,
        sources,
    )
}

pub(super) fn create(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    create_inner(conn, workspace_id, profile_id, args, None, sources)
}

/// `create_filed_report_snapshot(..., snapshot_id=id, created_at=timestamp)`.
pub(super) fn create_with(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    args: &Value,
    id: &str,
    timestamp: &str,
    sources: &dyn Sources,
) -> Result<Value> {
    create_inner(
        conn,
        workspace_id,
        profile_id,
        args,
        Some((id, timestamp)),
        sources,
    )
}

fn create_inner(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    args: &Value,
    identity: Option<(&str, &str)>,
    sources: &dyn Sources,
) -> Result<Value> {
    let mut args = args.clone();
    let saved = if !matches!(value(&args, "saved_snapshot_id"), Value::Null) {
        let raw = value(&args, "saved_snapshot_id");
        if !matches!(raw, Value::Str(t) if t.as_str().is_some_and(|t| !kassiber_pytext::strip(t).is_empty()))
        {
            return Err(invalid(
                "saved_snapshot_id must identify a captured local export",
                None,
            ));
        }
        let saved = super::snapshots::get_snapshot(conn, &py_str(raw), profile_id)?;
        let captured: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM filed_report_chain_captures WHERE snapshot_id=? AND profile_id=?)",params![py_str(raw),profile_id],|r|r.get(0)).map_err(sql_error)?;
        if text(&saved, "report_state") != "saved"
            || text(&saved, "workspace_id") != workspace_id
            || !captured
        {
            return Err(invalid(
                "The reference must be a locally captured saved export from this book",
                None,
            ));
        }
        if kassiber_pytext::strip(&py_str(value(&args, "report_state"))).to_lowercase() != "filed" {
            return Err(invalid(
                "A saved export reference is only valid for an explicit filed marker",
                None,
            ));
        }
        for field in [
            "report_kind",
            "period_start_year",
            "period_end_year",
            "content_sha256",
            "classification_summary",
            "gain_summary",
            "report_scope",
        ] {
            if matches!(value(&args, field), Value::Null) {
                replace(&mut args, field, value(&saved, field).clone());
            }
        }
        Some(saved)
    } else {
        None
    };
    let kind = token(value(&args, "report_kind"), "report_kind")?;
    let state = stripped(value(&args, "report_state")).to_lowercase();
    if !["saved", "filed"].contains(&state.as_str()) {
        return Err(invalid(
            "report_state must be saved or filed",
            Some("report_state"),
        ));
    }
    let start = year(value(&args, "period_start_year"), "period_start_year")?;
    let end = year(
        if matches!(value(&args, "period_end_year"), Value::Null) {
            value(&args, "period_start_year")
        } else {
            value(&args, "period_end_year")
        },
        "period_end_year",
    )?;
    if end < start {
        return Err(invalid(
            "period_end_year must not precede period_start_year",
            Some("period_end_year"),
        ));
    }
    let digest = stripped(value(&args, "content_sha256")).to_lowercase();
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid(
            "content_sha256 must be a lowercase SHA-256 digest",
            Some("content_sha256"),
        ));
    }
    let source = get(&args, "authored_source")
        .map(stripped)
        .unwrap_or_else(|| "user".into())
        .to_lowercase();
    if !["user", "cli", "gui", "ai_tool"].contains(&source.as_str()) {
        return Err(invalid(
            "authored_source is invalid",
            Some("authored_source"),
        ));
    }
    let note = match value(&args, "notes") {
        Value::Null => None,
        v if *v == s("") => None,
        v => Some(kassiber_pytext::strip(&py_str(v)).to_string()),
    };
    if note.as_ref().is_some_and(|n| n.chars().count() > 1000) {
        return Err(invalid("notes is too long", Some("notes")));
    }
    let matching: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM profiles WHERE id=? AND workspace_id=?)",
            params![profile_id, workspace_id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if !matching {
        return Err(AppError::new(
            "scope_mismatch",
            "filed report snapshot targets another book",
        ));
    }
    let (id, timestamp) = match identity {
        Some((id, timestamp)) => (id.to_owned(), timestamp.to_owned()),
        None => (sources.uuid(conn)?, sources.now(conn)?),
    };
    let classifications = classifications(value(&args, "classification_summary"))?;
    let gains = gains(value(&args, "gain_summary"))?;
    let scope = scope(value(&args, "report_scope"))?;
    if let Some(saved) = &saved {
        if kind != text(saved, "report_kind")
            || start != int(saved, "period_start_year")
            || end != int(saved, "period_end_year")
            || digest != text(saved, "content_sha256")
            || !same_object(&classifications, value(saved, "classification_summary"))?
            || !same_object(&gains, value(saved, "gain_summary"))?
            || !same_object(&scope, value(saved, "report_scope"))?
        {
            return Err(invalid(
                "Filed marker identity must match the referenced saved export",
                None,
            ));
        }
    }
    kassiber_store::begin_write_savepoint(conn, "filed_snapshot_create").map_err(sql_error)?;
    let result = (|| {
        conn.execute("INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,classification_summary_json,gain_summary_json,report_scope_json,authored_source,notes,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",params![id,workspace_id,profile_id,kind,state,start,end,digest,json(&classifications)?,json(&gains)?,json(&scope)?,source,note,timestamp]).map_err(sql_error)?;
        if let Some(saved) = saved {
            conn.execute("INSERT INTO filed_report_chain_dependencies(snapshot_id,profile_id,transaction_id,wallet_id,chain,network,txid,observation_json) SELECT ?,profile_id,transaction_id,wallet_id,chain,network,txid,observation_json FROM filed_report_chain_dependencies WHERE snapshot_id=? AND profile_id=?",params![id,text(&saved,"id"),profile_id]).map_err(sql_error)?;
            conn.execute("INSERT INTO filed_report_chain_captures(snapshot_id,profile_id,source_snapshot_id,captured_at) VALUES(?,?,?,?)",params![id,profile_id,text(&saved,"id"),sources.now(conn)?]).map_err(sql_error)?;
        }
        super::snapshots::get_snapshot(conn, &id, profile_id)
    })();
    match result {
        Ok(value) => {
            if let Err(error) = conn.execute_batch("RELEASE SAVEPOINT filed_snapshot_create") {
                let _ = conn.execute_batch(
                    "ROLLBACK TO filed_snapshot_create; RELEASE filed_snapshot_create",
                );
                return Err(sql_error(error));
            }
            Ok(value)
        }
        Err(error) => {
            let _ = conn
                .execute_batch("ROLLBACK TO filed_snapshot_create; RELEASE filed_snapshot_create");
            Err(error)
        }
    }
}
fn same_object(a: &Value, b: &Value) -> Result<bool> {
    Ok(json(a)? == json(b)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl Sources for Fixed {
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("new".into())
        }
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
    }
    fn encode(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                ensure_ascii: true,
                sort_keys: true,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap()
    }
    #[test]
    fn snapshot_write_python_rows_and_refusals() {
        let Value::List(cases) =
            kassiber_pytext::loads(include_str!("../tests/data/snapshot_writes.json")).unwrap()
        else {
            panic!()
        };
        assert!(cases.len() > 100);
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        for case in cases {
            conn.execute_batch("SAVEPOINT snapshot_fixture").unwrap();
            conn.execute_batch(include_str!("../tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(&text(&case, "sql")).unwrap();
            let result = if truthy(value(&case, "cli")) {
                create_cli(
                    conn,
                    &dict([("id", value(&case, "workspace").clone())]),
                    &dict([("id", value(&case, "profile").clone())]),
                    value(&case, "args"),
                    &Fixed,
                )
            } else {
                create(
                    conn,
                    &text(&case, "workspace"),
                    &text(&case, "profile"),
                    value(&case, "args"),
                    &Fixed,
                )
            };
            let actual = match result {
                Ok(v) => dict([("ok", v)]),
                Err(e) => dict([(
                    "error",
                    dict([
                        ("code", s(e.code)),
                        ("message", s(e.message)),
                        ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                        ("details", e.details.unwrap_or(Value::Null)),
                        ("retryable", Value::Bool(e.retryable)),
                    ]),
                )]),
            };
            assert_eq!(
                encode(&actual),
                encode(value(&case, "expected")),
                "{}",
                text(&case, "name")
            );
            let Value::Dict(tables) = value(&case, "tables") else {
                panic!()
            };
            for (table, expected) in tables {
                let table = key_string(table);
                let actual = Value::List(
                    rows(conn, &format!("SELECT * FROM {table} ORDER BY 1,2,3"), []).unwrap(),
                );
                assert_eq!(
                    encode(&actual),
                    encode(expected),
                    "{}: {table}",
                    text(&case, "name")
                );
            }
            assert!(
                !conn.is_autocommit(),
                "snapshot committed its caller: {}",
                text(&case, "name")
            );
            conn.execute_batch("ROLLBACK TO snapshot_fixture; RELEASE snapshot_fixture")
                .unwrap();
        }
    }
    #[test]
    fn snapshot_write_preserves_callers_transaction() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(
            "BEGIN; INSERT INTO settings(key,value) VALUES('caller','uncommitted');",
        )
        .unwrap();
        let args = dict([
            ("report_kind", s("summary")),
            ("report_state", s("saved")),
            ("period_start_year", Value::Int(2024.into())),
            ("content_sha256", s("a".repeat(64))),
        ]);
        create(conn, "ws", "p", &args, &Fixed).unwrap();
        assert!(!conn.is_autocommit());
        assert!(create(conn, "ws", "p", &args, &Fixed).is_err());
        assert_eq!(
            conn.query_row("SELECT value FROM settings WHERE key='caller'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "uncommitted"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn snapshot_create_dispatch_is_persistent() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET last_processed_at=NULL")
            .unwrap();
        let args = dict([
            ("report_kind", s("summary")),
            ("state", s("saved")),
            ("period_start_year", Value::Int(2024.into())),
            ("content_sha256", s("a".repeat(64))),
        ]);
        let saved =
            execute_with_sources(conn, "reports.filed-snapshots.create", &args, &Fixed).unwrap();
        assert_eq!(text(&saved, "id"), "new");
        assert_eq!(text(&saved, "report_state"), "saved");
        assert!(conn.is_autocommit());
        let listed = execute(conn, "reports.filed-snapshots.list", &dict::<&str>([])).unwrap();
        assert_eq!(value(&listed, "snapshots"), &Value::List(vec![saved]));
        assert!(
            execute_with_sources(conn, "reports.filed-snapshots.create", &args, &Fixed).is_err()
        );
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
