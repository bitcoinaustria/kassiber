#!/usr/bin/env python3
"""Write `leaves_mutations.json`, refusing any `find` that is not unique."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
T = "tests/test_rust_cli_leaves.py::"
JOURNAL = T + "test_journal_entries_and_events_match_python_byte_for_byte[False]"
AUDIT = T + "test_transfer_audit_matches_python_byte_for_byte[False]"
QREAD = T + "test_quarantine_reads_match_python_byte_for_byte[False]"
CLEAR = T + "test_quarantine_clear_matches_python_envelope_and_book[root-False]"
REFUSED = T + "test_quarantine_resolve_refusals_match_python_without_writes"
IDENT = T + "test_identify_matches_python_in_every_format[False]"
REVEAL = T + "test_reveal_descriptor_matches_python_and_keeps_its_gates[False]"
CLI = "crates/journals/src/cli.rs"
VIEWS = "crates/journals/src/views.rs"
STORE = "crates/kassiber/src/real_store/journals.rs"
IDENTIFY = "crates/wallets/src/identify.rs"
WALLETS = "crates/wallets/src/lib.rs"
CLIW = "crates/cli/src/wallets.rs"
RUN = "crates/cli/src/run.rs"


def resolve(argv_id):
    return T + f"test_quarantine_resolve_matches_python_envelope_and_book[{argv_id}-False]"


ENTRIES = [
    ("JL-limit-positive", CLI, "if limit.is_some_and(|limit| *limit <= BigInt::from(0)) {",
     "if limit.is_some_and(|limit| *limit < BigInt::from(0)) {", [JOURNAL],
     "`journals list --limit 0` is refused, not an empty page."),
    ("JL-order", CLI, "            WHERE je.profile_id = ? {cursor_sql}\n            ORDER BY je.occurred_at DESC,",
     "            WHERE je.profile_id = ? {cursor_sql}\n            ORDER BY je.occurred_at ASC,", [JOURNAL],
     "Entries list newest first."),
    ("JL-markers", CLI, '(name == "at_category" || name == "at_kennzahl")', '(name == "at_category")', [JOURNAL],
     "An absent Austrian Kennzahl is dropped from the row, as Python pops it."),
    ("JL-quantity-msat", CLI, '    fields.push((Key::from("quantity_msat"), Py::Int(quantity)));\n    Ok(Py::Dict(fields))\n}\n\n/// A row',
     '    Ok(Py::Dict(fields))\n}\n\n/// A row', [JOURNAL], "Every entry carries its integer msat quantity."),
    ("JL-meta", RUN, '"metadata.bip329.list" | "transactions.list" | "journals.list"',
     '"metadata.bip329.list" | "transactions.list"', [JOURNAL],
     "`journals list` pages through envelope meta, not the payload."),
    ("JE-limit-max", CLI, "    if limit > BigInt::from(MAX_EVENTS_LIMIT) {", "    if limit >= BigInt::from(MAX_EVENTS_LIMIT) {",
     [JOURNAL], "An events page may hold exactly 1000 rows."),
    ("JE-asset-case", CLI, "nonempty(filters.asset).map_or_else(String::new, kassiber_pytext::upper)",
     "nonempty(filters.asset).map_or_else(String::new, str::to_owned)", [JOURNAL],
     "The asset filter is case-insensitive."),
    ("JE-wallet-filter", CLI, '        conditions.push("je.wallet_id = ?".into());', '        conditions.push("je.wallet_id <> ?".into());',
     [JOURNAL], "The wallet filter keeps the wallet's events."),
    ("JE-start-bound", CLI, 'for (bound, op) in [(&start, ">="), (&end, "<=")]', 'for (bound, op) in [(&start, ">"), (&end, "<=")]',
     [JOURNAL], "Timestamp bounds are inclusive."),
    ("JG-compact-hidden", CLI, '            "calculation_compact" => continue,\n', "", [JOURNAL],
     "`journals events get` never prints the compact column."),
    ("JT-fee-once", CLI, '            fee = int_or_zero(row.get("source_fee_msat"))?;\n', "", [AUDIT],
     "A source's fee is booked on its first transfer."),
    ("JT-group-empty", CLI, '            if truthy(row.get("atomic_group_id")) {', '            if !matches!(row.get("atomic_group_id"), Sql::Null) {',
     [AUDIT], "An empty atomic group id is no transfer group."),
    ("JT-current", CLI, '("processed_journals_current", field("is_current"))', '("processed_journals_current", field("needs_processing"))',
     [AUDIT], "The processing status reports whether journals are current."),
    ("JT-tax-gate", CLI, "    require_tax_processing(&profile)?;\n", "",
     [T + "test_transfer_audit_tax_country_gate_matches_python[unsupported]"],
     "An unsupported tax country refuses the transfer audit, as it refuses processing."),
    ("JT-payout-amount", CLI, '("payout_amount", Py::Float(btc_of(&payout_amount)?)),',
     '("payout_amount", Py::Float(btc_of(&swap_fee)?)),', [AUDIT], "Direct payouts report the payout leg's amount."),
    ("JQ-order", CLI, "        WHERE q.profile_id = ?\n        ORDER BY t.occurred_at DESC\",", "        WHERE q.profile_id = ?\n        ORDER BY t.occurred_at ASC\",",
     [QREAD], "Quarantines list newest first."),
    ("JQ-explained", CLI, "            None => fields.push((key.clone(), value.clone())),", "            None => {}",
     [QREAD], "Each held row carries its explained cause, root and actions."),
    ("JQ-detail-default", CLI, '        _ if !truthy(value) => "{}".to_owned(),', '        _ if !truthy(value) => "null".to_owned(),',
     [T + "test_quarantine_reads_parse_stored_detail_as_python[]"], "An empty stored detail reads as `{}`."),
    ("JQ-show-held", CLI, 'format!("Transaction \'{reference}\' has no active quarantine"),', 'format!("Transaction \'{reference}\' is not quarantined"),',
     [QREAD], "`quarantine show` names a row with no active quarantine."),
    ("JC-invalidate", CLI, "        kassiber_wallets::invalidate_journals(conn, scope.profile_id)?;\n        Ok(dict([\n            (\"transaction_id\", s(id.as_str())),\n            (\"resolution\", s(\"clear\")),",
     "        Ok(dict([\n            (\"transaction_id\", s(id.as_str())),\n            (\"resolution\", s(\"clear\")),",
     [CLEAR], "Clearing a quarantine invalidates the journals."),
    ("JC-held", CLI, "        let (id, _) = transaction_id(conn, scope, reference)?;\n        ensure_quarantined(conn, scope.profile_id, &id)?;\n",
     "        let (id, _) = transaction_id(conn, scope, reference)?;\n", [CLEAR, T + "test_quarantine_clear_refuses_a_row_that_is_not_held"],
     "Only a held row can be cleared."),
    ("JR-no-price", CLI, '            return Err(AppError::new("validation", "Provide a fiat rate or value"));', "", [REFUSED],
     "A price override needs a reviewed rate or value."),
    ("JR-both-value", VIEWS, "                Parsed::Finite(total) if !total.is_negative() => Ok((Some(rate), Some(total))),",
     "                Parsed::Finite(total) => Ok((Some(rate), Some(total))),", [REFUSED],
     "With both prices given, a negative value is still refused."),
    ("JR-both-kept", VIEWS, "                Parsed::Finite(total) if !total.is_negative() => Ok((Some(rate), Some(total))),",
     "                Parsed::Finite(total) if !total.is_negative() => Ok((Some(rate), None)),",
     [resolve("price-override-transaction-a-fiat-rate-10-fiat-value-2")], "Both given prices are stored as given."),
    ("JR-unchanged-invalidates", CLI, '        if kassiber_envelope::get(&record, "updated") != Some(&Py::Bool(true)) {',
     '        if kassiber_envelope::get(&record, "updated") == Some(&Py::Bool(true)) {',
     [resolve("exclude-transaction-t")], "Journals are invalidated only when the metadata edit changed nothing."),
    ("JR-source", STORE, '                            source: "cli",\n                            reason,\n                        },\n                    );',
     '                            source: "gui",\n                            reason,\n                        },\n                    );',
     [resolve("exclude-transaction-t")], "CLI resolutions are authored by the CLI."),
    ("JR-note", STORE, '                    payload.retain(|(key, _)| *key != "note".into());\n', "",
     [resolve("exclude-transaction-t-reprocess")], "A reprocessed resolution drops the process-journals note."),
    ("JR-group-root", STORE, "                    Some(group) => {\n                        kassiber_journals::quarantine::groups::root_transaction_for_group(\n                            conn,\n                            &profile_id,\n                            group,\n                        )?\n                    }",
     "                    Some(group) => group.clone(),", [resolve("exclude-group-id-root:t")], "A root group resolves its root transaction."),
    ("WI-wallet-filter", IDENTIFY, '        format!(" AND w.id IN ({})", vec!["?"; wallet_ids.len()].join(", "))',
     "        String::new()", [IDENT], "`--wallet` restricts the scan to the selected wallets."),
    ("WI-scan-negative", IDENTIFY, "    if args.scan_to_index < BigInt::from(0) {", "    if args.scan_to_index < BigInt::from(-1) {",
     [IDENT], "A negative scan depth is refused."),
    ("WI-empty-inputs", IDENTIFY, "        && !given(&csv_text)\n", "        && given(&csv_text)\n", [IDENT],
     "Identify without any input is refused."),
    ("WI-nul", IDENTIFY, "    if text.contains('\\0') {", "    if text.contains('\\u{1}') {", [IDENT],
     "A candidate file with NUL bytes is refused as binary."),
    ("WI-flat-note", IDENTIFY, '        ("note", or_empty(item, "note")),\n    ])\n}', '        ("note", s("")),\n    ])\n}', [IDENT],
     "Flat identify rows keep the note."),
    ("WI-json-full", CLIW, "    if format == crate::render::Format::Json {\n        return report;", "    if format != crate::render::Format::Json {\n        return report;",
     [IDENT], "JSON keeps the full identify report; other formats flatten it."),
    ("WR-material-refusal", CLIW, "        Some(material) if kassiber_envelope::truthy(material) => {", "        Some(material) => {",
     [REVEAL], "`--material-only` refuses a wallet without descriptor material."),
    ("WR-config", WALLETS, '        ("config", config),\n    ]))', '        ("config", Value::Null),\n    ]))', [REVEAL],
     "The reveal prints the whole stored configuration."),
    ("WR-strip", WALLETS, "        .map(|value| kassiber_pytext::strip(&kassiber_envelope::py_str(value)).to_owned())",
     "        .map(|value| kassiber_envelope::py_str(value))", [REVEAL], "Descriptor material is stripped."),
    ("WA-negative-rows", CLIW, "        Some(Value::Int(_)) => 0,", "        Some(Value::Int(_)) => analysis::DEFAULT_SAMPLE_ROWS,",
     [T + "test_analyze_file_matches_python_without_a_book[--file-ledger.csv---sample-rows--4]"],
     "A negative sample size shows no rows."),
    ("WA-kind", RUN, '.and_then(|payload| emit(&mut sink, format, "wallets.file_analysis", payload)),',
     ".and_then(|payload| emit(&mut sink, format, &kind, payload)),",
     [T + "test_analyze_file_matches_python_without_a_book[default]"], "File analysis answers kind wallets.file_analysis."),
    ("WA-no-book", RUN, '        "wallets.analyze-file" => store', '        "wallets.analyze-file-unrouted" => store',
     [T + "test_analyze_file_matches_python_without_a_book[default]"], "File analysis opens no book."),
    ("MP-plan-kind", RUN, '.or(plan_only.then_some("maintenance.purge-legacy-ledger.plan")),', ".or(None),",
     [T + "test_purge_legacy_ledger_matches_python_envelope_and_book[False]"],
     "Without --confirm the purge only plans, under its plan kind."),
    ("MP-confirm-only", "crates/kassiber/src/real_store.rs", "        let plan = if confirm {\n            kassiber_store::retired::purge_legacy_ledger(conn)",
     "        let plan = if true {\n            kassiber_store::retired::purge_legacy_ledger(conn)",
     [T + "test_purge_legacy_ledger_matches_python_envelope_and_book[False]"], "Nothing is removed without --confirm."),
    ("MP-empty-tables", "crates/store/src/retired.rs", "        tables: counted.into_iter().filter(|(_, rows)| *rows != 0).collect(),",
     "        tables: counted.into_iter().collect(),",
     [T + "test_purge_legacy_ledger_matches_python_envelope_and_book[False]"], "The plan lists only tables that hold rows."),
    ("ST-legacy-open", "crates/store/src/schema_structure.rs", "        if legacy_ledger(&kind, &name, &table, sql.as_deref()) {\n            continue;\n        }\n", "",
     [T + "test_purge_legacy_ledger_matches_python_envelope_and_book[False]"],
     "A book that keeps populated ledger archives opens natively."),
    ("ST-legacy-shipped-only", "crates/store/src/schema_structure.rs",
     "        \"trigger\" => sql.is_some_and(|sql| {",
     "        \"trigger\" => ledger(name) || sql.is_some_and(|sql| {",
     ["populated_legacy_ledger_archives_pass_the_structural_check",
      ],
     "Only the gl_ triggers Kassiber shipped are accepted; any other is schema drift."),
    ("CLI-registered", "crates/cli/src/describe.rs", '    "journals.transfers.list",\n', "",
     [AUDIT], "Every leaf but the gated chat and MCP ones is native."),
]


def main():
    out = []
    for ident, path, find, replace, tests, rule in ENTRIES:
        text = (ROOT / path).read_text()
        count = text.count(find)
        if count != 1:
            raise SystemExit(f"{ident}: find occurs {count} times in {path}")
        out.append(dict(id=ident, file=path, find=find, replace=replace, expect_failing=tests, rule=rule))
    (Path(__file__).with_name("leaves_mutations.json")).write_text(json.dumps(out, indent=2) + "\n")


if __name__ == "__main__":
    main()
