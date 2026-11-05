//! Python open_db's ordered compatibility pass. Kept private while R3's
//! durability gates and R2's custody matcher/consolidation remain incomplete.
use super::{backfills, custody, data, ensure_column, ownership, repairs};
use crate::{map_sqlite_error, Error, ErrorKind, Result};
use rusqlite::Connection;
use serde_json::json;

fn execute(conn: &Connection, sql: &str) -> Result<()> {
    conn.execute_batch(sql).map_err(map_sqlite_error)
}

fn commit(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        execute(conn, "COMMIT")?;
    }
    Ok(())
}

// The manifest supplies exact definitions, never migration control flow.
// Named endpoints keep the handwritten ordering reviewable against db.py.
fn columns_between(conn: &Connection, first: (&str, &str), last: (&str, &str)) -> Result<()> {
    let columns = data()["functions"]["ensure_schema_compat"]["columns"]
        .as_array()
        .expect("compatibility columns");
    let position = |(table, name): (&str, &str)| {
        columns
            .iter()
            .position(|column| column[0] == table && column[1] == name)
            .expect("checked column endpoint")
    };
    let mut present = std::collections::BTreeMap::new();
    for column in &columns[position(first)..=position(last)] {
        let table = column[0].as_str().unwrap();
        let existing = match present.entry(table) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(super::columns(conn, table)?)
            }
        };
        let name = column[1].as_str().unwrap();
        if existing.iter().any(|(n, _)| n == name) {
            continue;
        }
        if ensure_column(conn, table, name, column[2].as_str().unwrap())? {
            commit(conn)?;
        }
        existing.push((name.to_owned(), String::new()));
    }
    Ok(())
}

fn boundary(step: &str, reason: &str) -> Error {
    Error::new(
        ErrorKind::UnsupportedSchemaLayout,
        "legacy custody migration requires the R2 custody matcher; migrate this book with Python",
    )
    .hint("Open this book with the Python build of Kassiber to migrate it first, then retry the Rust binary.")
    .details(json!({"migration_step":step,"reason":reason}))
}

fn componentless_recovery(conn: &Connection) -> Result<()> {
    // Same cheap pending predicate as Python. Without observations the higher
    // layer cannot recover a candidate either; retain R1's no-op admission.
    let pending: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS({})",
                repairs::statement(
                    "_backfill_legacy_componentless_review_transactions",
                    "SELECT 1"
                )
                .trim()
            ),
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)?;
    if pending {
        let observations: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM custody_gap_reviews r JOIN transactions t ON t.profile_id=r.profile_id WHERE r.component_id IS NULL AND r.action='dismissed' AND coalesce(r.event_kind,'review_decision')='review_decision' AND coalesce(t.excluded,0)=0 AND (NOT EXISTS(SELECT 1 FROM custody_gap_review_relation_sets s WHERE s.review_id=r.id) OR EXISTS(SELECT 1 FROM custody_gap_review_relation_sets s WHERE s.review_id=r.id AND (s.expected_source_count!=(SELECT COUNT(*) FROM custody_gap_review_transactions x WHERE x.review_id=r.id AND x.role='source') OR s.expected_return_count!=(SELECT COUNT(*) FROM custody_gap_review_transactions x WHERE x.review_id=r.id AND x.role='return')))))",
            [], |row| row.get(0),
        ).map_err(map_sqlite_error)?;
        if observations {
            return Err(boundary(
                "_backfill_legacy_componentless_review_transactions",
                "legacy_gap_candidate_recovery",
            ));
        }
    }
    Ok(())
}

pub(crate) fn migrate(
    conn: &mut Connection,
    uuid: &mut impl FnMut(&Connection) -> Result<String>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    super::install_base(conn)?;
    ensure_schema_compat(conn, uuid, observer)?;
    ownership::instance_id(conn, uuid, observer)?;
    super::install_graph(conn)?;
    commit(conn)?;
    observer("compat.complete")
}

