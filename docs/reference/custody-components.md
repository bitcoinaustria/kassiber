# Custody components

Custody components are Kassiber's durable interpretation layer for owned value
that moved through several wallets or Bitcoin rails. Imported transaction rows
remain immutable evidence anchors. A component says how value flowed between
those anchors; it never rewrites them.

Use a component when a single pair cannot describe the history:

- one transaction funded several owned destination wallets;
- several old wallets consolidated into one wallet;
- a migration crossed one or more historical wallets that are no longer
  available to import;
- on-chain, Liquid, and Lightning legs form one reviewed custody route;
- a refund or channel lifecycle returns value to the same logical wallet; or
- an N:M consolidation needs explicit source-to-destination allocation.

## Country-neutral ownership boundary

Bitcoin evidence determines the graph. Country never does.

Address/script ownership, txids/outpoints, payment hashes, provider evidence,
amount conservation, fee evidence, conflict clustering, and component
activation are country-neutral. The matcher API does not accept a tax country.
Only after a complete candidate/component graph exists does `tax_policy.py`
recommend how an already-proven edge is booked. Austrian Alt/Neu handling is a
downstream lot-classification consumer of the same generic allocation graph; it
cannot create, reject, rank, or reshape a match.

Exact wallet-native proofs and reviewed interpretations have deliberately
different persistence rules. A normal Bitcoin transaction graph or owned
script/outpoint is recomputed from imported evidence. A Lightning hash is exact
only when canonical, node-native/source-qualified, uniquely 1:1, and
equal-principal. Channel lifecycle suppression requires an explicit local
funding contribution and exact close evidence; every force-close vin match
requires an explicit local commitment outpoint, because candidate uniqueness
does not prove which commitment output belonged to the node. Otherwise these
paths fail closed into review. Reproducible proofs are recalculated whenever
journals run; it is derived state, not a second authored ledger. Manual gap
closures and any judgment that cannot be reproduced from that evidence are
versioned custody components and replicate as authored records. A future
Bitcoin layer plugs in by emitting the same country-neutral legs, conservation
units, anchors, and evidence grade; it does not add logic to a country tax
module.

## Atomic model

A component contains:

- versioned header evidence and review state;
- typed legs (`source`, `destination`, `retained`, `fee`, `external`, or
  `unresolved`);
- rail, chain/network, asset, exposure, conservation unit, amount, wallet,
  timestamp, and optional transaction anchor per leg; and
- explicit allocation edges for N:M flows.

Activation is all-or-nothing. Sources must equal owned destinations plus fees,
external value, retained custody, and zero unresolved value. Quantity is
conserved per exposure/unit, so BTC, L-BTC, and Lightning BTC can retain their
rail identity while sharing Bitcoin exposure. Every carrying-value allocation
must also remain inside one physical network domain: Bitcoin/Lightning mainnet
is compatible with Liquid mainnet, testnet with Liquid testnet, and regtest
with Elements regtest; signet never aliases another network. Known network
contradictions fail closed even when the authored legs omit network fields and
only their imported wallet anchors reveal the mismatch. Reviewed conversion
mode may cross these domains because it does not claim quantity continuity. A
component with incomplete, conflicting, or half-replicated evidence remains
visible but cannot book. Its known transaction anchors quarantine fail-closed,
and even a header with no arrived legs blocks report readiness.

An unknown rail/network is missing evidence, not a scope reset. Known domains
propagate through allocation edges and through receive/spend legs at the same
custody wallet. A mainnet route therefore cannot emerge as regtest after an
unscoped `untracked` wallet, whether the two halves live in one component or in
separate active components. A route with only one known domain and genuinely
unknown intermediate evidence remains valid. Future Bitcoin layers participate
without country-specific code by declaring their base chain/network domain on
the same generic leg contract.

Journal processing may persist a partial diagnostic snapshot while such a
header has no live transaction anchor to quarantine, but it returns an explicit
`custody_component_blockers` entry and every CLI, daemon, and desktop report
gate rejects that snapshot. A zero-row quarantine can therefore never make an
incomplete authored interpretation look report-ready.

