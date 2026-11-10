//! Reports over Python-produced journals. No acquisition or rebuilding.
pub mod artifact_csv;
pub mod artifact_publication;
pub mod artifact_xlsx;
mod austrian;
mod austrian_csv;
pub mod austrian_pdf;
mod austrian_text;
mod austrian_xlsx;
mod calendar;
mod capital_csv;
pub mod ch_tax;
mod commercial;
mod components;
pub mod country;
mod cross_check;
mod cross_check_csv;
mod cross_check_dialect;
mod cross_check_number;
mod cross_check_request;
mod cross_check_xlsx;
pub mod de_tax;
mod desktop;
pub mod es_tax;
pub mod exit_pdf;
pub mod exit_tax;
mod exit_xlsx;
pub mod export;
pub mod export_completion;
pub mod export_preview;
pub mod export_range;
pub mod export_request;
mod full_csv;
pub mod full_pdf;
mod full_xlsx;
mod gain_explanation;
mod gains_desktop;
pub mod gb_tax;
mod history;
mod holdings;
pub mod ie_tax;
mod journal;
pub mod jp_tax;
mod layer_reads;
pub mod legacy_privacy;
mod legacy_privacy_wire;
pub mod lightning;
mod platform_csv;
mod privacy_hygiene;
pub mod privacy_mirror;
pub mod psbt_privacy;
pub mod report_design;
pub mod review_figures;
mod snapshot_write;
mod snapshots;
mod source_funds_assemble;
mod source_funds_assembly;
pub mod source_funds_batch;
pub mod source_funds_bundle;
pub mod source_funds_cases;
mod source_funds_coverage;
pub mod source_funds_disclosure;
mod source_funds_explorers;
pub mod source_funds_links;
pub mod source_funds_mutations;
mod source_funds_order;
pub mod source_funds_pdf;
pub mod source_funds_projection;
mod source_funds_reads;
pub mod source_funds_recipients;
pub mod source_funds_report;
pub mod source_funds_review;
mod source_funds_suggest;
pub mod source_funds_svg;
mod source_funds_text;
pub mod source_funds_traversal;
mod sql;
mod standalone_csv;
mod summary;
pub mod summary_pdf;
mod summary_text;
pub mod tax_estimate;
mod tax_summary;
pub mod tax_years;
mod transaction_csv;
pub mod us_tax;
mod verification;
mod verification_xlsx;
mod year_handoff;
mod year_handoff_xlsx;
pub use kassiber_pytext::zone;

use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use sql::{rows, sql_error};

pub type Result<T> = std::result::Result<T, AppError>;
// Importing the Python tax adapter sets the process Decimal context to 32.
const DECIMAL: Context = Context::new(32, Rounding::HalfEven);

fn value<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}
fn text(row: &Value, key: &str) -> String {
    py_str(value(row, key))
}
fn int(row: &Value, key: &str) -> i64 {
    match value(row, key) {
        Value::Int(n) => i64::try_from(n.clone()).expect("SQLite integer"),
        _ => 0,
    }
}
fn decimal(v: &Value) -> Result<Decimal> {
    if matches!(v, Value::Null) || matches!(v, Value::Str(raw) if raw.as_str() == Some("")) {
        return Ok(Decimal::zero());
    }
    if let Value::Int(n) = v {
        if let Ok(n) = i64::try_from(n.clone()) {
            return Ok(Decimal::from(n));
        }
    }
    let raw = py_str(v);
    raw.trim()
        .parse()
        .map_err(|_| AppError::new("app_error", format!("Invalid decimal value: {raw}")))
}
fn arithmetic(e: kassiber_pydecimal::DecimalError) -> AppError {
    AppError::new("internal_error", e.to_string())
}
fn float(d: &Decimal) -> Result<Value> {
    Ok(Value::Float(d.to_string().parse().map_err(|_| {
        AppError::new("internal_error", "invalid report decimal")
    })?))
}
fn btc(msat: i64) -> Result<Decimal> {
    // Every SQLite integer has at most 19 decimal digits: scaling it by
    // 10^-11 is exact under the reference's precision-32 context. Avoid a
    // general BigInt division and its quotient/trailing-zero work.
    Decimal::from_parts(msat < 0, msat.unsigned_abs().into(), -11).map_err(arithmetic)
}
fn replace(row: &mut Value, key: &str, v: Value) {
    if let Value::Dict(fields) = row {
        if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| *k == key.into()) {
            *slot = v;
        } else {
            fields.push((key.into(), v));
        }
    }
}

