//! Reset the active profile, preserving its connections and other books.
//!
//! The default reset rebuilds observed and derived rows and keeps the book's
//! decisions for the resync to bind again ([`crate::book_reset`]);
//! `wipe_everything` is the pre-D1 blank-book reset.
use super::*;
use std::path::Path;

fn count(conn: &Connection, sql: &str, profile: &str) -> Result<i64> {
    let parameters = vec![profile; sql.matches('?').count()];
    conn.query_row(sql, rusqlite::params_from_iter(parameters), |r| r.get(0))
        .map_err(sql_error)
}
pub fn execute(
    conn: &Connection,
    args: &Value,
    attachments: &Path,
    sources: &dyn Sources,
) -> Result<Value> {
    let context = context::snapshot(conn)?;
    let wid = text(&context, "workspace_id");
    let pid = text(&context, "profile_id");
    if wid.is_empty() || pid.is_empty() {
        return Err(
            AppError::new("state_not_ready", "No current book is selected.")
                .with_hint("Select a books set and book before resetting book data."),
        );
    }
    sensitive::require_no_legacy(conn, std::slice::from_ref(&pid), "resetting its data")?;
    let wipe = flag(args, "wipe_everything");
    let root = kassiber_paths::canonicalize_allow_missing(attachments).map_err(storage_error)?;
    let mut paths = vec![];
    for row in if wipe {
        rows(conn,"SELECT stored_relpath FROM attachments WHERE profile_id=? AND stored_relpath IS NOT NULL AND stored_relpath!=''",[&pid])?
    } else {
        vec![]
    } {
        let path =
            kassiber_paths::canonicalize_allow_missing(&root.join(text(&row, "stored_relpath")))
                .map_err(storage_error)?;
        if path.starts_with(&root) {
            paths.push(path);
        }
    }
    let rates = count(conn, "SELECT COUNT(*) FROM rates_cache", &pid)?;
    let minutes = count(conn, "SELECT COUNT(*) FROM rates_checked_minutes", &pid)?;
    let clear = flag(args, "clear_shared_rates");
    let mut removed = dict::<&str>([]);
    for (table, sql) in COUNTS {
        replace(
            &mut removed,
            table,
            Value::Int(count(conn, sql, &pid)?.into()),
        );
    }
    for (table, n) in [("rates_cache", rates), ("rates_checked_minutes", minutes)] {
        replace(
            &mut removed,
            table,
            Value::Int((if clear { n } else { 0 }).into()),
        );
    }
    let mut kept_in_place = dict::<&str>([]);
    if !wipe {
        for table in KEPT_IN_PLACE {
            replace(
                &mut kept_in_place,
                table,
                Value::Int(
                    count(
                        conn,
                        &format!("SELECT COUNT(*) FROM {table} WHERE profile_id = ?"),
                        &pid,
                    )?
                    .into(),
                ),
            );
        }
        replace(
            &mut kept_in_place,
            "attachments",
            Value::Int(count(conn, "SELECT COUNT(*) FROM attachments WHERE profile_id = ? AND transaction_id IS NULL", &pid)?.into()),
        );
        removed = dict(
            DERIVED_COUNTS
                .iter()
                .map(|key| (*key, get(&removed, key).cloned().unwrap_or(Value::Null))),
        );
    }
    let tx = kassiber_store::write_transaction(conn).map_err(sql_error)?;
    let mut rebind = Vec::new();
    if wipe {
        for (table, n) in crate::book_reset::discard_profile(&tx, &pid)? {
            replace(&mut removed, &table, Value::Int(n.into()));
        }
        for sql in DELETES {
            let parameters = vec![&pid; sql.matches('?').count()];
            tx.execute(sql, rusqlite::params_from_iter(parameters))
                .map_err(sql_error)?;
        }
    } else {
        rebind = crate::book_reset::park_profile(&tx, &wid, &pid, &sources.now(&tx)?)?;
        for sql in DERIVED_DELETES {
            tx.execute(sql, [&pid]).map_err(sql_error)?;
        }
    }
    // Sync checkpoints and last-sync times describe history this book no
    // longer holds. Clear them so the next sync re-imports every wallet in
    // full, retired wallets included; each source keeps its user flags, as
    // `freshness.reset_source_checkpoint` does.
    replace(
        &mut removed,
        "sync_checkpoints",
        Value::Int(count(&tx, &format!("SELECT COUNT(*) FROM freshness_source_states WHERE profile_id = ? AND source_type IN {WALLET_SOURCES}"), &pid)?.into()),
    );
    tx.execute(
        &format!("UPDATE freshness_source_states SET status=CASE WHEN paused THEN 'paused' WHEN blocking_reports THEN 'blocking_reports' ELSE 'partially_stale' END,state=CASE WHEN paused THEN 'paused' WHEN blocking_reports THEN 'blocking_reports' ELSE 'partially_stale' END,stale_reason='book_data_reset',progress_json='{{}}',checkpoint_json='{{}}',rate_limited_until=NULL,cooldown_reason=NULL,last_error_at=NULL,last_error_code=NULL,last_error_message=NULL,last_phase=NULL,updated_at=? WHERE profile_id=? AND source_type IN {WALLET_SOURCES}"),
        [sources.now(&tx)?, pid.clone()],
    )
    .map_err(sql_error)?;
    let mut sync_times = 0_i64;
    for wallet in rows(
        &tx,
        "SELECT id, config_json FROM wallets WHERE profile_id = ? ORDER BY id",
        [&pid],
    )? {
        let stored = match get(&wallet, "config_json") {
            Some(Value::Str(raw)) if !raw.as_str().unwrap_or_default().is_empty() => {
                raw.as_str().unwrap_or_default().to_owned()
            }
            _ => "{}".to_owned(),
        };
        // Python skips a config it cannot decode rather than failing the reset.
        let Ok(Value::Dict(mut config)) = kassiber_pytext::loads(&stored) else {
            continue;
        };
        let Some(position) = config
            .iter()
            .position(|(key, _)| *key == kassiber_pyjson::Key::from("last_synced_at"))
        else {
            continue;
        };
        config.remove(position);
        let encoded = kassiber_pyjson::dumps(
            &Value::Dict(config),
            &kassiber_pyjson::Options {
                sort_keys: true,
                ..Default::default()
            },
        )
        .map_err(|_| AppError::new("app_error", "Wallet config could not be encoded"))?;
        tx.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            [encoded, text(&wallet, "id")],
        )
        .map_err(sql_error)?;
        sync_times += 1;
    }
    replace(
        &mut removed,
        "wallet_sync_times",
        Value::Int(sync_times.into()),
    );
    if clear {
        tx.execute("DELETE FROM rates_cache", [])
            .map_err(sql_error)?;
        tx.execute("DELETE FROM rates_checked_minutes", [])
            .map_err(sql_error)?;
    }
    tx.execute(INVALIDATE, [&pid]).map_err(sql_error)?;
    tx.commit().map_err(sql_error)?;
    // Python publishes the DB reset before best-effort managed file cleanup.
    let mut removed_files = 0_i64;
    for path in paths {
        if path.is_file() && std::fs::remove_file(&path).is_ok() {
            removed_files += 1;
            let mut parent = path.parent();
            while let Some(dir) = parent.filter(|dir| *dir != root && dir.is_dir()) {
                if std::fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            }
        }
    }
    replace(
        &mut removed,
        "attachment_files",
        Value::Int(removed_files.into()),
    );
    let mut preserved = dict([
        ("workspaces", Value::Int(1.into())),
        ("profiles", Value::Int(1.into())),
    ]);
    for table in ["accounts", "wallets"] {
        replace(
            &mut preserved,
            table,
            Value::Int(
                count(
                    conn,
                    &format!("SELECT COUNT(*) FROM {table} WHERE profile_id=?"),
                    &pid,
                )?
                .into(),
            ),
        );
    }
    replace(
        &mut preserved,
        "backends",
        Value::Int(count(conn, "SELECT COUNT(*) FROM backends", &pid)?.into()),
    );
    for (table, n) in [("rates_cache", rates), ("rates_checked_minutes", minutes)] {
        replace(
            &mut preserved,
            table,
            Value::Int((if clear { 0 } else { n }).into()),
        );
    }
    let mut payload = dict([
        ("reset", Value::Bool(true)),
        (
            "mode",
            s(if wipe {
                "wipe_everything"
            } else {
                "keep_decisions"
            }),
        ),
        (
            "workspace",
            dict([
                ("id", s(wid)),
                ("label", get(&context, "workspace_label").unwrap().clone()),
            ]),
        ),
        (
            "profile",
            dict([
                ("id", s(pid)),
                ("label", get(&context, "profile_label").unwrap().clone()),
            ]),
        ),
        ("preserved", preserved),
        ("removed", removed),
        ("rates_scope", s(if clear { "global" } else { "preserved" })),
        ("shared_rates_cleared", Value::Bool(clear)),
    ]);
    if !wipe {
        let kept: i64 = rebind.iter().map(|(_, n)| n).sum::<i64>()
            + match &kept_in_place {
                Value::Dict(items) => items
                    .iter()
                    .map(|(_, v)| match v {
                        Value::Int(n) => i64::try_from(n.clone()).unwrap_or(0),
                        _ => 0,
                    })
                    .sum(),
                _ => 0,
            };
        replace(
            &mut payload,
            "decisions",
            dict([
                ("kept", Value::Int(kept.into())),
                (
                    "bind_after_resync",
                    dict(
                        rebind
                            .iter()
                            .map(|(table, n)| (table.as_str(), Value::Int((*n).into()))),
                    ),
                ),
                ("kept_in_place", kept_in_place),
            ]),
        );
    }
    Ok(payload)
}

