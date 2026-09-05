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

The preferred direction is agent-assisted investigation with a reviewable
result. Users should not have to reproduce the agent's transaction search and
evidence comparisons by clicking through individual editors. CLI and built-in
chat share the typed investigation and plan/apply contracts; the UI makes
evidence, open questions and proposed accounting effects inspectable. The manual
component editor remains available for cases that need direct correction.
This is a product direction, not a claim that an unattended quarantine-resolution
agent is already implemented.

Start with the actual reason and evidence:

```bash
kassiber --machine journals quarantined
kassiber --machine journals quarantine show --transaction <transaction-id>
kassiber --machine journals transfers list
kassiber chat --tool-profile scoped
```

The built-in chat recognizes English and German quarantine requests and exposes
the transfer/custody tools needed to investigate them. CLI chat's default
`core` profile includes transaction review context, guided custody-gap review,
and audited price/exclusion repairs. Use `scoped` for specialist component
authoring based on the current question, or `full` for the complete catalog.

The AI workflow is:

1. Read `ui.journals.quarantine`, `ui.transactions.review_context` and, for
   ownership/rail problems, `ui.transfers.review_context`.
2. Use available local observations first. If evidence is missing, explain
   which wallet, source document, price or review decision is needed.
3. For a custody gap, use `ui.custody.review.plan` then
   `ui.custody.review.apply`; for a general multi-leg interpretation, use
   `ui.transfers.components.plan` then `ui.transfers.components.apply`.
   Apply must reference the previewed journal input version.
   The guided custody-gap tools require an on-device AI provider; an off-device
   chat cannot use them. The CLI can perform the same review locally.
4. For a substantiated price correction or explicit exclusion, use
   `ui.journals.quarantine.resolve`. It rebuilds by default. An exclusion is
   never a substitute for explaining an owned movement.
5. After custody changes, run `ui.journals.process` and reread quarantine and
   `ui.report.blockers`. Successful mutation alone does not prove resolution.

CLI equivalents for guided review and components are documented in
[custody-components.md](custody-components.md). Writes and network reads retain
the chat's consent and pinned book scope. AI interprets and proposes evidence;
the deterministic accounting boundary validates every application.

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
