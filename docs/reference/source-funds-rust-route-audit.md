# Source-funds Rust route audit

R5 continuation 5 accounts for every source-funds CLI command and desktop
daemon kind. There are 19 native CLI leaves and 21 native daemon kinds. Report
saving is a variant of the report CLI leaf. PDF rendering and PDF-containing
bundles remain Python-owned. AI chat and tool execution remain Python-owned
under the owner's R6 scope decision.

| CLI command | Daemon kind | Served by |
| --- | --- | --- |
| `reports.source-funds` | `ui.source_funds.preview` | Rust |
| `reports.source-funds --save-case` | `ui.source_funds.cases.save` | Rust |
| `source-funds.sources.list` | `ui.source_funds.sources.list` | Rust |
| `source-funds.sources.create` | `ui.source_funds.sources.create` | Rust |
| `source-funds.sources.attach` | `ui.source_funds.sources.attach` | Rust |
| `source-funds.links.list` | `ui.source_funds.links.list` | Rust |
| `source-funds.links.create` | `ui.source_funds.links.create` | Rust |
| `source-funds.links.review` | `ui.source_funds.links.review` | Rust |
| `source-funds.links.attach` | `ui.source_funds.links.attach` | Rust |
| `source-funds.links.bulk-review` | `ui.source_funds.links.bulk_review` | Rust |
| `source-funds.recipients.list` | `ui.source_funds.recipients.list` | Rust |
| `source-funds.recipients.create` | `ui.source_funds.recipients.create` | Rust |
| `source-funds.recipients.update` | `ui.source_funds.recipients.update` | Rust |
| `source-funds.recipients.delete` | `ui.source_funds.recipients.delete` | Rust |
| `source-funds.cases.list` | `ui.source_funds.cases.list` | Rust |
| `source-funds.review-context` | `ui.source_funds.review_context` | Rust |
| `source-funds.request-input` | `ui.source_funds.request_input` | Rust |
| `source-funds.coverage` | `ui.source_funds.coverage` | Rust |
| `source-funds.suggest` | `ui.source_funds.suggest` | Rust |
| `source-funds.assemble` | `ui.source_funds.assemble` | Rust |
| — | `ui.source_funds.evidence.list` | Rust |
| `reports.export-source-funds-pdf` | `ui.source_funds.export_pdf` | Python |
| `reports.export-source-funds-bundle` | `ui.source_funds.export_bundle` | Python |

`tests/test_rust_source_funds_routes.py` compares this inventory with Python's
argparse catalog and supported daemon kinds, then pins all three native
registration lists. Its process sweep checks both SQLite and SQLCipher,
complete response bytes under the established honest-native diagnostic
contract, all 112 tables and unchanged attachment hashes. Each binding runs
20 native CLI calls including report saving and one native daemon session
containing all 21 kinds. Every native entry has zero hybrid fallbacks. The two
CLI PDF routes and two daemon PDF routes explicitly run through Python.

The sweep uses missing scoped references for CLI mutations and an invalid
target for transaction-scoped daemon requests. Successful writes, frozen
reports, managed evidence and late rollback have their own source, recipient,
link, batch, case-save and persisted-pass oracles. Native book admission
continues to reject unknown schema objects; trigger-based failure injection
runs against connection-scoped services on both bindings.

The AI executor's `_SOURCE_FUNDS_MUTATING_AI_DAEMON_KINDS` branch calls
`_ui_source_funds_payload_from_conn` directly inside `_run_scoped_ai_mutation`.
It never crosses the native CLI/daemon transport. Python owns advertised tool
schemas, explicit mutation/egress consent, duplicate-call suppression, frozen
book/input binding and redaction, including source admissions and case saves.
The virtual `ui.source_funds.export` tool also remains Python-owned. Direct
CLI/desktop services retain validation, exact amounts, scope and reviewed-save
fingerprint checks. Their Python handlers do not enforce AI consent or
once-only execution outside the AI loop, so this port adds no native AI loop.

The [persisted-pass checkpoint](rust-shell.md#source-funds-persisted-pass-checkpoint-r5)
records the differential, rollback and named-mutation evidence. No ownership,
custody, tax or source-of-funds authority is inferred from route selection.