All leg, allocation, and valuation quantities are exact non-negative signed-64
integers. On the desktop daemon boundary, values above JavaScript's safe-integer
limit are represented as decimal strings; safe values remain JSON numbers for
backward compatibility. Preview, display, revision serialization, and daemon
input parsing keep that representation exact instead of coercing it through a
floating-point `Number`.

Manual review never waives anchor coverage. Every imported outbound and inbound
anchor must be replaced at its full economic amount; unknown change, a service
fee, or an external co-payment must be represented explicitly as a retained,
fee, external, or unresolved leg. This prevents a reviewed bridge from silently
erasing the unmatched part of a raw transaction.

Authored rail identity is evidence-bound as well. When the imported transaction
or wallet identifies its Bitcoin rail, chain, or network, an anchored leg may
omit an unavailable detail but may not contradict it. A BTC regtest transaction,
for example, cannot be relabeled as Liquid mainnet merely by editing a component.

For an outbound anchor whose backend stores the miner fee separately, the
source leg is the complete debit (`transaction amount + transaction fee`). The
fee leg allocates part of that source; it is not added a second time for anchor
coverage. For fee-inclusive imports, the source equals the stored net debit.

Anchored legs always inherit the imported transaction's canonical occurrence
time; an authored override is rejected. Transaction-less legs require a valid
RFC3339 timestamp. The original anchor id is retained even if a backend later
retracts the live transaction row, so deletion invalidates the component rather
than silently turning an anchored leg into a manual one. Excluding an imported
anchor likewise invalidates every active component that depends on it; even when
all of a component's anchors are excluded and no quarantine row can be written,
the component remains a hard report-readiness blocker.

Multiple chronological hops may live in one component. Connected N:M edges at
one stage are allocated together; later stages remain ordered so an
intermediate wallet receives its lots before it spends them. If any projected
member fails, the complete component is withheld. When a component both credits
and spends an intermediate wallet/exposure, cumulative outgoing value may never
run ahead of the component's earlier credits; unrelated pre-existing lots cannot
make a reversed migration route pass validation.
Every explicit or inferred allocation also requires its source occurrence time
to be no later than its sink. Anchored legs use the imported canonical times for
this check, so a later outbound row can never carry basis backward into an
earlier inbound row. Reusing one `untracked` wallet across separately authored
active components applies the same cumulative chronology across those pieces.

## Missing historical wallets

Use wallet kind `untracked` for an owned location whose transaction history is
missing. In a bulk component document, the explicit `untracked_wallet` sugar
creates or reuses that placeholder atomically:

```json
{
  "components": [
    {
      "component_type": "manual_bridge",
      "evidence_kind": "manual_migration_review",
      "evidence_grade": "reviewed",
      "legs": [
        {
          "id": "old-source",
          "role": "source",
          "transaction": "old-wallet-send-txid",
          "amount_msat": 100000000
        },
        {
          "id": "gap-receive",
          "role": "retained",
          "untracked_wallet": "Missing 2021 wallet",
          "occurred_at": "2021-06-01T00:00:00Z",
          "amount_msat": 100000000
        }
      ]
    }
  ]
}
```

A later component can use the same label as `untracked_wallet` for its source
leg and route the value into current wallets. Transaction-less owned legs must
always name a wallet and an `occurred_at` timestamp.

## N:M allocations

One-source fan-out and many-source consolidation are inferred only when the
flow is unambiguous. A genuine N:M graph needs explicit edges:

```json
"allocations": [
  {
    "source_leg_id": "source-a",
    "sink_leg_id": "destination-c",
    "source_amount_msat": 60000,
    "sink_amount_msat": 60000
  },
  {
    "source_leg_id": "source-b",
    "sink_leg_id": "destination-c",
    "source_amount_msat": 40000,
    "sink_amount_msat": 40000
  }
]
```

