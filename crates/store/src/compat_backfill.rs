//! Small ordered compatibility backfills. Preserve authored content and Python
//! JSON bytes; only the reference's derived caches and explicit columns change.
use super::{
    data, python_json as py,
    repairs::{statement, statements},
};
use crate::{map_sqlite_error, stored_values::Shape, Error, Result};
use kassiber_pyjson::{BigInt, Key, Value};
use rusqlite::{params, Connection, OptionalExtension};

fn unsupported() -> Error {
    crate::stored_values::refusal(Shape::Other)
}
fn object(raw: Option<&str>) -> Result<Vec<(Key, Value)>> {
    match py::loads(raw.filter(|v| !v.is_empty()).unwrap_or("{}")) {
        Ok(Value::Dict(items)) => Ok(items),
        Ok(_) | Err(py::ParseError::Invalid) => Ok(Vec::new()),
        Err(_) => Err(unsupported()),
    }
}
pub(super) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Int(value) => value != &BigInt::from(0),
        Value::Float(value) => *value != 0.0,
        Value::Str(value) => value.code_points().next().is_some(),
        Value::Bytes(value) => !value.is_empty(),
        Value::List(items) => !items.is_empty(),
        Value::Dict(items) => !items.is_empty(),
        Value::Decimal(_) => true,
    }
}
pub(super) fn integer(value: &Value) -> Result<Option<BigInt>> {
    Ok(match value {
        Value::Int(value) => Some(value.clone()),
        Value::Bool(value) => Some(BigInt::from(u8::from(*value))),
        Value::Float(value) => {
            if !value.is_finite() {
                return Err(crate::stored_values::refusal(Shape::NonFinite));
            }
            let bits = value.to_bits();
            let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
            let mantissa = if bits >> 52 & 0x7ff == 0 {
                bits & ((1u64 << 52) - 1)
            } else {
                (bits & ((1u64 << 52) - 1)) | (1u64 << 52)
            };
            let number = if exponent >= 0 {
                BigInt::from(mantissa) << (exponent as usize)
            } else {
                BigInt::from(mantissa) >> ((-exponent) as usize)
            };
            Some(if value.is_sign_negative() {
                -number
            } else {
                number
            })
        }
        Value::Str(text) => {
            let Some(text) = text.as_str() else {
                return Ok(None);
            };
            let text = text.trim_matches(char::is_whitespace);
            let (negative, text) = if let Some(text) = text.strip_prefix('-') {
                (true, text)
            } else {
                (false, text.strip_prefix('+').unwrap_or(text))
            };
            let mut digits = String::new();
            let mut previous_digit = false;
            for c in text.chars() {
                if c == '_' {
                    if !previous_digit {
                        return Ok(None);
                    }
                    previous_digit = false;
                    continue;
                }
                let digit = if c.is_ascii_digit() {
                    u64::from(c as u8 - b'0')
                } else {
                    let Some(value) = data()["unicode_decimal"][u32::from(c).to_string()].as_u64()
                    else {
                        return Ok(None);
                    };
                    value
                };
                digits.push(char::from(b'0' + digit as u8));
                previous_digit = true;
            }
            if !previous_digit || digits.len() > kassiber_pyjson::INT_MAX_STR_DIGITS {
                return Ok(None);
            }
            let number = BigInt::parse_bytes(digits.as_bytes(), 10).expect("decimal digits");
            Some(if negative { -number } else { number })
        }
        _ => None,
    })
}
fn sql_integer(value: BigInt) -> Result<i64> {
    i64::try_from(value).map_err(|_| crate::stored_values::refusal(Shape::BeyondI64))
}
pub(super) fn whitespace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
pub(super) fn asset(text: &str) -> String {
    let text = text.trim_matches(whitespace);
    let lower = text.to_lowercase();
    if lower.len() == 64 && lower.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        lower
    } else if ["l-btc", "lbtc"].contains(&lower.as_str()) {
        "LBTC".into()
    } else {
        text.to_uppercase()
    }
}
fn eligible_hash(text: &str) -> bool {
    if text.len() != 64 {
        return false;
    }
    let mut bytes = text.bytes();
    while let Some(first) = bytes.next() {
        if first.is_ascii_whitespace() || first == b'\x0b' {
            continue;
        }
        if !first.is_ascii_hexdigit()
            || !bytes
                .next()
                .is_some_and(|second| second.is_ascii_hexdigit())
        {
            return false;
        }
    }
    true
}
pub(crate) fn payment_hash(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let function = "_backfill_payment_hash_from_raw_json";
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let mut statement_sql = tx
        .prepare(statement(function, "SELECT id, raw_json"))
        .map_err(map_sqlite_error)?;
    let rows = statement_sql
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(statement_sql);
    for (id, raw) in rows {
        let payload = Value::Dict(object(raw.as_deref())?);
        let Some(Value::Str(text)) = py::get(&payload, "payment_hash") else {
            continue;
        };
        let Some(text) = text.as_str() else {
            continue;
        };
        let text = text.trim_matches(whitespace).to_lowercase();
        if !eligible_hash(&text) {
            continue;
        }
        tx.execute(
            statement(function, "UPDATE transactions"),
            params![text, id],
        )
        .map_err(map_sqlite_error)?;
        observer("payment-hash.after_update")?;
    }
    observer("payment-hash.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("payment-hash.after_commit")
}
pub(crate) fn swap_fees(conn: &mut Connection) -> Result<()> {
    let function = "_clear_stale_same_asset_swap_fees";
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let mut stmt = tx
        .prepare(statement(function, "SELECT p.id"))
        .map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    for (id, kind, out_asset, in_asset) in rows {
        if asset(&out_asset) != asset(&in_asset) {
            continue;
        }
        if [
            "chain-swap",
            "peg-in",
            "peg-out",
            "reverse-submarine-swap",
            "submarine-swap",
            "swap-refund",
        ]
        .contains(
            &kind
                .unwrap_or_default()
                .trim_matches(whitespace)
                .to_lowercase()
                .as_str(),
        ) {
            continue;
        }
        tx.execute(statement(function, "UPDATE transaction_pairs"), [id])
            .map_err(map_sqlite_error)?;
    }
    tx.commit().map_err(map_sqlite_error)
}
pub(crate) fn bip329(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let function = "_ensure_bip329_wallet_agnostic_schema";
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let mut stmt = tx
        .prepare(statement(function, "SELECT profile_id"))
        .map_err(map_sqlite_error)?;
    let groups = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    for (profile, kind, reference) in groups {
        // Select named fields; SELECT * column order differs on ALTER layouts.
        let mut stmt=tx.prepare("SELECT id,label,origin,spendable,data_json FROM bip329_labels WHERE profile_id=? AND record_type=? AND ref=? ORDER BY created_at ASC,id ASC").map_err(map_sqlite_error)?;
        let rows = stmt
            .query_map(params![profile, kind, reference], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sqlite_error)?;
        drop(stmt);
        let Some(canonical) = rows.last() else {
            continue;
        };
        let (mut label, mut origin, mut spendable) = (None, None, None);
        let mut merged = Vec::new();
        for (_, next_label, next_origin, next_spendable, raw) in &rows {
            if next_label.is_some() {
                label = next_label.clone();
            }
            if next_origin.is_some() {
                origin = next_origin.clone();
            }
            if next_spendable.is_some() {
                spendable = *next_spendable;
            }
            for (key, value) in object(raw.as_deref())? {
                if let Some((_, old)) = merged.iter_mut().find(|(old, _)| old == &key) {
                    *old = value;
                } else {
                    merged.push((key, value));
                }
            }
        }
        tx.execute(
            statement(function, "UPDATE bip329_labels\n            SET"),
            params![
                label,
                origin,
                spendable,
                py::dumps(&Value::Dict(merged), true, false)?,
                canonical.0
            ],
        )
        .map_err(map_sqlite_error)?;
        tx.execute(
            statement(function, "DELETE FROM bip329_labels"),
            params![profile, kind, reference, canonical.0],
        )
        .map_err(map_sqlite_error)?;
        observer("bip329.after_merge")?;
    }
    for sql in statements(function).into_iter().filter(|sql| {
        sql.trim_start().starts_with("CREATE")
            || sql.trim_start().starts_with("UPDATE bip329_labels SET")
    }) {
        tx.execute_batch(sql).map_err(map_sqlite_error)?;
    }
    observer("bip329.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("bip329.after_commit")
}
pub(crate) fn inbox(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let function = "ensure_inbox_sources";
    let mut stmt = conn
        .prepare("PRAGMA table_info(chain_analysis_watch_inbox)")
        .map_err(map_sqlite_error)?;
    let columns = stmt
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(3)?)))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    if !columns
        .iter()
        .any(|(name, not_null)| name == "watch_id" && *not_null != 0)
    {
        return Ok(());
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let trigger:Option<String>=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='trg_report_chain_impacts_delete_inbox'",[],|r|r.get(0)).optional().map_err(map_sqlite_error)?;
    for sql in statements(function) {
        let trimmed = sql.trim_start();
        if trimmed.starts_with("SELECT")
            || trimmed.starts_with("PRAGMA")
            || trimmed.starts_with("SAVEPOINT")
            || trimmed.starts_with("ROLLBACK")
            || trimmed.starts_with("RELEASE")
        {
            continue;
        }
        tx.execute_batch(sql).map_err(map_sqlite_error)?;
        if trimmed.starts_with("CREATE TABLE") {
            observer("inbox.after_create")?;
        }
        if trimmed.starts_with("INSERT") {
            observer("inbox.after_copy")?;
        }
        if trimmed.starts_with("ALTER TABLE") {
            observer("inbox.after_replace")?;
        }
    }
    if let Some(trigger) = trigger {
        tx.execute_batch(&trigger).map_err(map_sqlite_error)?;
    }
    observer("inbox.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("inbox.after_commit")
}
pub(crate) fn gap_inputs(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let function = "ensure_schema_compat";
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    tx.execute_batch("DROP TABLE IF EXISTS custody_gap_candidate_snapshots;DROP TABLE IF EXISTS custody_gap_projection_rows").map_err(map_sqlite_error)?;
    let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='custody_gap_candidate_projections')",[],|r|r.get(0)).map_err(map_sqlite_error)?;
    if present {
        let mut stmt = tx
            .prepare(statement(function, "SELECT projection.profile_id"))
            .map_err(map_sqlite_error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sqlite_error)?;
        drop(stmt);
        for (profile, workspace, version, ignored, accounting_ignored, created) in rows {
            let version = match py::loads(&version) {
                Ok(value) => value,
                Err(py::ParseError::Invalid) => continue,
                Err(_) => return Err(unsupported()),
            };
            let last = if !truthy(&version) {
                Value::from(0)
            } else {
                match version {
                    Value::List(mut values) => values.pop().unwrap(),
                    Value::Str(text) => {
                        let Some(point) = text.code_points().last() else {
                            continue;
                        };
                        Value::Str(kassiber_pyjson::PyStr::from_code_points([point]).unwrap())
                    }
                    Value::Dict(_) => return Err(unsupported()),
                    _ => continue,
                }
            };
            let Some(input_version) = integer(&last)? else {
                continue;
            };
            let input_version = sql_integer(input_version)?;
            tx.execute(
                statement(
                    function,
                    "INSERT OR REPLACE INTO journal_custody_gap_inputs",
                ),
                params![
                    workspace,
                    profile,
                    input_version,
                    ignored,
                    accounting_ignored,
                    created
                ],
            )
            .map_err(map_sqlite_error)?;
            observer("gap-inputs.after_copy")?;
        }
    }
    for table in [
        "custody_gap_candidate_boundaries",
        "custody_gap_candidates",
        "custody_gap_candidate_projections",
        "custody_tax_migration_baseline_events",
        "custody_tax_migration_baselines",
        "custody_tax_migration_reports",
    ] {
        tx.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))
            .map_err(map_sqlite_error)?;
    }
    observer("gap-inputs.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("gap-inputs.after_commit")
}
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn step(
    conn: &mut Connection,
    name: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    match name {
        "bip329" => bip329(conn, observer),
        "payment-hash" => payment_hash(conn, observer),
        "swap-fees" => swap_fees(conn),
        "inbox" => inbox(conn, observer),
        "gap-inputs" => gap_inputs(conn, observer),
        "ownership-history" => {
            super::ownership::inline_history(conn, &mut super::ownership::uuid_hex, observer)
        }
        "swap-schema" => swap_schema(conn, observer),
        "liquid" => super::liquid::liquid(conn, observer),
        "database-identity" => {
            super::ownership::instance_id(conn, &mut super::ownership::uuid_hex, observer)
        }
        _ => super::custody::step(conn, name, observer),
    }
}
pub(crate) fn swap_schema(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    super::repairs::migrate_pair_uniques(conn, observer)?;
    for column in data()["functions"]["_ensure_swap_matching_schema"]["columns"]
        .as_array()
        .expect("swap columns")
    {
        super::ensure_column(
            conn,
            column[0].as_str().unwrap(),
            column[1].as_str().unwrap(),
            column[2].as_str().unwrap(),
        )?;
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let indexes = statements("_ensure_swap_matching_schema");
    for sql in &indexes {
        if super::repairs::unchanged_index_statement(&tx, sql, &indexes)? {
            continue;
        }
        tx.execute_batch(sql).map_err(map_sqlite_error)?;
    }
    observer("swap-schema.before_indexes_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("swap-schema.after_indexes_commit")?;
    swap_fees(conn)?;
    payment_hash(conn, observer)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    fn fixture(step_name: &str) -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        let conn = &book.connection;
        conn.execute_batch("INSERT INTO workspaces VALUES('ws','Synthetic','fixed');INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','pf','Wallet','custom','fixed');").unwrap();
        match step_name {
            "bip329"=>conn.execute_batch("DROP INDEX idx_bip329_labels_profile_object;INSERT INTO bip329_labels(id,workspace_id,profile_id,wallet_id,record_type,ref,label,data_json,created_at) VALUES('a','ws','pf','wal','tx','ref','Old','{\"kept\":1,\"changed\":1}','a'),('b','ws','pf',NULL,'tx','ref','New','{\"changed\":2}','b');").unwrap(),
            "payment-hash"=>{
                let raw=format!("{{\"payment_hash\":\"{}\",\"extra\":NaN}}","ab".repeat(32));
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES('tx','ws','pf','wal','tx','fixed','inbound','BTC',1,?,'fixed')",[raw]).unwrap();
            }
            "gap-inputs"=>conn.execute_batch("CREATE TABLE custody_gap_candidate_projections(profile_id TEXT,producer_kind TEXT,version_json TEXT,ignored_ids_json TEXT,accounting_ignored_ids_json TEXT,created_at TEXT);INSERT INTO custody_gap_candidate_projections VALUES('pf','journal','[1,2,7]','[\"retained\"]','[]','fixed');").unwrap(),
            _=>unreachable!()
        }
        (root, book)
    }
    #[test]
    fn backfills_publish_complete_rows_or_roll_back_at_every_boundary() {
        for (name, points) in [
            (
                "bip329",
                vec![
                    "bip329.after_merge",
                    "bip329.before_commit",
                    "bip329.after_commit",
                ],
            ),
            (
                "payment-hash",
                vec![
                    "payment-hash.after_update",
                    "payment-hash.before_commit",
                    "payment-hash.after_commit",
                ],
            ),
            (
                "gap-inputs",
                vec![
                    "gap-inputs.after_copy",
                    "gap-inputs.before_commit",
                    "gap-inputs.after_commit",
                ],
            ),
        ] {
            for target in points {
                let (_root, mut book) = fixture(name);
                let conn = &mut book.connection;
                assert!(step(conn, name, &mut |label| if label == target {
                    Err(Error::new(ErrorKind::Io, "injected backfill failure"))
                } else {
                    Ok(())
                })
                .is_err());
                assert!(conn.is_autocommit());
                let committed = target.ends_with("after_commit");
                match name {
                    "bip329" => assert_eq!(
                        conn.query_row("SELECT COUNT(*) FROM bip329_labels", [], |r| r
                            .get::<_, i64>(0))
                            .unwrap(),
                        if committed { 1 } else { 2 }
                    ),
                    "payment-hash" => assert_eq!(
                        conn.query_row(
                            "SELECT payment_hash IS NOT NULL FROM transactions",
                            [],
                            |r| r.get::<_, bool>(0)
                        )
                        .unwrap(),
                        committed
                    ),
                    "gap-inputs" => assert_eq!(
                        conn.query_row(
                            "SELECT COUNT(*) FROM journal_custody_gap_inputs",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                        i64::from(committed)
                    ),
                    _ => unreachable!(),
                }
                step(conn, name, &mut |_| Ok(())).unwrap();
                match name {
                    "bip329" => assert_eq!(
                        conn.query_row(
                            "SELECT id,label,data_json,wallet_id IS NULL FROM bip329_labels",
                            [],
                            |r| Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, bool>(3)?
                            ))
                        )
                        .unwrap(),
                        (
                            "b".into(),
                            "New".into(),
                            "{\"changed\": 2, \"kept\": 1}".into(),
                            true
                        )
                    ),
                    "payment-hash" => assert_eq!(
                        conn.query_row(
                            "SELECT payment_hash,payment_hash_source FROM transactions",
                            [],
                            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        )
                        .unwrap(),
                        ("ab".repeat(32), "importer_backfill".into())
                    ),
                    "gap-inputs" => assert_eq!(
                        conn.query_row(
                            "SELECT input_version,ignored_ids_json FROM journal_custody_gap_inputs",
                            [],
                            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                        )
                        .unwrap(),
                        (7, "[\"retained\"]".into())
                    ),
                    _ => unreachable!(),
                }
            }
        }
    }
    #[test]
    fn integer_conversion_matches_python_shapes() {
        for (raw,expected) in [("42","42"),("-42.9","-42"),("true","1"),(r#"" +１２_３ ""#,"123"),("-0.5","0"),("1e100","10000000000000000159028911097599180468360808563945281389781327557747838772170381060813469985856815104")] {
            assert_eq!(integer(&py::loads(raw).unwrap()).unwrap().unwrap().to_string(),expected);
        }
        for raw in [r#""1.0""#, r#""_1""#, r#""1_""#, r#""1__2""#, "null", "[]"] {
            assert_eq!(integer(&py::loads(raw).unwrap()).unwrap(), None);
        }
    }
    #[test]
    fn same_asset_manual_fees_clear_without_changing_real_swap_fees() {
        let (_root, mut book) = fixture("payment-hash");
        let conn = &mut book.connection;
        conn.execute_batch("UPDATE transactions SET asset='LBTC';INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES('out','ws','pf','wal','out','fixed','outbound','l-btc',1,'fixed');INSERT INTO transaction_pairs(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,swap_fee_msat,swap_fee_kind,created_at) VALUES('pair','ws','pf','out','tx','manual',7,'legacy','fixed');").unwrap();
        swap_fees(conn).unwrap();
        assert!(conn
            .query_row(
                "SELECT swap_fee_msat IS NULL AND swap_fee_kind IS NULL FROM transaction_pairs",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
        for kind in [
            "peg-in",
            "chain-swap",
            "reverse-submarine-swap",
            "swap-refund",
        ] {
            conn.execute(
                "UPDATE transaction_pairs SET kind=?,swap_fee_msat=7,swap_fee_kind='reviewed'",
                [kind],
            )
            .unwrap();
            swap_fees(conn).unwrap();
            assert_eq!(
                conn.query_row("SELECT swap_fee_msat FROM transaction_pairs", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                7
            );
        }
    }
    #[test]
    fn inbox_rebuild_keeps_cursor_holes_and_rolls_back_each_boundary() {
        for target in [
            "inbox.after_create",
            "inbox.after_copy",
            "inbox.after_replace",
            "inbox.before_commit",
            "inbox.after_commit",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut book = crate::Book::create(root.path(), None).unwrap();
            let conn = &mut book.connection;
            conn.execute_batch("INSERT INTO workspaces VALUES('ws','Synthetic','fixed');INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES('watch','pf','book','{}','{}','{}','fixed','fixed','fixed');DROP TRIGGER trg_report_chain_impacts_delete_inbox;").unwrap();
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='chain_analysis_watch_inbox'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            conn.execute_batch("DROP TABLE chain_analysis_watch_inbox")
                .unwrap();
            conn.execute_batch(&ddl.replace(
                "watch_id TEXT REFERENCES",
                "watch_id TEXT NOT NULL REFERENCES",
            ))
            .unwrap();
            conn.execute_batch("INSERT INTO chain_analysis_watch_inbox(rowid,id,watch_id,profile_id,sequence,code,observation_json,created_at) VALUES(7,'a','watch','pf',1,'synthetic','{}','fixed'),(123,'b','watch','pf',2,'synthetic','{}','fixed')").unwrap();
            assert!(inbox(conn, &mut |label| if label == target {
                Err(Error::new(ErrorKind::Io, "injected inbox failure"))
            } else {
                Ok(())
            })
            .is_err());
            assert!(conn.is_autocommit());
            inbox(conn, &mut |_| Ok(())).unwrap();
            let cursors: Vec<i64> = conn
                .prepare("SELECT rowid FROM chain_analysis_watch_inbox ORDER BY rowid")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(cursors, [7, 123]);
            conn.execute("INSERT INTO chain_analysis_watch_inbox(id,profile_id,sequence,code,observation_json,created_at) VALUES('sync','pf',3,'sync','{}','fixed')",[]).unwrap();
        }
    }
    #[test]
    fn current_books_need_no_data_backfill() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let changes = book.connection.total_changes();
        for name in ["bip329", "payment-hash", "swap-fees", "inbox", "gap-inputs"] {
            step(&mut book.connection, name, &mut |_| Ok(())).unwrap();
            assert_eq!(book.connection.total_changes(), changes);
        }
    }
}
