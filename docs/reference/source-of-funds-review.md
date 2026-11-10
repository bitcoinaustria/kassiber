# Source-of-funds review

The desktop follows one case through four steps: choose funds, trace and resolve,
review the report, and export. It starts with an existing transaction and the
full amount; a partial amount, planned-sale purpose or exact reference can be
set explicitly. Only the current step is shown. The canonical recorded flow
is the primary trace view, with expandable stations and transaction details;
open findings and an assistant action lead to missing evidence. Manual source
and link tools remain available on demand.

The report step shows a concise disclosure summary and the shared interactive
evidence graph, using only the report's disclosure-filtered flow. Exact references,
recipient and detail controls, and original PDF graphics are expandable. Print
graphics retain their original palette in either app theme. Continuing to export requires
a current exportable review, an available printable preview and explicit preview
confirmation. Approval is bound to the review fingerprint: changed evidence,
amount or disclosure options require another review. A restored draft step is
not approval. Truncated reviews, stale/error responses and preview rendering
errors cannot authorize export. Saved PDF and ZIP receipts describe the frozen
case, not future edits. No step sends information to a bank or external service.

`core/source_funds_review.py` composes the existing source-funds report with
target-reachable links, sources and attachments under one database read
snapshot. It does not interpret custody or tax independently. The result names
the canonical book and target, the normalized report recipe, current findings,
and a review fingerprint. Bounded record lists explicitly report truncation.
The fingerprint covers provenance and evidence edits even when those edits do
not invalidate journals. Coverage shares immutable inputs only for one request;
there is no persistent report cache that can outlive a book change.
A smaller target amount follows a single complete, explicit same-asset route
with exact millisatoshi ratios; this changes only the report projection, not
the authored allocation. Multiple possible sources and ratios requiring
rounding remain unresolved.

Stored source, link and case listings refuse fractional or nonfinite
millisatoshi quantities with `validation`. They do not truncate legacy values
into a different BTC display amount or rewrite the retained evidence.

A report states the gross upstream demand its reviewed routes require, per
source denomination, and says so explicitly when that gross differs from the
selected target. It claims a target composition share only when the gross
equals the target in a single asset matching it. A route difference is named as
network fees only when the disclosed rows actually account for it: every hop
that consumed value must have consumed exactly the fee its own row records, the
totals must agree, and no privacy boundary may sit on the route. Otherwise it
stays reported as a difference, never silently reclassified as a fee. The
verdict is computed once and stored as
`allocations.route_difference_explanation`, so a saved case renders what it was
saved with rather than whatever today's rules would conclude. Source denominations are
never summed together, so a peg-out route funding a BTC target from an L-BTC
root shows both assets rather than one target-denominated total. A `unknown`
root source is exportable but establishes no economic origin, so coverage
reports it in its own `unattributed` bucket rather than as fully traced or
attested, and the report carries a matching `unknown_origin` warning naming the
unattributed amount. Cases saved before source assets and gross demand were
recorded are not re-exported; re-run the report and save the case again.

The current Python reference has a known discrepancy in the fee explanation:
its transaction-node projection omits the privacy-boundary flag that the fee
guard checks, and the guard does not inspect privacy link types. A reviewed
CoinJoin route can therefore retain its privacy warning while its difference
is still named as network fees. The synthetic
[reproduction](../../crates/reports/scripts/repro_python_source_funds_fee_privacy.py)
pins this behavior without changing it. Resolving it is separate from the Rust
read port; the privacy restriction above remains the intended contract.

Reviewing a link is not editing it. The editor is built from exact stored
millisatoshis rather than an 8-decimal projection, only fields the reviewer
deliberately changed are sent, a rejection may not carry an allocation edit, and
`ui.source_funds.links.review` accepts optional
`expected_allocation_amount_msat` / `expected_from_allocation_amount_msat`
preconditions so an approval cannot land on amounts that changed since they were
inspected (`source_funds_link_stale`). Accepting a link still records an
explicit allocation policy. Batch eligibility has one definition, published as
`bulk_review.eligible_link_ids` on the inspection and used to bound the apply;
`eligible_beyond_inspection` counts eligible hops beyond the reviewed frontier.

Reviewed branching paths are evaluated in dependency order: every downstream
allocation reaches a shared ancestor before its root sources are consumed.
An additional intermediate wallet therefore cannot change coverage merely by
changing traversal order. The planner reads only the bounded target graph;
cycles, depth/size limits, insufficient source amounts and ambiguous partial
source selection still block export.

When complete native history realizes an existing reviewed custody shortcut,
the journal stores a local reconciliation certificate against that component
revision and input version. Every native claim must be fully selected by the
quantity arbiter. The report can then retain the user's already reviewed
compressed disclosure without creating a second MOVE or adding intermediate
wallet identities to it. A stale journal, retired component or retracted route
invalidates the certificate. These records are derived, not authored.

