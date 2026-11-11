# Rust port: audit package and Privacy Mirror

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

**Status:** native in Rust and registered (2026-11-06). This page lists every
entry point the two Python modules serve, what the package contains, what
is privacy-sensitive, and where the Rust port differs on purpose. Methodology
stays in [Privacy Mirror](privacy-mirror.md), the evidence summary's contract
in [Source-of-funds review](source-of-funds-review.md) and the AI boundary in
[AI](ai.md).

## Entry points

| Surface | Python | Rust | Evidence |
| --- | --- | --- | --- |
| `ui.audit.evidence.summary` (daemon kind and AI read tool) | `audit_package.build_evidence_summary` | `kassiber_journals::evidence_summary` (earlier port) | `tests/test_rust_evidence_summary.py` |
| `ui.reports.export_audit_package` (daemon kind) | `audit_package.export_audit_package` via `daemon._ui_report_export_payload_from_conn` | `kassiber_journals::audit_package` | `tests/test_rust_audit_package.py` |
| AI `ui.reports.export` with `report=audit_package`, `format=package` | `_ai_report_export_target` → the kind above | `kassiber_ai::mutate::report_export_target` → the kind above | `tests/test_rust_ai_tools.py`, `crates/ai/tests/fences.rs` |
| Custody AI audit rows (`custody_ai_audit.py`) | `append_assistance_record`, `redacted_audit_summary` | `kassiber_transfers::custody_ai_audit`; the summary's `custody_ai_assistance` section | earlier port |
| `reports privacy-mirror` (CLI, every `--format`) | `core_reports.report_privacy_mirror` (redacted) and `privacy_mirror_table_rows` | `kassiber_reports::privacy_mirror` over `kassiber_wallets::analysis_index` | `tests/test_rust_privacy_mirror_process.py` |
| `ui.reports.privacy_mirror` (daemon kind) | the same, local desktop audience | the same | as above |
| AI `ui.reports.privacy_mirror` (read tool) | `_reports_privacy_mirror_payload(..., redacted=True)` | `kassiber_ai::projections` → `reports.privacy-mirror` | `tests/test_rust_ai_tools.py` |
| Linkage analysis (`privacy_linkage.py`) | `build_privacy_linkage_graph`, `analyze_psbt_privacy` | `kassiber_reports::psbt_privacy` (earlier port) | reached only through `reports psbt-privacy` and `ui.reports.psbt_privacy`, which are native |

There is no CLI leaf for the audit package; the desktop and the assistant
reach it through the daemon kind. The package renders no report, so the
[shared report design](report-design.md) has nothing to replace.

## Package contents

`ui.reports.export_audit_package` writes one directory under the exports
root, `reports/kassiber-audit-package-<YYYYmmdd-HHMMSS>[-N]`:

- `manifest.json`: the evidence summary (`schema_version` 2, `workspace`,
  `profile`, `scope`, `journal_freshness`, `custody_quantity`,
  `authored_custody_evidence`, the filed-report sections, review history,
  `custody_ai_assistance`, `review_workflow_receipts`,
  `schema_migration_audits`, `transactions`, `summary`,
  `excluded_sensitive_material`) plus `package`: `schema_version`,
  `generated_at`, the five options, `evidence_files`, `url_references` and
  `warnings`. Written as `json.dumps(json_ready(manifest), indent=2,
  sort_keys=True)` and a newline: ASCII-escaped, byte-identical to Python.
- `evidence/<safe id>-<sha256(id)[:12]>-<safe name>`: a copy of each managed
  file the scope names (direct evidence, and with review state the evidence
  of reviewed links and their root sources). Names keep CPython `str.isalnum`
  characters and `._-`; the id digest keeps two ids from colliding.

Options (`include_copied_attachments`, `include_url_references`,
`include_journal_state`, `include_review_state`, `include_edit_history`) and
scope (`transaction`, `transactions` or `source_funds_case`) are validated
in Python's order. The assistant may pass only the scope fields.

The response is Python's: `dir`, `manifest`, `format` (`directory`), `scope`
(`audit_package`), `filename`, the transaction, ready and blocked counts,
`evidence_file_count`, `url_reference_count` and the package warnings.

