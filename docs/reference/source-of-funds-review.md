# Source-of-funds review

The desktop flow starts with an explicit transaction and amount. It presents
the current origin evidence and unresolved findings for that target, then
offers investigation through the existing assistant. Manual source/link edits,
disclosure controls and PDF/ZIP export remain available. A preview is not a
completed review, and a saved export is not current after the case changes.

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
ZIP exports compare copied evidence with the saved file hashes and
retain frozen URLs. Missing or changed disclosed files block export, while
withheld modes never read those files. Publication replaces a prior ZIP only
after the complete verified archive is ready.