/// `report_explanation.result_reference`: the reference
/// `ui.reports.explain_capital_gain` accepts for one journal entry of
/// `profile` (a `profiles` row).
pub fn result_reference(conn: &Connection, profile: &Value, entry_id: &Value) -> Result<Value> {
    gain_explanation::reference(conn, profile, entry_id)
}

/// `journal_calculation.decode_compact(text)`; `None` where Python raises
/// `ValueError` for malformed compact text.
pub fn decode_compact_calculation(text: &str) -> Option<Value> {
    gain_explanation::compact(text)
}

/// Each read holds one SQLite snapshot across admission and report construction.
/// Saved snapshots deliberately remain readable even when live journals are stale.
pub fn execute(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    execute_with_sources(conn, command, args, &kassiber_setup::SystemSources)
}

/// Injectable identities and clock for byte-exact snapshot write oracles.
pub fn execute_with_sources(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    if source_funds_suggest::handles(command) {
        return source_funds_suggest::execute(conn, command, args, sources);
    }
    if source_funds_assemble::handles(command) {
        return source_funds_assemble::execute(conn, command, args, sources);
    }
    if source_funds_cases::handles(command, args) {
        return source_funds_cases::execute(conn, command, args, sources);
    }
    if source_funds_batch::handles(command) {
        return source_funds_batch::execute(conn, command, args, sources);
    }
    if source_funds_recipients::handles(command) {
        return source_funds_recipients::execute(conn, command, args, sources);
    }
    if source_funds_links::handles(command) {
        return source_funds_links::execute(conn, command, args, sources);
    }
    if source_funds_mutations::handles(command) {
        return source_funds_mutations::execute(conn, command, args, sources);
    }
    if matches!(
        command,
        "reports.privacy-mirror" | "ui.reports.privacy_mirror"
    ) {
        return privacy_mirror::execute(conn, command, args, sources);
    }
    if command == "reports.filed-snapshots.create" {
        let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
        return snapshot_write::create_cli(conn, &workspace, &profile, args, sources);
    }
    let outermost = conn.is_autocommit();
    conn.execute_batch("SAVEPOINT report_read")
        .map_err(sql_error)?;
    let result = (|| {
        if command == "ui.privacy_hygiene.snapshot" {
            return legacy_privacy_wire::snapshot(conn, args);
        }
        if matches!(
            command,
            "source-funds.coverage" | "ui.source_funds.coverage"
        ) {
            return source_funds_coverage::execute(conn, command, args);
        }
        if source_funds_review::handles(command) {
            return source_funds_review::execute(conn, command, args);
        }
        if matches!(command, "reports.source-funds" | "ui.source_funds.preview") {
            return source_funds_report::execute(conn, command, args);
        }
        if source_funds_reads::handles(command) {
            return source_funds_reads::execute(conn, command, args);
        }
        if matches!(
            command,
            "reports.balance-history" | "ui.reports.balance_history"
        ) {
            history::validate_interval(args)?;
        }
        if command == "ui.reports.capital_gains" {
            return gains_desktop::report(conn, args, sources);
        }
        if command == "ui.reports.explain_capital_gain" {
            let profile = gains_desktop::active_profile(conn)?.ok_or_else(|| {
                AppError::new("not_found", "Select a book before explaining a report")
            })?;
            let args = get(args, "raw_args").unwrap_or(args);
            if !matches!(args, Value::Null | Value::Dict(_)) {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])));
            }
            return gain_explanation::explain(conn, &profile, value(args, "reference"));
        }
        if command == "reports.cross-check" {
            return cross_check::read_cli(conn, args);
        }
        if command == "ui.reports.cross_check" {
            return cross_check_request::execute(
                conn,
                command,
                args,
                std::path::Path::new("."),
                sources,
            );
        }
        if matches!(command, "reports.psbt-privacy" | "ui.reports.psbt_privacy") {
            // A lone JSON surrogate is non-ASCII PSBT text, never an instruction
            // to discard bytes and analyze a different, potentially valid proposal.
            let psbt = match value(args, "psbt") {
                Value::Str(text) => text.as_str().unwrap_or("\u{fffd}"),
                _ => "",
            };
            let payload = psbt_privacy::report(conn, args, psbt)?;
            return Ok(if truthy(value(args, "table_rows")) {
                psbt_privacy::table_rows(&payload)
            } else {
                payload
            });
        }
        let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
        if matches!(
            command,
            "reports.privacy-hygiene" | "ui.reports.privacy_hygiene"
        ) {
            let report = privacy_hygiene::report(conn, &workspace, &profile)?;
            return Ok(
                if truthy(value(args, "text")) || truthy(value(args, "csv")) {
                    privacy_hygiene::table(&report)
                } else {
                    report
                },
            );
        }
        if command == "reports.explain-capital-gain" {
            let reference = kassiber_pytext::loads(&text(args, "reference"))
                .map_err(|_| AppError::new("validation", "Result reference must be JSON"))?;
            return gain_explanation::explain(conn, &profile, &reference);
        }
        if command == "reports.commercial-subledger" {
            return commercial::report(conn, &profile);
        }
        if command == "reports.filed-snapshots.list" {
            return Ok(dict([(
                "snapshots",
                Value::List(snapshots::list(conn, &text(&profile, "id"))?),
            )]));
        }
        let austrian = matches!(
            command,
            "reports.austrian-e1kv" | "reports.austrian-tax-summary"
        );
        require_context(conn, &profile, austrian)?;
        if command == "ui.reports.balance_sheet" || command == "ui.reports.portfolio_summary" {
            let Value::List(rows) =
                holdings::report(conn, &profile, command == "ui.reports.portfolio_summary")?
            else {
                unreachable!()
            };
            return Ok(desktop::holdings(rows));
        }
        match command {
            "reports.balance-history" | "ui.reports.balance_history" => {
                let rows = history::report(conn, &profile, args)?;
                if command == "ui.reports.balance_history" {
                    Ok(desktop::history(rows, args))
                } else {
                    Ok(Value::List(rows))
                }
            }
            "reports.exit-tax" | "ui.reports.exit_tax_preview" => {
                let report = exit_tax::report(conn, &workspace, &profile, args, sources)?;
                if truthy(value(args, "text")) {
                    Ok(s(exit_tax::render(&report)?))
                } else if truthy(value(args, "csv")) {
                    Ok(value(&report, "lots").clone())
                } else {
                    Ok(report)
                }
            }
            "reports.journal-entries" => journal::entries(conn, &profile),
            "reports.tax-summary" => tax_summary::report(conn, &profile),
            "ui.reports.tax_summary" => {
                let Value::List(rows) = tax_summary::report(conn, &profile)? else {
                    unreachable!()
                };
                Ok(desktop::tax_summary(rows, value(args, "year")))
            }
            "reports.austrian-e1kv" | "reports.austrian-tax-summary" => {
                let report = austrian::report(conn, &workspace, &profile, args)?;
                if truthy(value(args, "csv")) {
                    Ok(value(
                        &report,
                        if command == "reports.austrian-tax-summary" {
                            "summary_rows"
                        } else {
                            "rows"
                        },
                    )
                    .clone())
                } else if truthy(value(args, "text")) {
                    Ok(s(austrian_text::render(&report)?))
                } else {
                    Ok(report)
                }
            }
            "reports.summary" | "ui.reports.summary" => {
                let report = summary::report(conn, &workspace, &profile, args)?;
                if command == "reports.summary" && truthy(value(args, "text")) {
                    Ok(s(summary_text::render(&report)))
                } else {
                    Ok(report)
                }
            }
            "reports.balance-sheet" => holdings::report(conn, &profile, false),
            "reports.portfolio-summary" => holdings::report(conn, &profile, true),
            "reports.capital-gains" => capital_gains(conn, &profile),
            _ => Err(AppError::new(
                "not_implemented",
                format!("{command} is not implemented in the Rust backend yet"),
            )),
        }
    })();
    kassiber_store::debug_assert_no_write(conn, outermost, "report_read");
    let release = conn
        .execute_batch("ROLLBACK TO report_read; RELEASE report_read")
        .map_err(sql_error);
    match result {
        Err(e) => Err(e),
        Ok(v) => {
            release?;
            Ok(v)
        }
    }
}