## Hashing

Each managed file is hashed (SHA-256) and withheld with
`attachment_hash_mismatch` when its stored hash differs; a missing file or a
storage path outside the attachments root is withheld with its own blocker.
The manifest records the hash and size of every copy. Authored custody
evidence appears as hashes only. Nothing is signed, in Python or in Rust.

## Redaction and what is sensitive

- Never in a package: descriptors, xpubs, backend URLs and credentials, raw
  wallet files and configuration, environment files, logs, AI settings, raw
  AI custody proposals (hashes and consent outcomes only), unrelated books,
  raw technical wallet evidence and chain observer state. The summary does
  not carry them, so the export cannot copy them.
- URL references stay literal and are never fetched. Embedded credentials,
  secret-like query keys and secret-like fragments redact the reference:
  `url` becomes empty and `redacted_url` carries the parts with `REDACTED`
  values, built with urllib's exact parse, `parse_qsl`, `urlencode` and
  `urlunparse` round trip (including the dropped empty `;params`).
- Unreviewed source-of-funds links keep `review_details_redacted`; bounded
  scopes exclude profile-wide freshness, custody totals and report-wide
  filed-report payloads.
- The assistant receives `redact_evidence_for_ai` of the response: local
  paths (`dir`, `manifest`) and URL targets never reach the model.
- The Mirror's CLI and AI audiences are `ai_export_safe` (graph subjects
  behind process- and book-bound `ca-ref:` handles); the desktop kind keeps
  the local audience. Both are local analysis: no wallet sync, provider or
  network access, with the product offline switch on or off.

## Publication

Python creates the package directory, then writes into it as it goes. Rust
creates the same empty directory at the same point, so a refused summary
leaves what Python leaves and the next managed name gets the same counter,
then assembles the package in a hidden sibling (`.<name>.staging`), syncs
every file and directory, and renames it over the empty directory only when
complete. Storage failures answer `export_write_failed` with the package
directory as `destination` and the `errno`.

## Differences from Python

| Case | Python | Rust |
| --- | --- | --- |
| A write, read or rename fails | an untyped `OSError` crashes the request | `export_write_failed`; no partial package |
| An evidence file changes between its hash check and its copy | the new bytes are copied under the old hash | the copy is hashed as written and the export fails |
| Extended attributes of copied evidence | copied by `shutil.copy2` on Linux | not copied (contents, permission bits and times are) |
| Assistant PDF and package targets | exported | exported; PDF envelopes differ only in the [report design's declared fields](report-design.md#envelope-differences) |

## Registration fixes found by the process tests

Registering the Mirror compared unsorted wire bytes for the first time. Two
key-order differences, invisible to the earlier sorted-key corpora, now
follow Python: entropy rows end with `investigation`, and projected findings
list `source_ids` last (Python pops and reassigns it).

## Evidence

- `tests/test_rust_audit_package.py`: three daemon sessions (whole book and
  every option combination, bounded scopes and refusals, SQLCipher) with
  byte-identical responses, identical exported trees (manifest bytes, copied
  bytes, permission bits and copied modification times) and all 112 tables
  unchanged. The corpus covers hash mismatches, missing and escaping storage
  paths, blank names, a hostile attachment id and fifteen URL shapes.
- `tests/test_rust_privacy_mirror_process.py`: CLI JSON, table, plain and
  CSV and daemon sessions on empty, stored and owned books with dataset
  claims, cold, warm and incremental publications, invalid scope and
  arguments, offline off and on, plaintext and SQLCipher; stdout bytes and
  all 112 tables after every step.
- `tests/test_rust_evidence_summary.py` now places its managed file in the
  book's attachments root (it had written it elsewhere, so every attachment
  compared as missing) and checks that it is found.
- `tests/test_rust_ai_tools.py::test_pdf_and_audit_package_export_targets_run_natively`
  and the export cases of the mutating-tool comparison.
- Mutation entries, all killed by the named tests:
  `crates/daemon/scripts/desktop_projection_mutations.json` (seven `audit-*`,
  three `mirror-*`) and `crates/ai/scripts/mutations.json`
  (`B-mutate-audit-package-target`, `B-mutate-source-funds-case-strip`).