fn ensure_schema_compat(
    conn: &mut Connection,
    uuid: &mut impl FnMut(&Connection) -> Result<String>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    backfills::inbox(conn, observer)?;
    ownership::inline_history(conn, uuid, observer)?;
    execute(
        conn,
        repairs::statement("ensure_schema_compat", "UPDATE source_funds_links"),
    )?;
    backfills::gap_inputs(conn, observer)?;
    columns_between(
        conn,
        ("chain_observer_instances", "state_epoch"),
        ("profiles", "journal_semantics_version"),
    )?;
    crate::semantics::adopt(conn, observer)?;
    execute(
        conn,
        repairs::statement(
            "ensure_schema_compat",
            "CREATE TRIGGER IF NOT EXISTS trg_journal_semantics_unstamped",
        ),
    )?;
    ensure_column(conn, "profiles", "swap_candidate_count", "INTEGER")?;
    if ensure_column(conn, "profiles", "ownership_review_counts_json", "TEXT")? {
        execute(
            conn,
            repairs::statement("ensure_schema_compat", "UPDATE profiles"),
        )?;
        commit(conn)?;
    }
    commit(conn)?;
    crate::semantics::migrate_stage(conn, false, observer)?;
    columns_between(
        conn,
        ("backends", "batch_size"),
        ("transactions", "pricing_review_accepted_price"),
    )?;
    execute(
        conn,
        repairs::statement("ensure_schema_compat", "CREATE INDEX"),
    )?;
    columns_between(
        conn,
        ("transactions", "commercial_applied_link_id"),
        ("transactions", "privacy_boundary"),
    )?;
    crate::retired::drop_device_sync_with_observer(conn, observer)?;
    columns_between(
        conn,
        ("journal_entries", "fiat_value_exact"),
        ("source_funds_cases", "target_external_id"),
    )?;
    repairs::step(conn, "source-target", observer)?;
    columns_between(
        conn,
        ("source_funds_recipients", "active"),
        ("ai_providers", "agent_access"),
    )?;
    execute(
        conn,
        repairs::statement("ensure_schema_compat", "UPDATE ai_providers"),
    )?;
    commit(conn)?;
    repairs::step(conn, "ai-refs", observer)?;
    repairs::step(conn, "ai-checks", observer)?;
    backfills::bip329(conn, observer)?;
    repairs::migrate_recipient_unique(conn)?;
    super::migrate_msat(conn, observer)?;
    // Python adds the accepted coarse price again here: the msat rebuild's
    // fixed column list drops it from a legacy REAL-typed transactions table.
    if ensure_column(
        conn,
        "transactions",
        "pricing_review_accepted_price",
        "TEXT",
    )? {
        commit(conn)?;
    }
    columns_between(
        conn,
        ("transactions", "amount_includes_fee"),
        ("journal_entries", "calculation_compact"),
    )?;
    crate::semantics::migrate_stage(conn, true, observer)?;
    columns_between(
        conn,
        ("transactions", "external_id_kind"),
        ("journal_custody_decisions", "target_rail"),
    )?;
    repairs::projection_shapes(conn)?;
    repairs::migrate_attachments(conn, observer)?;
    columns_between(
        conn,
        ("attachments", "copied_from_attachment_id"),
        ("attachments", "copied_from_transaction_id"),
    )?;
    super::liquid::liquid(conn, observer)?;
    backfills::swap_schema(conn, observer)?;
    repairs::direct_payout(conn)?;
    repairs::economic_notes(conn)?;
    repairs::step(conn, "legacy-freeze", observer)?;
    custody::durable_evidence(conn, observer)?;
    repairs::leg_roles(conn, observer)?;
    repairs::ai_audit_consent(conn, observer)?;
    // As in Python, create the view after renames can rewrite its references.
    repairs::step(conn, "projection-view", observer)?;
    custody::relation_schema(conn, observer)?;
    custody::review_relations(conn, observer)?;
    componentless_recovery(conn)?;
    repairs::immutability(conn)?;
    let pending: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM transaction_pairs WHERE component_id IS NULL) OR EXISTS(SELECT 1 FROM direct_swap_payouts WHERE component_id IS NULL) OR EXISTS(SELECT 1 FROM custody_authored_migration_issues WHERE resolved_at IS NULL)",
        [], |row| row.get(0),
    ).map_err(map_sqlite_error)?;
    if pending {
        return Err(boundary(
            "refresh_legacy_authored_components",
            "legacy_authored_custody",
        ));
    }
    repairs::commercial(conn)?;
    repairs::freshness(conn)?;
    repairs::step(conn, "graph-cache", observer)?;
    crate::retired::retire_ledger_with_observer(conn, observer)?;
    crate::semantics::partial_payout(conn, observer)?;
    transaction_page_index(conn, observer)?;
    Ok(())
}