fn semantics_current(profile: &Value) -> bool {
    get(profile, "required_journal_semantics_version").is_some()
        && get(profile, "journal_semantics_version").is_some()
        && kassiber_store::semantics::compatible(
            int(profile, "required_journal_semantics_version"),
            int(profile, "journal_semantics_version"),
        )
}

fn require_context(conn: &Connection, profile: &Value, austrian: bool) -> Result<()> {
    let id = text(profile, "id");
    kassiber_setup::networks::require_accounting(conn, &id)?;
    let country = kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase();
    if !country::known(&country) {
        return Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{}'", text(profile, "tax_country")),
        ));
    }
    if austrian {
        self::austrian::validate(profile)?;
    }
    let blockers = components::blockers(conn, profile)?;
    if !blockers.is_empty() {
        return Err(AppError::new("custody_component_incomplete", "Reports are blocked by an incomplete or conflicting custody component.")
            .with_hint("Repair or supersede every authored active component in `kassiber transfers components list` before relying on reports.")
            .with_details(dict([("components", Value::List(blockers))])));
    }
    let issues = quantity_issues(conn, &id)?;
    if !issues.is_empty() {
        let blocked_from = issues
            .iter()
            .map(|i| value(i, "blocks_from"))
            .find(|v| truthy(v))
            .cloned()
            .unwrap_or(Value::Null);
        return Err(AppError::new("custody_quantity_unresolved", "Reports are blocked by unresolved custody quantity.")
            .with_hint("Review the custody quantity issues before relying on tax or portfolio reports. Later basis is not final.")
            .with_details(dict([("blocked_from",blocked_from),("issues",Value::List(issues.into_iter().take(20).collect()))])));
    }
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [&id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if !semantics_current(profile)
        || !truthy(value(profile, "last_processed_at"))
        || int(profile, "last_processed_tx_count") != active
        || int(profile, "journal_input_version") != int(profile, "last_processed_input_version")
    {
        return Err(AppError::new(
            "app_error",
            "Reports require fresh journals. Run `kassiber journals process` first.",
        ));
    }
    Ok(())
}