The allocator is a deterministic max-flow, not a greedy edge walk, so flexible
edges can reroute around constrained destinations. Explicit fees stay attached
to their original outbound source. An unexplained residual in legacy pair rows
is deterministic but not evidentiary; use a custody component when the reviewed
per-source allocation is known.

For an automatically proven many-wallet consolidation, Bitcoin does not define
which contributing input paid which sat of the transaction fee. Kassiber's
derived journal uses a stable convention: the largest contributor (wallet-id
tie-break) bears the single shared fee. This is an accounting allocation, not a
claim about Bitcoin's transaction graph. If wallet-level fee/lot attribution is
material, replace the derived interpretation with an explicit reviewed N:M
component.

Fee legs are loss sinks of their allocation source. Their asset and any named
wallet must match that source, as must their Bitcoin rail/network scope. A
destination-paid or third-asset fee must be a separate source-to-fee edge from
the wallet/asset that actually lost value. That source must also fund an owned
transfer or external disposal row: a fee-only source cannot be projected and
therefore cannot activate.

## Conversion components

Unlike-quantity conversions require explicit review, policy, and exact balanced
valuations. Kassiber currently accepts exactly one quantity source and one owned
destination (plus explicit compatible loss legs); more complex conversions
must be split into auditable components. Profile-fiat valuation units such as
`eur` and `eur-cent` are projected into journal pricing. Other units remain
conservation evidence and still pass through the normal pricing gate. A fee or
external-loss leg cannot exist only as fiat valuation with zero asset quantity:
such a revision is not activatable because the journal cannot dispose of value
without an explicit priced quantity leg. A conversion fee's sink quantity must
equal the source quantity allocated to it, and its exact valuation must be the
same proportional source value the journal will book. This prevents an authored
fee amount or valuation from activating and then being silently replaced by a
different projector interpretation.

## CLI and desktop

The desktop **Swap Matching → Close gaps** tab previews and bulk-activates the
same JSON contract. The daemon and compiled/browser allowlists expose only the
specific `ui.transfers.components.*` kinds.

```bash
# Preview without persisting components or placeholder wallets.
kassiber transfers components bulk-resolve --file migrations.json --dry-run

# Activate the complete batch atomically.
kassiber transfers components bulk-resolve --file migrations.json

# Save incomplete work without affecting accounting.
kassiber transfers components bulk-resolve --file migrations.json --draft

kassiber transfers components list
kassiber transfers components show --component-id <id>
kassiber transfers components update --component-id <id> --file revision.json
kassiber transfers components activate --component-id <id>
kassiber transfers components supersede --component-id <id> --reason "bad evidence"
kassiber transfers components undo --component-id <id>
```

The operation flag is authoritative: embedded JSON cannot override `--draft`
or a desktop “Save as drafts” action. Desktop preview calls the same daemon
validation inside a rollback-only transaction, including anchor, scope,
conflict, placeholder-wallet, and conservation checks.

Updates create immutable revisions. Replication preserves concurrent revisions
and out-of-order revision links; derived active memberships rebuild only after a
complete replay. Competing active revisions stay visible and ineffective until
the user resolves them. Signed cross-replica version-vector dependencies are
deferred until their complete prefix arrives, so mailbox delivery order cannot
apply a component before its wallet or transaction anchors. Same-id rewrites of
economic header, leg, or allocation facts are rejected at both SQLite and replay
boundaries; correction means a new revision, while lifecycle transitions remain
legal. Every new header commits to its exact leg/allocation counts, so later
child inserts and direct or signed child/header deletes cannot rewrite the
revision; whole profile/workspace deletion can still cascade. Transaction
fingerprint deduplication retains the existing signed wire identity for rows and
references, preventing a device-local id alias from creating a false tombstone
or leaking into a component anchor. Arbitrary local evidence and leg
`location_ref` values stay behind the daemon boundary; renderer-safe edits retain
those hidden values by immutable leg id without returning them to the webview.
