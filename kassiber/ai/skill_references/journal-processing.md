# Journal Processing

Use this reference when the user wants tax calculations, journal entries, quarantine review, or transfer pairing.

## Processing order

Standard sequence:

```bash
kassiber wallets sync --wallet <wallet>
kassiber rates sync
kassiber journals process
```

Journal processing recognizes complete native HTLC claims/refunds automatically
when both endpoints are authoritative, quantities conserve, fee timing is
representable and the full evidence graph has no conflict. A positive principal
shortfall across dates remains `native_transition_fee_timing_unresolved`:
neither manual pairing nor a price override supplies the missing in-transit
custody and fee timeline. The book's policy is applied after matching. For
remaining BTC ↔ LBTC peg-ins / peg-outs, provider-only evidence or incomplete submarine routes,
inspect the candidates and review their meaning before relying on reports.

Re-run `kassiber journals process` after:

- imports
- wallet sync
- transfer pairing or unpairing
- exclusion changes
- note or tag changes that affect review flow
- rate overrides

## Process journals

```bash
kassiber journals process
kassiber journals list
```

Use explicit scope flags if needed:

```bash
kassiber journals process --workspace project-satoshi --profile main
```

## Journal events

Inspect entries:

```bash
kassiber journals events list --limit 50
kassiber journals events list --wallet satoshi-liquid --asset BTC --entry-type disposal
kassiber journals events get --event-id <event-id>
```

`journals events list` supports:

- `--wallet`
- `--account`
- `--asset`
- `--entry-type`
- `--start`
- `--end`
- `--cursor`
- `--limit`

When scripting, use `--machine` and follow `next_cursor`.

## Quarantine

Prefer the portable review workflow for an investigation spanning several cases:

```bash
kassiber --machine review cases --limit 20
kassiber --machine review cases --limit 20 --cursor <next_cursor>
kassiber --machine --output review-plan.json review plan --operations-file operations.json --expected-input-version <input_version>
kassiber --machine review apply --artifact-file review-plan.json --idempotency-key <unique-review-key>
kassiber --machine review receipt --idempotency-key <unique-review-key>
```

Use `input_version` from cases, follow `next_cursor` until null, and inspect
transaction/evidence context before writing `operations.json`. The file is an
array of typed operations, for example after verifying the stated invoice:

```json
[{"type":"price_override","transaction_id":"<transaction-id>","fiat_rate":"20000","reason":"Reviewed invoice evidence <evidence-id>"}]
```

A price operation requires exactly one of `fiat_rate` or `fiat_value`, as an exact
decimal string, plus an audit reason. `exclude` requires `transaction_id` and a
reason establishing why the row belongs outside accounting. Never use exclusion
to hide missing evidence, a custody gap, or a transfer. `custody_component` wraps
an existing typed component planner request under `request`; the CLI supports
its actions, while AI may only create components. Reviewed conversion approval
remains unavailable to AI. Unsupported repairs remain unresolved.

In chat use `ui.review.cases`, inspect `ui.transactions.review_context` and
`ui.transfers.review_context`, then `ui.review.plan`. Explain the returned
before/after effects and obtain per-call consent for `ui.review.apply`, passing
the artifact unchanged. The default CLI `core` profile and built-in chat both
advertise this bounded review pack for quarantine questions. Planning is read
only; applying atomically rechecks scope/version/effects, writes, rebuilds, and
stores a durable receipt. Read `ui.review.receipt` after an uncertain response
before retrying with the same idempotency key. A `verified` receipt means the
planned effects were reproduced; inspect `verification.report_ready` and remaining
quarantine before claiming resolution. A stale artifact needs a new plan.

No new background agent or automatic continuation is implied: at a tool/token
budget limit or cancellation, report what was inspected, `next_cursor`, unresolved
evidence, and any receipt/idempotency key needed for the next turn. Re-read cases
if its cursor expires. Do not increase the budget instead of checking an uncertain
write. Selected redacted local facts can reach the configured provider; native
custody-gap/lineage tools remain local-provider only. Portable AI plans reject
path/URL/secret-bearing free text rather than changing a digest-bound artifact;
refer to local evidence IDs. Explicit CLI files retain their exact content.

The older single-row commands remain available:

```bash
kassiber journals quarantined
kassiber journals quarantine show --transaction <transaction-id>
kassiber journals quarantine resolve price-override --transaction <transaction-id> --fiat-rate <rate>
kassiber journals quarantine resolve exclude --transaction <transaction-id>
```

`journals quarantined` has no pagination or `--limit`. The individual AI tool
`ui.journals.quarantine.resolve` repairs reviewed prices or explicit exclusions.
After custody mutations outside `ui.review.apply`, run `ui.journals.process`
and reread quarantine/report blockers; an applied component alone does not
mean the book is resolved.

Clear quarantine state only when the workflow truly calls for it:

```bash
kassiber journals quarantine clear --transaction <transaction-id>
```

## Transfers

Manual transfer pairing is available when auto-detection misses a self-transfer:

```bash
kassiber journals transfers list
kassiber transfers list
kassiber transfers pair --tx-out <txid-or-external-id> --tx-in <txid-or-external-id> --kind manual --policy carrying-value
kassiber transfers unpair --pair-id <pair-id>
```

Use `journals transfers list` to inspect the current computed transfer audit directly. It surfaces same-asset transfer matches with exact sent / received / fee amounts, plus any stored cross-asset pair links, so you do not need to infer pairing from `journals process` counts or from journal rows.

Same-asset carrying-value pairs are supported. Reviewed BTC ↔ LBTC rail changes may carry value on every profile while `bitcoin_rail_carrying_value` is enabled. Austrian policy additionally supports reviewed carrying-value treatment for other eligible crypto conversions. Cross-asset `--policy taxable` pairs stay on the normal SELL + BUY path.

Auto-detection is intentionally conservative: Kassiber only auto-pairs
rows with canonical scoped transaction identity, owned script/outpoint evidence,
or source-qualified Lightning evidence. Arbitrary provider/import ids never
establish ownership. For BTC ↔ LBTC swaps, review the surfaced pair or create
an explicit custody component when the route is incomplete.

Use `transfers components plan --action create` for 1:N, N:1, N:M,
multi-hop migrations, or missing intermediate wallets. Represent missing owned
custody with `untracked_wallet`; genuine N:M requires explicit allocations.
Activate only after exact anchor coverage and conservation pass atomically.
In chat, use `ui.transfers.components.list` to avoid duplicating reviewed state,
then `ui.transfers.components.plan` before asking
for consent to write the final component set with the returned
`expected_input_version`.

If `kassiber --machine journals transfers list` reports
`summary.cross_asset_pairs: 0`, no cross-asset swap pair is active yet.
Do not describe Austrian carry-value as already paired, already reflected in
holdings, or already visible in reports until a pair exists and journals are
reprocessed.

Timing and amount similarity can help identify candidate peg-ins / peg-outs,
but those heuristics are only for review. They do not create a pair on their
own.

## Missing user input

When local evidence is insufficient, call `ui.review.request_input` using current
`ui.review.cases` case IDs and `input_version`. Choose `connect_wallet`,
`import_history`, or `attach_evidence` and explain the specific missing facts
in one or two short sentences: what is missing and why it matters. Use wallet
names and meaningful dates or amounts; omit internal IDs, schema fields and
diagnostic codes. The card supplies the action. End with a brief explanation
instead of repeating a menu of setup instructions or the full investigation.
For `attach_evidence`, select exactly one case. The user-selected file becomes a
durable attachment on that transaction; local analysis uses the same managed
copy. Saving the attachment does not approve its contents or alter accounting.
This read-only tool returns a scoped handoff to an existing local input dialog;
it neither imports data nor approves an accounting interpretation. The
explanation is advisory text, not evidence. Do not include private paths,
descriptors, credentials, or custody graphs. Missing-wallet gap investigation
continues to require a local provider; the generic handoff cannot reveal it.

After issuing the handoff, explain why the requested input is needed and wait.
Do not offer unrelated repair menus or substitute an exclusion. A user may
cancel or supply only part of the history. When the user step completes, read
fresh cases and recorded evidence in the same book, discard stale cursors and
plans, and verify whether the missing facts now exist before proposing changes.
Creating a connection, attaching a document, or completing an import does not by
itself establish basis, custody continuity, or report readiness. A handoff ID is
only a correlation identifier; it is not an application receipt or permission to
write. Standard CLI imports followed by `review cases` use the same accounting
recheck; no background agent runs while the chat is waiting.