fn quantity_issues(conn: &Connection, id: &str) -> Result<Vec<Value>> {
    rows(conn,"SELECT issue_id,issue_type,state,asset,amount_msat,occurred_at,reason,blocks_from FROM journal_quantity_issues WHERE profile_id=? ORDER BY COALESCE(occurred_at,''),issue_id",[id])
        .map_err(|_| AppError::new("custody_quantity_state_unavailable", "Custody quantity blocker state is unavailable.")
            .with_hint("Repair or migrate the local book, then rebuild journals before relying on reports.")
            .with_details(dict([("profile_id",s(id)),("operation",s("read_journal_quantity_issues")),("error_type",s("OperationalError"))])))
}

const CAPITAL_GAINS_SQL: &str = "SELECT je.id AS journal_entry_id,je.occurred_at,w.label AS wallet,je.transaction_id,je.entry_type,je.asset,ABS(je.quantity) AS quantity,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.gain_loss,0) AS gain_loss,COALESCE(je.description,'') AS description,je.at_category,je.at_kennzahl,je.capital_gains_type FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' ORDER BY je.occurred_at ASC,je.created_at ASC,je.id ASC";

fn capital_gains_rows(conn: &Connection, profile: &Value) -> Result<(Option<String>, Vec<Value>)> {
    let database: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key='database_instance_id'",
            [],
            |r| r.get(0),
        )
        .ok();
    let result = rows(conn, CAPITAL_GAINS_SQL, [text(profile, "id")])?;
    validate_capital_identity(database.as_deref(), !result.is_empty())?;
    Ok((database, result))
}

