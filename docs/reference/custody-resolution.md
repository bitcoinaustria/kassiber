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
JSON cannot impersonate a native observer. Provider IDs, matching amounts and
nearby timestamps remain candidate evidence. A reviewed component can supply
missing historical meaning with explicit quantities and durable provenance.

## Automatic evidence and remaining boundaries

| Situation | Result |
| --- | --- |
| Same scoped transaction observed by both own wallets, exact quantities | Automatic MOVE; no manual pairing required. |
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
accounting consequences. **Investigate with assistant** on Quarantine starts the
same workflow available to external agents through the CLI. The UI displays the
proposed changes and their computed effects, then asks for one approval of that
exact proposal. Manual component editing remains available.

The shared `core/review_workflow.py` module exposes four operations:

| Operation | Contract |
| --- | --- |
| `review cases` / `ui.review.cases` | Current canonical quarantine cases, paginated with a book/version-bound cursor; recent execution receipts support continuation. |
| `review plan` / `ui.review.plan` | Apply typed operations to an isolated in-memory book snapshot and rebuild with the canonical custody journal. Return a portable artifact containing scope, input version, operations, before/after effects and a digest. No live-book writes or network calls. |
| `review apply` / `ui.review.apply` | Revalidate scope, version and effects under one writer transaction, apply the exact operations, rebuild/store journals and append a durable receipt. Any failure rolls back the whole batch. |
| `review receipt` / `ui.review.receipt` | Retrieve the historical execution and verification result by receipt ID or idempotency key in the active book. |

Supported batch operations are exact price overrides, explicitly justified
exclusions, and typed custody components. The CLI accepts the existing component
create/revise/state actions. AI batches create components; conversion components
remain drafts until separately reviewed. Missing-wallet gap investigation keeps
its existing local-provider-only tools (`ui.custody.review.plan/apply`) and is
not smuggled into the general batch interface.

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
they are included as bounded audit summaries and are not replicated as authored
accounting decisions. SQLCipher snapshots use the same binding and an ephemeral
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