The CLI and built-in assistant cross the same interface:

```sh
kassiber --machine --output inspection.json source-funds review-context \
  --target-transaction TRANSACTION_ID
kassiber --machine source-funds request-input \
  --target-transaction TRANSACTION_ID --action attach_evidence \
  --recipe-file inspection.json --expected-review-fingerprint FINGERPRINT
```

The corresponding tools are `ui.source_funds.review_context` and
`ui.source_funds.request_input`. A recipe file can contain the returned
inspection envelope or the recipe itself. `connect_wallet`, `import_history`
and `attach_evidence` reuse the existing input cards. These requests are
advisory reads, not permission to change provenance. They also work for
transactions that are not quarantined: valid accounting does not necessarily
provide complete documentary origin evidence.

Before a card opens, Kassiber recomputes the original recipe and fingerprint.
The native attachment picker repeats that check after selection and stores a
managed copy on the exact target transaction. Analysis receives an opaque
token for that copy. Import/setup calls retain the original book scope.
Successful input resumes the original conversation; a changed conversation,
book, intervening draft or active turn prevents automatic continuation.
The agent then rereads the current case. Source assertions and links still
require the existing mutation consent, and provenance cannot authorize tax
pairing or turn a CoinJoin pattern into proof of ownership.

Automatic continuation carries the exact recipe in typed, ephemeral screen
context. AI requests reject recipes that would change under privacy redaction
or exceed the continuation size limit; use a short, non-sensitive destination
label and note for that path. Local CLI and desktop review retain their richer
recipe support.

Export readiness comes from canonical `explain_gates`. A reviewed declaration
of missing history can be disclosed as a gap; it remains an assertion about
unknown history. Recipients and disclosure choices remain explicit. Saved
cases bind the target, amount, planned-sale destination/note and disclosure
recipe.
The desktop also supplies the inspected fingerprint when saving; the daemon
rechecks it under the write lock before creating the snapshot.
The case and immutable snapshot are inserted atomically. A failed insert or
standalone commit leaves neither row; nested saves preserve the caller's
transaction and leave its final commit to that caller.
ZIP exports compare copied evidence with the saved file hashes and
retain frozen URLs. Missing or changed disclosed files block export, while
withheld modes never read those files. Publication replaces a prior ZIP only
after the complete verified archive is ready.

The interactive route uses the same `EvidenceGraph` canvas, camera, keyboard
navigation and layered layout as Chain Analysis. Its adapter accepts only the
report's canonical `simplified_flow` nodes and explicit edges; adjacent levels
never imply a connection. CoinJoin/PayJoin boundaries remain marked boundaries.
The printable SVG/PDF stays server-rendered from that same frozen report, so
interactive selection and zoom cannot alter disclosure or evidence authority.