fn validate_capital_identity(database: Option<&str>, nonempty: bool) -> Result<()> {
    if nonempty {
        database
            .filter(|s| {
                s.len() == 32
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or_else(|| {
                AppError::new(
                    "invalid_project_database",
                    "database instance identity is missing or invalid",
                )
            })?;
    }
    Ok(())
}

fn capital_gains(conn: &Connection, profile: &Value) -> Result<Value> {
    let (database, mut result) = capital_gains_rows(conn, profile)?;
    for row in &mut result {
        let db = database
            .as_deref()
            .expect("validated capital gains identity");
        let reference = dict([
            ("database_id", s(db)),
            ("workspace_id", value(profile, "workspace_id").clone()),
            ("profile_id", value(profile, "id").clone()),
            (
                "input_version",
                Value::Int(int(profile, "journal_input_version").into()),
            ),
            ("processed_at", value(profile, "last_processed_at").clone()),
            ("entry_id", value(row, "journal_entry_id").clone()),
        ]);
        let quantity = int(row, "quantity");
        // No crypto field before 1 March 2022, as Reports states it.
        let stated =
            austrian::stated_kennzahl(value(row, "at_kennzahl"), &text(row, "occurred_at"))?;
        replace(row, "at_kennzahl", stated);
        if let Value::Dict(fields) = row {
            fields.retain(|(k,v)| *k != "journal_entry_id".into() && !(matches!(k,kassiber_pyjson::Key::Str(k) if k.as_str().is_some_and(|k|["at_category","at_kennzahl"].contains(&k))) && matches!(v,Value::Null)));
        }
        replace(row, "explanation_reference", reference);
        replace(row, "quantity_msat", Value::Int(quantity.into()));
        replace(row, "quantity", float(&btc(quantity)?)?);
    }
    Ok(Value::List(result))
}

/// A book whose latest time holds three BTC prices: file order z (30000),
/// y (20000), x (10000), while ids and creation times say x, y, z. Every
/// latest-transaction-rate read takes x, the file's last row.
#[cfg(test)]
pub(crate) fn same_time_rate_book() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let creation: serde_json::Value =
        serde_json::from_str(include_str!("../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','W','t');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','P','EUR','t');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('w','ws','p','Cold','custom','t');
        INSERT INTO import_batches(id,workspace_id,profile_id,source_format,imported_at) VALUES('b','ws','p','csv','t');").unwrap();
    for (position, id, created, rate) in [
        (1, "z", "2026-01-03", "30000"),
        (2, "y", "2026-01-02", "20000"),
        (3, "x", "2026-01-01", "10000"),
    ] {
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fiat_currency,fiat_rate,fiat_rate_exact,created_at) VALUES(?1,'ws','p','w',?1,'2024-06-01T00:00:00Z','inbound','BTC',100000000000,'EUR',?3,?3,?2)", kassiber_store::rusqlite::params![id, created, rate]).unwrap();
        conn.execute("INSERT INTO import_batch_transactions(batch_id,transaction_id,source_position) VALUES('b',?,?)", kassiber_store::rusqlite::params![id, position]).unwrap();
    }
    conn
}

#[cfg(test)]
mod amount_tests {
    use super::*;
    #[test]
    fn scaled_sqlite_quantities_match_context_division() {
        for msat in [
            i64::MIN,
            i64::MIN + 1,
            -1_000_000_000_000,
            -1_000,
            -1,
            0,
            1,
            10,
            1_000,
            9_007_199_254_740_993,
            i64::MAX,
        ] {
            let divided = Decimal::from(msat)
                .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
                .unwrap();
            let scaled = btc(msat).unwrap();
            assert_eq!(scaled, divided);
            assert_eq!(float(&scaled).unwrap(), float(&divided).unwrap());
            // Scale representation can differ; subsequent precision-32
            // arithmetic must preserve the same numeric and float results.
            for base in [
                "0",
                "1.2345678901234567890123456789012",
                "-9999999999999999999999.123456789",
            ] {
                let base: Decimal = base.parse().unwrap();
                assert_eq!(
                    float(&base.add(&scaled, &DECIMAL).unwrap()).unwrap(),
                    float(&base.add(&divided, &DECIMAL).unwrap()).unwrap()
                );
            }
        }
    }
}
