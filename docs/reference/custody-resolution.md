# Understanding movements and resolving quarantine

Kassiber should follow the owner's Bitcoin across every connected watch-only
wallet. A verified movement is a custody fact; a missing price or earlier cost
basis is a separate accounting problem. Connecting all relevant wallets gives
the interpreter more evidence, but never proves that the owner has no other
wallets.

## One path from observation to report

1. Wallet adapters observe transactions and outputs. Native chain sync records
   closed provenance bound to the normalized graph and exact wallet quantity.
2. `CustodyJournalBuilder` combines the current observations, ownership index,
   channel lifecycle and reviewed custody components.
3. The custody interpreter proposes exact quantity claims. The arbitrator
   checks ownership, scope, conservation and competing claims once.
4. The finalized projection supplies selected movements and external events to
   RP2. Reports, graphs and AI read the resulting stored projection.

An export's transaction hash can identify an event, but graph-shaped imported
JSON cannot impersonate a native observer, and a supporting import cannot erase
one either: enrichment updates prices and still-empty metadata columns while an
observation under a closed provenance commitment stays intact. Provider IDs, matching amounts and
nearby timestamps remain candidate evidence. A reviewed component can supply
missing historical meaning with explicit quantities and durable provenance.

Swap and refund candidates cannot connect known different Bitcoin network
domains. Provider-ID and refund-funding uniqueness is checked within compatible
networks; unknown-network duplicates remain competing evidence rather than
promoting a candidate to exact. Payment-hash cardinality uses the same compatible
population before occupancy and pair dismissals. A native HTLC route that still
has several possible payment records receives `native_transition_ambiguous`;
it cannot fall through to an arbitrary disposal and fresh acquisition.
This also applies to time/amount suggestions
used by report-readiness checks.

Missing-wallet discovery uses the same explicit inbound classification and
user override precedence as the accounting adapter. A classified purchase or
income receipt is not an unexplained custody return merely because its wallet
is a postmix wallet. Search-capacity limits cannot turn already-disqualified
external sources back into custody holds; genuine unresolved sources remain
held when their search is incomplete. Notes and taxability flags do not
establish an external origin.

## Automatic evidence and remaining boundaries

| Situation | Result |
| --- | --- |
| Same scoped transaction observed by both own wallets, exact quantities | Automatic MOVE; no manual pairing required. |
| The recipient synced first and its native inputs imply a missing debit from another connected wallet | `ownership_transfer_source_missing` holds the receipt until the sender's history is available. It does not invent a new acquisition or an absent debit. |
| A collaborative transaction returns a wallet's entire contribution while another participant funds the recipient or pays the fee | Complete native wallet-net evidence rules out a missing debit. A proven zero-movement observer row is non-economic; foreign-paid fees are not attributed to the owner. |
| An address list and a later descriptor overlap on some receipt outputs | Complete current native evidence and canonical script ownership apportion the receipt once per output. Imported rows remain unchanged. Incomplete or conflicting evidence produces a targeted quantity hold. |
| Wallets disagree on confirmation | The newest closed observation determines the physical transaction's state. Equal-time disagreement remains pending; an older confirmed row cannot override a newer unconfirmed observation. |
| Known owned scripts/outpoints explain a fan-out or consolidation | Automatic conserving allocations where the complete native proof is available. |
| Multiple own source and destination wallets in one fully owned transaction | Automatic N:M allocations, with one network fee, when all inputs/sinks and amounts reconcile. The FIFO cells are accounting allocations, not physical tracing of individual satoshis. |
| Collaborative transaction with foreign participants and a complete, exactly conserving set of own wallet movements | Automatic own-wallet allocations when current native graphs prove every own contribution and receipt. Foreign participants' fees do not become the owner's fees. |
| A same-block `A → B → A → C` sequence | Native input references establish order before wallet-based hints; a wallet round trip is not a blockchain cycle. |
| Zero-value inbound placeholder beside a real receipt | Does not compete with the positive receipt. |
| Unrelated export receipt with a different canonical stored txid | Does not make an owned receipt ambiguous merely because its provider ID differs. |
| Complete, conflict-free native HTLC claim or refund linking two own endpoints | Automatically interpreted during journal processing when its fee timing is representable. No authored review is created; the existing book policy determines the tax projection after matching. |
| Proven HTLC route with a principal shortfall across different dates | Targeted fee-timing quarantine. Assigning every later refund/claim fee to the funding date could put it in the wrong reporting period. The candidate remains visible. |
| Provider-only swap evidence, ambiguous HTLC route or incomplete funding amount | Remains a review candidate. A link alone does not prove that it covers both complete wallet rows. |
| Only some N:M destination receipts have synced | Remains unresolved until the destination population is complete; recorded and synthesized receipt paths must not overlap source allocations. |
| Incomplete input ownership, unknown Liquid amounts, duplicate receipts, unexplained own residual | Review remains necessary. Connecting a missing wallet or completing sync may provide the missing facts. |
| Proven custody movement with missing acquisition basis or required fee price | Custody remains understood; tax reporting can still be blocked until the accounting evidence is supplied. |