fn transaction_page_index(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn.transaction().map_err(map_sqlite_error)?;
    observer("transaction_page_index.before")?;
    tx.execute_batch(super::sql("_ensure_transaction_page_index", 1))
        .map_err(map_sqlite_error)?;
    let definition: String = tx.query_row(
        "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_transactions_profile_page'",
        [], |row| row.get(0),
    ).map_err(map_sqlite_error)?;
    let normalized = definition
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" if not exists", "");
    if normalized != "create index idx_transactions_profile_page on transactions(profile_id, occurred_at, created_at, id, wallet_id)" {
        return Err(Error::migration("transaction_page_index"));
    }
    observer("transaction_page_index.after")?;
    tx.commit().map_err(map_sqlite_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A current book taken back to main #654's storage: no accepted coarse
    /// price, no provider `agent_access`, no `ai_provider_checks` and the
    /// narrow custody AI consent CHECK, with one audit row. It also lacks
    /// `privacy_boundary`, which the pass adds after the accepted price.
    fn pre_sync_book() -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        let conn = &book.connection;
        let objects: Vec<(String, String)> = conn
            .prepare("SELECT name,sql FROM sqlite_master WHERE tbl_name='custody_ai_assistance_audits' AND sql IS NOT NULL ORDER BY type!='table',name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut script = String::from("PRAGMA foreign_keys=OFF; DROP TABLE ai_provider_checks; ALTER TABLE ai_providers DROP COLUMN agent_access; ALTER TABLE transactions DROP COLUMN pricing_review_accepted_price; ALTER TABLE transactions DROP COLUMN privacy_boundary; DROP TABLE custody_ai_assistance_audits;");
        for (name, sql) in objects {
            let sql = if name == "custody_ai_assistance_audits" {
                let narrow =
                    sql.replace("'mode_auto', 'mode_full_access',\n        'mcp_host', ", "");
                assert_ne!(narrow, sql);
                narrow
            } else {
                sql
            };
            script.push_str(&sql);
            script.push(';');
        }
        script.push_str("PRAGMA foreign_keys=ON; INSERT INTO workspaces VALUES('ws','Synthetic','fixed'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');");
        conn.execute_batch(&script).unwrap();
        conn.execute("INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,facts_sha256,consent_decision,consent_requested_at,consent_decided_at,execution_status,created_at) VALUES('audit','ws','pf','tool','kind','call','local','m',?1,'allow_once','t','t','executed','t')", [&"0".repeat(64)]).unwrap();
        (root, book)
    }

    #[test]
    fn main_sync_storage_upgrades_a_pre_sync_book_and_admits_it() {
        let (_root, mut book) = pre_sync_book();
        let conn = &mut book.connection;
        assert!(crate::schema::verify(conn).is_err());
        migrate(conn, &mut |_| panic!("identity must survive"), &mut |_| {
            Ok(())
        })
        .unwrap();
        crate::schema::verify(conn).unwrap();
        let has = |table: &str, column: &str| {
            super::super::columns(conn, table)
                .unwrap()
                .iter()
                .any(|(name, _)| name == column)
        };
        // Python adds the accepted price before the later transaction
        // columns, so a book missing both keeps that order.
        let order: Vec<String> = super::super::columns(conn, "transactions")
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| {
                ["pricing_review_accepted_price", "privacy_boundary"].contains(&name.as_str())
            })
            .collect();
        assert_eq!(order, ["pricing_review_accepted_price", "privacy_boundary"]);
        assert!(has("ai_providers", "agent_access"));
        assert!(has("ai_provider_checks", "value_json"));
        let audits: Vec<(String, String)> = conn
            .prepare("SELECT id,consent_decision FROM custody_ai_assistance_audits")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(audits, [("audit".to_owned(), "allow_once".to_owned())]);
        conn.execute("INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,facts_sha256,consent_decision,consent_requested_at,consent_decided_at,execution_status,created_at) VALUES('host','ws','pf','tool','kind','call','local','m',?1,'mcp_host','t','t','executed','t')", [&"1".repeat(64)]).unwrap();
        conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) VALUES('p','http://127.0.0.1:9/v1','local','t','t')", []).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT agent_access FROM ai_providers WHERE name='p'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "full"
        );
        let version: i64 = conn
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .unwrap();
        migrate(conn, &mut |_| panic!("identity must survive"), &mut |_| {
            Ok(())
        })
        .unwrap();
        assert_eq!(
            conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }

    #[test]
    fn page_index_storage_errors_roll_back_and_retry() {
        for failure in ["busy", "full", "rollback"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("errors.sqlite3");
            let mut conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE transactions(profile_id,occurred_at,created_at,id,wallet_id); INSERT INTO transactions VALUES(1,2,3,4,5);").unwrap();
            conn.busy_timeout(std::time::Duration::ZERO).unwrap();
            let other = Connection::open(&path).unwrap();
            let cookie: i64 = conn
                .query_row("PRAGMA schema_version", [], |r| r.get(0))
                .unwrap();
            if failure == "busy" {
                other.execute_batch("BEGIN IMMEDIATE").unwrap();
            } else if failure == "full" {
                let pages: i64 = conn
                    .query_row("PRAGMA page_count", [], |r| r.get(0))
                    .unwrap();
                conn.execute_batch(&format!("PRAGMA max_page_count={pages}"))
                    .unwrap();
            }
            assert!(transaction_page_index(&mut conn, &mut |label| {
                if failure == "rollback" && label == "transaction_page_index.after" {
                    Err(Error::migration("injected"))
                } else {
                    Ok(())
                }
            })
            .is_err());
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                cookie
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            if failure == "busy" {
                other.execute_batch("ROLLBACK").unwrap();
            }
            conn.execute_batch("PRAGMA max_page_count=1073741823")
                .unwrap();
            transaction_page_index(&mut conn, &mut |_| Ok(())).unwrap();
            transaction_page_index(&mut conn, &mut |_| Ok(())).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                cookie + 1
            );
        }
    }

    #[test]
    fn current_open_repairs_changed_indexes_and_view_before_admission() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let conn = &mut book.connection;
        let before = crate::schema::fingerprint(conn).unwrap();
        conn.execute_batch("DROP INDEX idx_transaction_pairs_active_out; CREATE INDEX idx_transaction_pairs_active_out ON transaction_pairs(id); DROP INDEX idx_commercial_links_unique_payment_tx_active; CREATE UNIQUE INDEX idx_commercial_links_unique_payment_tx_active ON commercial_links(id); DROP VIEW journal_custody_projection_relations; CREATE VIEW journal_custody_projection_relations AS SELECT 1;").unwrap();
        migrate(conn, &mut |_| panic!("identity must survive"), &mut |_| {
            Ok(())
        })
        .unwrap();
        assert_eq!(crate::schema::fingerprint(conn).unwrap(), before);
        crate::schema::verify(conn).unwrap();
    }

    #[test]
    fn current_book_keeps_identity_audits_and_journal_mode() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let identity = book.instance_id().to_owned();
        let conn = &mut book.connection;
        let mode = |conn: &Connection| {
            conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                .unwrap()
        };
        let journal = mode(conn);
        let audits = |conn: &Connection| {
            conn.prepare(
                "SELECT id,impact_json,created_at FROM schema_migration_audits ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };
        let before = audits(conn);
        let schema_version: i64 = conn
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .unwrap();
        migrate(
            conn,
            &mut |_| panic!("current identities must survive"),
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(crate::read_instance_id(conn).unwrap(), identity);
        assert_eq!(audits(conn), before);
        assert_eq!(mode(conn), journal);
        assert_eq!(
            conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            schema_version
        );
        crate::schema::verify(conn).unwrap();
    }
}