/// `maintenance._DERIVED_RESET_COUNTS`: what a keep-decisions reset reports removed.
const DERIVED_COUNTS: &[&str] = &[
    "transactions",
    "chain_analysis_observations",
    "chain_observer_instances",
    "chain_observer_coverage",
    "chain_observer_values",
    "journal_entries",
    "journal_quarantines",
    "journal_tax_summary",
    "journal_account_holdings",
    "journal_wallet_holdings",
    "journal_quantity_postings",
    "journal_quantity_issues",
    "journal_quantity_balances",
    "journal_custody_decisions",
    "journal_custody_economic_relations",
    "journal_custody_gap_inputs",
    "journal_custody_reconciliations",
    "rates_cache",
    "rates_checked_minutes",
];
/// `maintenance._delete_profile_derived_rows`, in Python's order.
const DERIVED_DELETES: &[&str] = &[
    "DELETE FROM chain_observer_instances WHERE profile_id = ?",
    "DELETE FROM chain_analysis_observations WHERE profile_id = ?",
    "DELETE FROM journal_wallet_holdings WHERE profile_id = ?",
    "DELETE FROM journal_quantity_balances WHERE profile_id = ?",
    "DELETE FROM journal_custody_decisions WHERE profile_id = ?",
    "DELETE FROM journal_custody_economic_relations WHERE profile_id = ?",
    "DELETE FROM journal_custody_reconciliations WHERE profile_id = ?",
    "DELETE FROM journal_custody_gap_inputs WHERE profile_id = ?",
    "DELETE FROM journal_quantity_issues WHERE profile_id = ?",
    "DELETE FROM journal_quantity_postings WHERE profile_id = ?",
    "DELETE FROM journal_account_holdings WHERE profile_id = ?",
    "DELETE FROM journal_tax_summary WHERE profile_id = ?",
    "DELETE FROM journal_quarantines WHERE profile_id = ?",
    "DELETE FROM journal_entries WHERE profile_id = ?",
    "DELETE FROM transactions WHERE profile_id = ?",
];
/// `maintenance._KEPT_IN_PLACE_TABLES`; book-level attachments follow.
const KEPT_IN_PLACE: &[&str] = &[
    "custody_components",
    "custody_gap_reviews",
    "custody_authored_evidence_snapshots",
    "filed_report_snapshots",
    "review_workflow_receipts",
    "external_documents",
    "source_funds_sources",
    "source_funds_recipients",
    "tags",
    "saved_views",
    "swap_matching_rules",
    "bip329_labels",
    "chain_analysis_cases",
    "chain_analysis_labels",
    "chain_analysis_datasets",
    "ai_chat_sessions",
];