For HTLC claim/refund observations, LWK and Bitcoin Core RPC retain a small
non-secret attestation after inspecting the witness. The matcher requires
current closed provenance from the matching observer, one canonical funding
input and the existing whole-row coverage checks. Merely copying the
attestation into an import does not establish proof.
It does not add witnesses or preimages to normalized transaction `raw_json`.
LWK's existing opaque dependency state is a separate storage boundary.

An unchanged sync still invalidates journals if its newly published observer
authority or effective confirmation changes the accounting inputs. Refreshes
with the same facts keep journals current; timestamp updates alone do not
trigger a rebuild.

## CoinJoin, Payjoin and intermediate wallets

Kassiber can follow proven own-wallet movements inside collaborative Bitcoin
transactions. It does not need to identify a particular CoinJoin coordinator or
declare every mixed-input transaction a Payjoin. A generic `collaborative`
boundary preserves uncertainty about the protocol. In particular,
[BIP 78 Payjoin](https://github.com/bitcoin/bips/blob/master/bip-0078.mediawiki)
allows the receiver to contribute inputs: sharing a transaction does not prove
that all inputs belong to the sender.

Connecting intermediate wallets supplies their owned scripts, spent outpoints
and receipts. Complete native observations can explain conserving 1:1, 1:N and
N:M own-wallet flows even when other participants appear in the same transaction.
Allocation cells carry accounting basis; they do not assert a physical mapping
from a particular input's satoshis to a particular output.

When a reviewed shortcut is later explained by complete native intermediate
history, the journal uses that native route only when it reproduces every
reviewed source-to-return allocation and accounts for each fee. Exact native
fees may resolve a previously unexplained reviewed remainder. Missing steps,
competing wallet activity or different allocations retain the reviewed
interpretation and its remaining holds. This changes the effective projection,
never the immutable authored review.

The whole transaction fee is a chain fact; the owner's share can still be
unknown. If the owner contributes 1 BTC and receives 0.9999 BTC while another
participant does the same, the transaction fee is 20,000 sats but the owner's
net reduction is only 10,000 sats. The adapter preserves that net movement and
does not charge the owner the entire fee. An unexplained own difference remains
`privacy_hop_unresolved` until evidence distinguishes a payment, receipt or fee.
Adding all connections does not by itself supply that commercial meaning.

Use settled wallet records or supporting documents for the missing attribution.
Wasabi `paymentsInCoinJoin` metadata is retained in wallet import metadata, but a
scheduled or running round does not prove a settled transaction payment and is
not automatically used as one. Samourai's public Deposit/Badbank/Premix/Postmix/
Ricochet sources help organize the ownership history; native observations and
the same quantity checks still establish movements.

## Why a transaction is quarantined

A quarantine holds a transaction out of the tax journals because the evidence
is not enough to book it; it never guesses a taxable event. Every journal
rebuild recomputes all rows, so a quarantine clears by itself once the missing
evidence arrives. Desktop syncs, imports and "Process journals" end with that
rebuild (see [daemon](daemon.md)); the quarantine snapshot reports
`summary.freshness` so a list older than the book's transactions is marked.

`core/quarantine_catalog.py` is the single vocabulary for reading a stored
reason. It never decides whether something is quarantined:

| Category | Meaning | Typical next step |
| --- | --- | --- |
| `missing_wallet_history` | Part of the movement happened in a wallet whose history is not in the book (sending wallet not synced, intermediate wallet missing, channel sweep wallet). | Sync or connect that wallet; import its history. |
| `missing_chain_evidence` | The stored graph cannot prove amounts (no input values, confidential outputs, file-only side, pending or replaced). | Sync again from the chain; wait for confirmation. |
| `missing_price` | No usable price for the valuation date. | Refresh rates or enter the price. |
| `missing_acquisition_history` | A disposal needs earlier acquisitions Kassiber has not seen or priced. | Import purchase history or connect the source wallet. |
| `needs_decision` | The evidence has more than one valid reading. | Pair, classify or review the component. |
| `unsupported` | Kassiber cannot book the shape yet. | Stays visible until supported. |
| `downstream` | A consequence of another transaction's problem (basis barrier, blocked transfer chain, contaminated lots, carried basis). | Resolve the root; the row follows. |

When a transaction has several reasons, the stored `reason` is the root cause
and the rest are kept in `detail.additional_reasons`: a custody gap hold is
not labelled with the basis barrier it raises. `custody_basis_barrier` rows
name the transactions that set their pool's barrier in
`detail.root_transaction_ids`; a receipt behind the barrier that books nothing
at all is listed as such a downstream row instead of disappearing.
`custody_quantity_unresolved` keeps a deterministic primary `blocker_code` and,
when several issues apply, every `blocker_codes`/`issue_ids` plus `gap_ids`.

`ui.journals.quarantine` and `journals quarantined` add per row `category`,
`blocks_reports` (the row is named by a persisted custody quantity issue, which
blocks every report), `is_downstream`, `root`, `reasons`, normalized
`evidence` (wallet labels instead of ids, required/available msat, gap id) and
ordered `actions`, plus the `group_key` of the cause it belongs to. The
snapshot also groups rows by root cause (each group names up to 25 of its
`root_transaction_ids` and its `root_count`), orders roots before their
consequences and oldest first, pages with `offset`, and lists
`assumptions`: outflows booked as disposals only because no owned destination
is known, and kind-less receipts booked as purchases at market value. Neither
assumption is a quarantine, but both change results when the owner has an
unconnected wallet. An optional `scope` picks which rows a page lists:
`attention` (root causes, plus downstream rows whose root cannot be named),
`waiting` (rows that only follow a named root) or `all` (the default); the
summary always covers the whole book and reports `attention_count`,
`waiting_count` and the listed scope's `scope_count`. `ui.review.badges`
reports `quarantine_attention` beside the row count: the root causes, or every
held row when none is on record, so the side-nav counts what needs the user.

A `reviewed_residual_suspense` hold most often comes from a pair whose legs
are not one movement: a pair review splits whatever the source sent beyond
the destination and fee into a suspense leg. When the held transaction is one
leg of a current pair, the evidence names the `pair_id` and the other leg,
both legs as `pair_legs` (wallet, amount, time, txid), and whether the legs
carry different same-asset txids or the receipt predates the spend; the first
action is `review_pair`, which opens that pair where it can be unpaired.
Unpairing removes the suspense on the next journal run, and journal
auto-pairing never recreates a pair the owner removed.

The desktop's Quarantine page leads with a summary: how many rows need the
user, how many only wait on a cause (listed on request, one line each with the
cause it waits on), whether reports are blocked, and the one step that fixes
them. **Fix all** covers pairs that join two different on-chain transactions;
**Fix with assistant** hands the rest to the assistant. Either way the change
is previewed through `ui.review.plan` and applied, journals included, on one
confirmation. Below, one card per cause says what was seen and what to do and
lists its own transactions; a suspense left by pairs lists each pair side by
side, and a pair Kassiber cannot decide keeps its own Unpair through the same
preview. Long lists fold (three rows, four causes); rows beyond the loaded
page appear once those are resolved. The side-nav badge counts the causes. The
page keeps the gap editor behind developer tools and offers exclusion only for
price and decision questions.

## Desktop review

The **Transfers & Custody** surface combines transfer/swap review with custody
gaps and components. Gaps and components retain their developer-mode gate.
Existing gap links open the selected case, including a case outside the first
page. Component creation and revision use structured legs and allocations rather
than a JSON text editor. Unsupported existing shapes are rejected explicitly.

The editor first previews the resolved server plan, quantities and validation.
Saving a draft or activating it requires a separate confirmation against that
preview's book version. Editing the form or changing books invalidates the
preview. Revisions retain original timestamps and separate source/destination
conversion amounts unless the user edits them.

## Resolve with the CLI or chat

The agent investigates through typed tools; Kassiber computes and validates the
accounting consequences. On Quarantine, **Fix all** previews and applies the
repairs Kassiber decides itself (pairs that join two different on-chain
transactions) as one reviewed proposal. **Fix with assistant** hands the
remaining causes to the same workflow available to external agents through the
CLI. Either way the UI displays the proposed changes and their computed effects,
then asks for one approval of that exact proposal. Manual component editing
remains available.

Terminal chat waits for the daemon's revalidated `review_preview` and prints
the complete proposal or historical retry receipt before asking for approval.
`ui.review.apply` cannot be pre-approved with `--yes`, `--allow-tool`, `/allow`,
or a previous session answer. Every proposal needs a new terminal answer;
non-interactive chat denies it. The explicit `review plan/apply` CLI remains
available for externally reviewed, portable artifacts.

The shared `core/review_workflow.py` module exposes four operations:

| Operation | Contract |
| --- | --- |
| `review cases` / `ui.review.cases` | Current canonical quarantine cases, paginated with a book/version-bound cursor; rows that only wait on a case are counted in `waiting_count` instead of listed; recent execution receipts support continuation. |
| `review plan` / `ui.review.plan` | Apply typed operations to an isolated in-memory book snapshot and rebuild with the canonical custody journal. Return a portable artifact containing scope, input version, operations, before/after effects and a digest. No live-book writes or network calls. |
| `review apply` / `ui.review.apply` | Revalidate scope, version and effects under one writer transaction, apply the exact operations, rebuild/store journals and append a durable receipt. Any failure rolls back the whole batch. |
| `review receipt` / `ui.review.receipt` | Retrieve the historical execution and verification result by receipt ID or idempotency key in the active book. |

Supported batch operations are exact price overrides, explicitly justified
exclusions, pair-review removal (`unpair`), typed custody components, and local
inbound `kind_override` declarations. The CLI accepts the existing component
create/revise/state actions. AI batches create components; conversion components
remain drafts until separately reviewed. Missing-wallet gap investigation keeps
its existing local-provider-only tools (`ui.custody.review.plan/apply`) and is
not smuggled into the general batch interface.

Acquisition review is available in the desktop Tax tab and the local CLI. It
accepts a non-quarantined inbound transaction, using existing authored metadata
and append-only history; clearing `kind` restores the importer's classification.
It does not rewrite import evidence or use loan markers. Active custody legs must
be reviewed through their custody workflow. Desktop classification applies
separately from the ordinary Save action; other dirty fields must be saved or
discarded first.

For example, use this operation with the same `review plan/apply` commands:
`{"type":"kind_override","transaction_id":"…","kind":"income","reason":"Source reviewed"}`.
Set `kind` to null to clear it. The optional `valuation_mode` currently accepts
only `market_value`; `zero_cost` produces `acquisition_valuation_unsupported`.
The canonical Austrian gate also rejects airdrop/hardfork FMV interpretation,
including existing imports; see [Austrian support](../austrian-handoff.md).
Existing `metadata records kind set/clear` remains available for direct authored
metadata editing; use review plan/apply when approving an exact economic effect.

Acquisition artifacts bind the database instance, profile policy, input version
and canonical economic output, with exact before/after whole-book acquisition
basis, recognized income, disposal basis/proceeds and realized gain/loss by asset
in the profile currency. These are accounting amounts, not estimated tax due;
quarantine means the shown totals are incomplete. They are read-only until the
explicit atomic apply, which rechecks the same evidence and appends a receipt.
This financial preview is local desktop/CLI only, not a new AI tool surface.

An external agent can save and inspect the same portable proposal:

```bash
kassiber --machine review cases --limit 100
# Follow next_cursor until null. Use the returned input_version below.
kassiber --machine --output proposal.json review plan \
  --operations-file corrections.json --expected-input-version 7
# After reviewing proposal.json:
kassiber --machine review apply \
  --artifact-file proposal.json --idempotency-key review-2026-09-05-1
kassiber --machine review receipt --idempotency-key review-2026-09-05-1
```

`corrections.json` contains an ordered operations array, for example:

```json
[
  {
    "type": "price_override",
    "transaction_id": "transaction-from-review-cases",
    "fiat_rate": "20000",
    "reason": "Acquisition rate verified against the supplied invoice"
  }
]
```

An `unpair` operation (`pair_id`, `reason`) removes a pair review; a case
whose suspense came from a pair lists `unpair` in `supported_operations` and
names the pair with its legs. The Quarantine page uses it for **Fix all** and
for unpairing a single pair the owner judged, so either path stores the
recalculated journals in the same transaction.

Prices are decimal strings. A price assertion still needs evidence: the module
checks arithmetic and records the reviewed assertion, rather than proving an
invoice's contents. An exclusion is never a substitute for explaining an owned
movement or missing acquisition basis. Native chain authority, component anchor
coverage and conservation retain their existing checks.

The chat's bounded review capability pack is selected by English/German review
requests or the Quarantine screen. It includes case pagination, transaction and
transfer context, evidence reads and the shared plan/apply tools. A review turn
defaults to 16 model rounds (ordinary turns remain 8; explicit limits win).
At budget exhaustion the answer retains a bounded continuation packet with
case cursor/version and applied receipt IDs. A resumed agent inspects current
cases and retrieves receipts; unapplied plans must be reconstructed unless the
external CLI agent saved their artifact. Kassiber does not persist model
reasoning or create a separate background agent scheduler.

When a case needs input, the assistant can call `ui.review.request_input` with
canonical case IDs and their input version. Its read-only response offers one
of three actions: connect a wallet, import transaction history, or attach a
document to a single case. The chat renders that request as an action card and
opens the existing connection/import flow. History can be imported into the
case's existing connection. A document selected for a case is copied into its
managed attachments; analysis reads that same saved copy through an opaque
token. Supplying a file or creating a connection does not resolve a quarantine.

Successful input continues the same conversation with fresh cases. If another
turn or draft intervened, the card offers an explicit continuation instead.
Closing a dialog is not success; failed synchronization after connection setup
is shown as partial completion. Book and conversation changes invalidate old
cards. Dialog requests carry `expected_scope`, checked by the daemon before
dispatch, including network probes and explicit target profiles. Choosing
“I don't have this” asks the assistant to explain the remaining gap; it never
excludes a transaction or invents acquisition basis.

Before showing consent, the daemon recomputes the proposal's effects. Apply is
always once-only consent and stays pinned to the chat's original book. A digest
binds the content being reviewed; it never grants accounting authority. An AI
proposal that would change under privacy redaction or exceed the tool argument
limit is rejected with instructions to use local evidence identifiers or a
smaller batch. Network evidence gathering keeps its separate existing consent.

A receipt's `verified` status means the canonical rebuild completed and matched
the preview. Check `verification.report_ready` and remaining quarantines; verified
does not mean every case was resolved. Receipts are historical, not a promise
about a subsequently changed book. Retrying the same key and artifact returns
the original receipt even after the book changes; reusing the key for another
artifact fails. Preview and apply use recorded observations and prices with
identical semantics, without an extra sync/repricing pass only at application.

Applied receipts reference the existing transaction/component audit history;
they are included as bounded audit summaries, not as authored accounting
decisions. SQLCipher snapshots use the same binding and an ephemeral
in-memory encryption key; decrypted book pages are never exported to disk for
planning. Detailed component and guided-review contracts remain documented in
[custody-components.md](custody-components.md).

## Regression evidence

The audit is covered by ownership and RP2 engine tests, same-block chronology
tests, actual LWK/Core-record-to-matcher tests, database-backed matcher loader tests,
and scoped/core AI tool tests. The ordinary fast regtest lane follows recorded
node activity through sync, journal, report and XLSX export; the independent
chain-observer lane compares Bitcoin/Liquid adapters against local node truth.
These fixtures prove the specified cases, not the completeness of any user's
private book.

The remaining architecture boundary is explicit fee timing for multi-date HTLC
routes with a shortfall. A native link proves the movement but does not justify
booking every residual fee at the source timestamp. Do not bypass this with an
exclusion or price override. A complete automatic solution needs separately
timed fee evidence and in-transit custody in the custody-to-tax projection.
Period-boundary tests must verify both fee dates and the wallet/in-transit
balances between funding and return; moving the fee date alone is insufficient.