The partial Rust R5 port serves the stored source, link, case and recipient
listings through the CLI and daemon, plus the daemon's paginated evidence
listing. These editor reads retain full attachment metadata without opening
evidence files. Case listings use their frozen recipient fields and do not
load or validate the saved report payload. They do not rebuild journals or
establish provenance, ownership or tax facts. Native source creation and managed
evidence association, link creation/review and managed-id attachment, plus
recipient creation, updates and soft deletion, are
available through the CLI and daemon. They retain the provenance boundary and
roll back failed writes. AI tool execution, consent and once-only admission
remain Python-owned under R6. The native report producer also
serves `reports source-funds` and `ui.source_funds.preview`, including
disclosure, current stored custody and
structural-lineage checks, recipients, explorer references, findings, text and
SVG diagrams. These reads hold a database snapshot and open no evidence file
or network connection. Native tied-flow order is deterministic; its complete
bytes match a measured Python hash-seed variant. The known Python fee-attribution
bug is retained for parity; see the [producer checkpoint](rust-shell.md#source-funds-report-producer-checkpoint-r5-partial).
Case saving through `reports source-funds --save-case` and
`ui.source_funds.cases.save` freezes the complete report, SVG, recipient fields
and target external reference. Report generation and both snapshot rows share
writer admission. The native daemon
`ui.source_funds.review_context` supplies the fingerprint used by reviewed saves;
those saves reject stale or truncated inspections under the same write lock.
CLI review-context, input handoffs and coverage share this producer.
Persisted suggest and assemble also use the native service; see the
[persisted-pass checkpoint](rust-shell.md#source-funds-persisted-pass-checkpoint-r5).
Native batch review revalidates the target's
suggested links against current stored custody or authoritative, wallet-scoped
structural lineage under the same write lock as approval. An explicit empty
selection approves nothing; CLI dry runs return the proposed result and roll
back every write. Neither path establishes ownership or tax facts.
PDF rendering and PDF-containing
bundles remain in Python by the owner's decision; their routes are not
registered natively.

Native `source-funds review-context` / `request-input` and the corresponding
daemon kinds bind the recipe, reachable editor records, evidence metadata and
input version to the same review fingerprint. The 200-record inspection caps,
truncation refusals and advisory input handoffs retain the Python contract.
Current deterministic bulk eligibility is computed outside that fingerprint.
Only an explicitly selected, bounded CLI recipe file is read; evidence files
remain unopened. See the [review checkpoint](rust-shell.md#source-funds-review-and-input-checkpoint-r5-partial).
Native CLI and daemon coverage classify historical inbound observations through
those report gates. Totals retain each inbound hop and already-spent inflows;
they are not a current-holdings calculation. The six buckets remain exhaustive,
and capped rows stay visible as `not_classified`. Shared report inputs live
only for that request. The Python truncated text currently includes skipped
rows in its claimed "classified" count; the
[reproduction](../../crates/reports/scripts/repro_python_source_funds_coverage.py)
records that display bug and Rust retains its text for parity. See the
[coverage checkpoint](rust-shell.md#source-funds-coverage-checkpoint-r5-partial).
Native `source-funds suggest --dry-run` previews local privacy-boundary,
current stored custody, structural-input and provider-hint candidates. It
retains the reference ordering, reviewed/rejected-link vetoes, confidence
supersession, target frontier and 500-row ceiling. Its preview savepoint
rolls back every temporary edit before returning the preview, including on
failure. Suggestions remain heuristic provenance requiring review; they grant
no ownership or tax authority. Evidence files and backends are not opened.
Persisted suggestions and assembly run natively through CLI and daemon.
Assembly loads inputs once, then commits each suggestion pass and each review
pass separately, retaining completed stages if a later stage fails. It defaults
to eight passes and caps requests at sixteen. There is no assembly dry-run
contract. See the
[suggestion checkpoint](rust-shell.md#source-funds-suggestion-preview-checkpoint-r5-partial).
PDF rendering and PDF-containing bundles remain in Python by the owner's
decision; their routes are not registered natively.

Uncaught failures on the native source-funds daemon reads include an empty
native debug string; typed refusals retain null. Python-only traceback frames
and stderr are not fabricated. Non-debug response fields retain exact parity.

The AI tool executor calls Python's connection-scoped source-funds service
directly. It does not invoke the native CLI or daemon transport. Its advertised
schemas, explicit consent, duplicate-call suppression, frozen book/input
binding and redacted projections remain Python-owned under R6, including source
admission and case saving. Direct CLI and desktop requests retain their own
validation and case-review fingerprint barriers; they do not enter the AI
consent loop.

Recipient mutations preserve exact, profile-scoped references and frozen case
recipient fields. Python currently translates duplicate-recipient constraint
errors to validation errors on SQLite but leaves SQLCipher's distinct exception
untranslated. The Rust service preserves that envelope difference. The local
[reproduction](../../crates/reports/scripts/repro_python_source_funds_recipient_constraints.py)
records the discrepancy without changing Python product behavior. No recipient
mutation tools are exposed to the in-product AI.

Native CLI and daemon source-funds failures preserve non-debug response bytes.
Under plan 21's honest-native contract, requested native debug text is empty:
CLI debug is null without `--debug`, while daemon debug is null for typed
refusals and an empty string for unhandled service failures. Python tracebacks
are not synthesized. Daemon errors retain the existing secret redaction floor.

The Python reference commits source/link evidence associations before response
projection. A malformed stored amount can therefore make the request fail after
the association has committed. Rust keeps the write and projection in one
transaction and rolls that failure back. The synthetic
[reproduction](../../crates/reports/scripts/repro_python_source_funds_projection_commit.py)
records this discrepancy and the reference CLI's unredacted error message;
Python product behavior is unchanged. This corrupt-record path is excluded
from process parity acceptance and is tested separately for native rollback.

Batch review has the same Python post-commit response-projection discrepancy.
The synthetic [batch reproduction](../../crates/reports/scripts/repro_python_source_funds_batch_projection_commit.py)
records it on both database bindings. Native batch admission retains all updates
and response projection in one transaction and rolls back the entire batch on
failure; Python product behavior is unchanged.

Persisted suggestions can likewise release their outermost savepoint before
projecting the response, and assembly's bulk-review stage commits before its
projection. The [pass reproduction](../../crates/reports/scripts/repro_python_source_funds_pass_projection.py)
records corrupt-allocation failures after those commits on both bindings.
Native passes keep their own write and response projection atomic while
retaining earlier committed assembly stages. These injected corrupt-record
paths have separate native rollback tests and are excluded from parity claims;
Python product behavior is unchanged.