/// Wallet sources whose checkpoints gate re-importing a book's history.
const WALLET_SOURCES: &str = "('onchain_wallet', 'btcpay_wallet', 'btcpay_provenance')";

const COUNTS: &[(&str, &str)] = &[
    ("transactions", "SELECT COUNT(*) FROM transactions WHERE profile_id = ?"),
    ("chain_analysis_observations", "SELECT COUNT(*) FROM chain_analysis_observations WHERE profile_id = ?"),
    ("chain_analysis_cases", "SELECT COUNT(*) FROM chain_analysis_cases WHERE profile_id = ?"),
    ("chain_analysis_labels", "SELECT COUNT(*) FROM chain_analysis_labels WHERE profile_id = ?"),
    ("chain_analysis_datasets", "SELECT COUNT(*) FROM chain_analysis_datasets WHERE profile_id = ?"),
    ("chain_analysis_dataset_claims", "SELECT COUNT(*) FROM chain_analysis_dataset_claims WHERE dataset_id IN (SELECT id FROM chain_analysis_datasets WHERE profile_id=?)"),
    ("chain_analysis_label_history", "SELECT COUNT(*) FROM chain_analysis_label_history WHERE label_id IN (SELECT id FROM chain_analysis_labels WHERE profile_id=?)"),
    ("chain_observer_instances", "SELECT COUNT(*) FROM chain_observer_instances WHERE profile_id = ?"),
    ("chain_observer_coverage", "SELECT COUNT(*) FROM chain_observer_coverage coverage JOIN chain_observer_instances observer ON observer.id = coverage.observer_id WHERE observer.profile_id = ?"),
    ("chain_observer_values", "SELECT COUNT(*) FROM chain_observer_values observer_value JOIN chain_observer_instances observer ON observer.id = observer_value.observer_id WHERE observer.profile_id = ?"),
    ("custody_components", "SELECT COUNT(*) FROM custody_components WHERE profile_id = ?"),
    ("custody_component_legs", "SELECT COUNT(*) FROM custody_component_legs WHERE profile_id = ?"),
    ("custody_component_allocations", "SELECT COUNT(*) FROM custody_component_allocations WHERE profile_id = ?"),
    ("custody_component_evidence_commitments", "SELECT COUNT(*) FROM custody_component_evidence_commitments WHERE profile_id = ?"),
    ("transaction_tags", "SELECT COUNT(*) FROM transaction_tags tt LEFT JOIN tags tag ON tag.id = tt.tag_id LEFT JOIN transactions tx ON tx.id = tt.transaction_id WHERE tag.profile_id = ? OR tx.profile_id = ?"),
    ("journal_entries", "SELECT COUNT(*) FROM journal_entries WHERE profile_id = ?"),
    ("journal_quarantines", "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id = ?"),
    ("journal_tax_summary", "SELECT COUNT(*) FROM journal_tax_summary WHERE profile_id = ?"),
    ("journal_account_holdings", "SELECT COUNT(*) FROM journal_account_holdings WHERE profile_id = ?"),
    ("journal_wallet_holdings", "SELECT COUNT(*) FROM journal_wallet_holdings WHERE profile_id = ?"),
    ("journal_quantity_postings", "SELECT COUNT(*) FROM journal_quantity_postings WHERE profile_id = ?"),
    ("journal_quantity_issues", "SELECT COUNT(*) FROM journal_quantity_issues WHERE profile_id = ?"),
    ("journal_quantity_balances", "SELECT COUNT(*) FROM journal_quantity_balances WHERE profile_id = ?"),
    ("journal_custody_decisions", "SELECT COUNT(*) FROM journal_custody_decisions WHERE profile_id = ?"),
    ("journal_custody_economic_relations", "SELECT COUNT(*) FROM journal_custody_economic_relations WHERE profile_id = ?"),
    ("journal_custody_gap_inputs", "SELECT COUNT(*) FROM journal_custody_gap_inputs WHERE profile_id = ?"),
    ("journal_custody_reconciliations", "SELECT COUNT(*) FROM journal_custody_reconciliations WHERE profile_id = ?"),
    ("custody_authored_evidence_snapshots", "SELECT COUNT(*) FROM custody_authored_evidence_snapshots WHERE profile_id = ?"),
    ("review_workflow_receipts", "SELECT COUNT(*) FROM review_workflow_receipts WHERE profile_id = ?"),
    ("synthetic_import_duplicate_reviews", "SELECT COUNT(*) FROM synthetic_import_duplicate_reviews WHERE profile_id = ?"),
    ("custody_gap_reviews", "SELECT COUNT(*) FROM custody_gap_reviews WHERE profile_id = ?"),
    ("custody_gap_review_relation_sets", "SELECT COUNT(*) FROM custody_gap_review_relation_sets WHERE profile_id = ?"),
    ("custody_gap_review_transactions", "SELECT COUNT(*) FROM custody_gap_review_transactions WHERE profile_id = ?"),
    ("filed_report_snapshots", "SELECT COUNT(*) FROM filed_report_snapshots WHERE profile_id = ?"),
    ("custody_filed_report_impacts", "SELECT COUNT(*) FROM custody_filed_report_impacts WHERE profile_id = ?"),
    ("custody_filed_report_impact_resolutions", "SELECT COUNT(*) FROM custody_filed_report_impact_resolutions WHERE profile_id = ?"),
    ("transaction_pairs", "SELECT COUNT(*) FROM transaction_pairs WHERE profile_id = ?"),
    ("direct_swap_payouts", "SELECT COUNT(*) FROM direct_swap_payouts WHERE profile_id = ?"),
    ("transaction_pair_dismissals", "SELECT COUNT(*) FROM transaction_pair_dismissals WHERE profile_id = ?"),
    ("swap_matching_rules", "SELECT COUNT(*) FROM swap_matching_rules WHERE profile_id = ?"),
    ("saved_views", "SELECT COUNT(*) FROM saved_views WHERE profile_id = ?"),
    ("bip329_labels", "SELECT COUNT(*) FROM bip329_labels WHERE profile_id = ?"),
    ("attachments", "SELECT COUNT(*) FROM attachments WHERE profile_id = ?"),
    ("tags", "SELECT COUNT(*) FROM tags WHERE profile_id = ?"),
    ("source_funds_sources", "SELECT COUNT(*) FROM source_funds_sources WHERE profile_id = ?"),
    ("source_funds_links", "SELECT COUNT(*) FROM source_funds_links WHERE profile_id = ?"),
    ("source_funds_link_attachments", "SELECT COUNT(*) FROM source_funds_link_attachments WHERE link_id IN ( SELECT id FROM source_funds_links WHERE profile_id = ? )"),
    ("source_funds_source_attachments", "SELECT COUNT(*) FROM source_funds_source_attachments WHERE source_id IN ( SELECT id FROM source_funds_sources WHERE profile_id = ? )"),
    ("source_funds_cases", "SELECT COUNT(*) FROM source_funds_cases WHERE profile_id = ?"),
    ("source_funds_snapshots", "SELECT COUNT(*) FROM source_funds_snapshots WHERE case_id IN ( SELECT id FROM source_funds_cases WHERE profile_id = ? )"),
    ("source_funds_recipients", "SELECT COUNT(*) FROM source_funds_recipients WHERE profile_id = ?"),
    ("ai_chat_sessions", "SELECT COUNT(*) FROM ai_chat_sessions WHERE profile_id = ?"),
    ("ai_chat_messages", "SELECT COUNT(*) FROM ai_chat_messages WHERE session_id IN ( SELECT id FROM ai_chat_sessions WHERE profile_id = ? )"),
];
const DELETES: &[&str] = &[
    "DELETE FROM chain_observer_instances WHERE profile_id = ?",
    "DELETE FROM custody_filed_report_impact_resolutions WHERE profile_id = ?",
    "DELETE FROM custody_filed_report_impacts WHERE profile_id = ?",
    "DELETE FROM custody_gap_review_transactions WHERE profile_id = ?",
    "DELETE FROM custody_gap_review_relation_sets WHERE profile_id = ?",
    "DELETE FROM custody_gap_reviews WHERE profile_id = ?",
    "DELETE FROM filed_report_snapshots WHERE profile_id = ?",
    "INSERT INTO custody_component_purge_authorizations(profile_id) VALUES(?)",
    "DELETE FROM custody_components WHERE profile_id = ?",
    "DELETE FROM custody_component_purge_authorizations WHERE profile_id = ?",
    "DELETE FROM source_funds_snapshots WHERE case_id IN ( SELECT id FROM source_funds_cases WHERE profile_id = ? )",
    "DELETE FROM source_funds_link_attachments WHERE link_id IN ( SELECT id FROM source_funds_links WHERE profile_id = ? )",
    "DELETE FROM source_funds_source_attachments WHERE source_id IN ( SELECT id FROM source_funds_sources WHERE profile_id = ? )",
    "DELETE FROM ai_chat_messages WHERE session_id IN ( SELECT id FROM ai_chat_sessions WHERE profile_id = ? )",
    "DELETE FROM chain_analysis_observations WHERE profile_id = ?",
    "DELETE FROM chain_analysis_cases WHERE profile_id = ?",
    "DELETE FROM chain_analysis_labels WHERE profile_id = ?",
    "DELETE FROM chain_analysis_datasets WHERE profile_id = ?",
    "DELETE FROM source_funds_links WHERE profile_id = ?",
    "DELETE FROM source_funds_cases WHERE profile_id = ?",
    "DELETE FROM source_funds_sources WHERE profile_id = ?",
    "DELETE FROM source_funds_recipients WHERE profile_id = ?",
    "DELETE FROM ai_chat_sessions WHERE profile_id = ?",
    "DELETE FROM saved_views WHERE profile_id = ?",
    "DELETE FROM swap_matching_rules WHERE profile_id = ?",
    "DELETE FROM transaction_pair_dismissals WHERE profile_id = ?",
    "DELETE FROM direct_swap_payouts WHERE profile_id = ?",
    "DELETE FROM transaction_pairs WHERE profile_id = ?",
    "DELETE FROM bip329_labels WHERE profile_id = ?",
    "DELETE FROM journal_wallet_holdings WHERE profile_id = ?",
    "DELETE FROM journal_quantity_balances WHERE profile_id = ?",
    "DELETE FROM journal_custody_decisions WHERE profile_id = ?",
    "DELETE FROM journal_custody_economic_relations WHERE profile_id = ?",
    "DELETE FROM journal_custody_reconciliations WHERE profile_id = ?",
    "DELETE FROM journal_custody_gap_inputs WHERE profile_id = ?",
    "DELETE FROM journal_quantity_issues WHERE profile_id = ?",
    "DELETE FROM journal_quantity_postings WHERE profile_id = ?",
    "DELETE FROM custody_gap_review_transactions WHERE profile_id = ?",
    "DELETE FROM custody_gap_review_relation_sets WHERE profile_id = ?",
    "DELETE FROM custody_gap_reviews WHERE profile_id = ?",
    "DELETE FROM review_workflow_receipts WHERE profile_id = ?",
    "DELETE FROM synthetic_import_duplicate_reviews WHERE profile_id = ?",
    "DELETE FROM custody_filed_report_impact_resolutions WHERE profile_id = ?",
    "DELETE FROM custody_filed_report_impacts WHERE profile_id = ?",
    "DELETE FROM filed_report_snapshots WHERE profile_id = ?",
    "DELETE FROM custody_authored_evidence_snapshots WHERE profile_id = ?",
    "DELETE FROM journal_account_holdings WHERE profile_id = ?",
    "DELETE FROM journal_tax_summary WHERE profile_id = ?",
    "DELETE FROM journal_quarantines WHERE profile_id = ?",
    "DELETE FROM journal_entries WHERE profile_id = ?",
    "DELETE FROM attachments WHERE profile_id = ?",
    "DELETE FROM transaction_tags WHERE tag_id IN (SELECT id FROM tags WHERE profile_id = ?) OR transaction_id IN ( SELECT id FROM transactions WHERE profile_id = ? )",
    "DELETE FROM tags WHERE profile_id = ?",
    "DELETE FROM transactions WHERE profile_id = ?",
];
/// Runs last, after the sync state and rate clean-up, in Python's order.
const INVALIDATE: &str = "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0, last_processed_input_version = 0, journal_input_version = journal_input_version + 1, ownership_review_counts_json = NULL WHERE id = ?";
