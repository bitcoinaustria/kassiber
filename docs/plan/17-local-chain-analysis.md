# Local chain analysis

Status: research baseline plus implemented investigation module, 2026-09-06.
The pre-implementation audit below was made against `main` at `593285e6a`, after
PRs #543, #542, #544, #548 and #549. It is retained as the rationale and expansion
map; the table describes that older baseline, not the current feature set.

The shared indexed engine, desktop workbench, CLI/chat queries, observer views,
reversible hypotheses, local label exposure, bounded compatible-partition
entropy, explicit backend acquisition and immutable saved-case comparisons are
now implemented. Read [the current interface and limits](../reference/local-chain-analysis.md)
for shipped behavior. PSBT v2, background watchlists, full-chain indexing and
commercial attribution datasets remain outside this implementation. Execution
and remaining acceptance work live in [TODO.md](../../TODO.md).

## Product outcome

Kassiber should answer a concrete question about a transaction, output, wallet
or route with reproducible local evidence: what happened, what can be linked,
who can see that link, where the explanation stops, and what additional input
would resolve it. Bitcoin, Liquid and the user's Lightning records are the
initial scope. A chart or a heuristic count is not sufficient evidence of this
capability.

Transaction understanding and privacy analysis need the same observed graph.
They ask different questions of it. Accounting adds reviewed economic meaning;
privacy adds a particular observer's knowledge and explicitly defeasible
inferences. A co-spend proves that inputs participate in one transaction. It
does not, by itself, prove a common owner or a unique input-to-output value path.

## Audited baseline before implementation

| Capability | What the existing code actually does | Missing capability |
| --- | --- | --- |
| Transaction inspection | One transaction's inputs, outputs, fees, local ownership, confirmation context and current custody annotations; at most 250 strands per side | Bounded recursive exploration with an explicit frontier and complete paths |
| Wallet ownership | Inventory, exact receive outpoints, scripts and active/retired public wallet policies | A foreign address is not identified just because it is adjacent to an owned one |
| Local observations | BDK/LWK/Core sync, stored vin/vout, spent outputs and an optional reference-graph cache | A reusable index over all these observations; imported book history is not the entire chain |
| Privacy linkage | Owned Bitcoin/BTC output reuse, co-spend/change hypotheses, observer views | General external-neighbour graph; Liquid/Lightning graph analysis |
| Transaction hygiene | Separate engine for equal-output patterns, explicit collaborative metadata, round amounts/rates, script/witness/locktime tells, reuse and dust | Mirror's old static catalog did not mean these checks ran on its dataset |
| CoinJoin / Payjoin | Native custody interpreter can conserve observed owner deltas and attributable fees, including collaborative transactions | Public shape alone cannot prove participation or reconstruct a private participant mapping |
| PSBT preflight | PSBT v0 unsigned transaction parsing and simulations against local inventory | PSBT v2; validated input-map prevout data; explicit input evidence completeness |
| Swaps | Source-bound, cardinality-checked native HTLC claim/refund evidence and reviewed economic relations | One investigation exposing the complete lifecycle and every related leg |
| Cross-rail custody | Canonical custody lineage separates quantity/custody state from basis state; current graph detail shows one related swap route | All relevant routes, proof types, missing legs and uncertainty in one query |
| Source of funds | Reviewed allocations, evidence, assembly, immutable cases, amount conservation and export gates | Physical output tracing must not reuse accounting allocations as unique satoshi paths |
| Search and history | Local transaction search, details, metadata history and book review tools | Path queries, graph differences, reproducible investigation snapshots and watchlists |
| Entity labels | User-authored metadata and reviewed evidence | Versioned local attribution datasets, exact matches, conflicts and label provenance |
| Entropy / anonymity | No Boltzmann engine | Bounded compatible-assignment analysis with explicit assumptions and computation limits |
| Peel / postmix analysis | No general multi-hop detector | Explainable path patterns, reconvergence and later evidence that changes linkability |
| Chain coverage | Wallet sync freshness and reorg handling exist | Requested graph depth, available branches, chain tips and backend/index coverage in one result |
| Agents | Read-only graph/privacy/review-context tools and scoped missing-input handoffs | Queries over a shared investigation; model-created edges must never count as observations |

Code entry points:
[transaction_graph](../../kassiber/core/transaction_graph.py),
[ownership](../../kassiber/core/ownership.py),
[onchain](../../kassiber/core/onchain.py),
[privacy_linkage](../../kassiber/core/privacy_linkage.py),
[privacy_hygiene](../../kassiber/core/privacy_hygiene.py),
[reports](../../kassiber/core/reports.py),
[native transitions](../../kassiber/core/custody_native_transitions.py),
[custody snapshots](../../kassiber/core/ui_snapshot.py),
[source-funds review](../../kassiber/core/source_funds_review.py).

PR #543 already supplied the important native custody and review foundation.
Reusing it is preferable to another transfer matcher inside privacy analysis.
PR #542 and the RP2 update concern economic basis; they do not turn a basis
allocation into a physical ownership proof. The import validation and dependency
updates do not add an investigative chain index.

The audit also reproduced specific defects in the older analytical projections:
collaborative common-input unions, a positive fee mislabeled as a fingerprint,
counterparty tells penalizing the receiver, score calculation after truncation,
transaction-level source capacity counted for each sibling output, and
cross-network successor/block-height contamination. Correcting these is a
prerequisite to expanding their reach. The current correction scope and tests
are tracked separately from future capabilities in TODO.

## One shared investigation module

The external interface should stay small: inspect a subject, trace a bounded
question, and compare two result snapshots. Capability and coverage information
is part of every result. These are proposed operations, not current CLI flags.
Desktop, CLI and AI use the same implementation and explanation codes.

Internally compose three existing responsibilities:

1. An observation index built from normalized chain records and source-bound
   observer provenance. Reuse `onchain` parsing and ownership resolution.
2. Hypothesis evaluation over that immutable index. Reuse corrected privacy
   detectors, recording their premises, exclusions and rule version.
3. Economic/evidence annotations from current custody and source-funds readers.
   These readers retain authority over their own semantics. Investigation never
   invokes RP2, rebuilds journals or writes an accounting interpretation.

Start with a rebuildable SQLite projection and indexed adjacency over the
existing observers. Measure the workload before choosing a full-chain storage
engine. Current privacy consumers perform several full-profile reads and do
not share one immutable observation snapshot. Repeatedly calling the existing
per-TX renderer over a profile is not the traversal implementation.

### Facts and relations

- Transaction identity: `(chain, network, txid)`; output identity adds `vout`.
  Asset and confidential-value visibility are explicit leg properties.
- Physical edges: an output is created by a transaction or named by a spending
  input. Conflicting spenders are alternatives, not an arbitrary first row.
- Observation provenance: source, observation time, block hash/height, tip,
  confirmation/retraction state and content/version binding. Cached reference
  data is distinguishable from current native wallet authority.
- Private ownership: wallet evidence identifies controlled inputs/outputs and
  change. This does not automatically become public observer knowledge.
- Hypotheses: possible common control, change, peel continuation, CoinJoin
  shape, fingerprint or amount/time correlation. Record rule version, premises,
  counterevidence and eligible population; merges must be reversible.
- Economic relations: reviewed transfer, source allocation, swap, deposit,
  withdrawal or channel lifecycle. Preserve native-proof versus reviewed-source
  versus candidate status instead of coercing them all into physical edges.
- Attribution: a sourced label about an address/entity at a given time. The
  fact is that the source made the claim; propagation to a cluster is separate.

Keep amount arithmetic in exact integers with explicit units and assets. Do not
turn unknown confidential values into zero, or apply one asset's decimals to
another. A FIFO, haircut or poison value-flow allocation is a declared model,
never proof that particular satoshis travelled through a selected output.

### Query and coverage contract

A trace specifies direction, subject/domain, depth, time interval and budgets
for nodes, edges, wall time and returned evidence. Stable cursors and result
snapshots bind to the observation, ownership, custody/source-funds, label-dataset
and rule/model revisions, observer perspective and query parameters used by the
explanation. Read them in one consistent snapshot and bind its fingerprint to
the result. Evidence edits can change an answer without changing the chain.
Each stopped branch has a reason: missing parent,
spender coverage unavailable, confidential value, private payment, competing
spend, ambiguous ownership, stale observation, unsupported protocol or budget.
Reaching the budget must not look like reaching the origin of funds.
Stopping is specific to the question: unknown confidential values stop amount
inference, not traversal of visible topology. Return separate completeness for
topology, amounts/assets, ownership and economic explanation.

Compute aggregates before presentation pagination. Return observed amounts,
unknown amounts/counts, paths and alternatives, rather than an invented
confidence percentage. Keep these separate: certainty of an observation,
strength of an ownership inference, observer visibility, and economic meaning.
A legacy privacy score can only be a disclosed prioritization heuristic; it is
not an empirically calibrated probability of deanonymization. The existing
model can return a high number for an empty or poorly observed book. Replace
the headline grade with a coverage-aware investigation summary so that absent
signals cannot read as a completed assessment.

Reorg/replacement handling retracts affected observations and dependent
hypotheses. A saved result describes its historical tips/revisions; it does not
silently adopt the current graph. Derived indexes remain local/rebuildable and
outside authored-data replication. No raw descriptors, preimages or full
private Lightning routes enter the analysis payload or an AI provider request.
Reuse existing AI redaction and capability scoping, including attachment
URLs/paths and private identity-graph data; local operator views and provider
payloads are distinct permissions, not one universal graph dump.

Separate immutable transaction shape from mutable inclusion/spend status.
The existing sanitized reference cache stores reduced vin/vout, not raw
witnesses. Its presence or update timestamp proves neither current confirmation
nor unspentness nor availability of witness-level contract evidence. Enrichment
must preserve stronger current wallet observations and mark cached-reference
provenance explicitly.
Witness-derived findings must retain validated source-bound extraction evidence;
the reduced cache cannot recreate witness completeness it never stored.

## Data acquisition is a separate action

Local-only inspection consumes already stored observations without network
access. Expanding the graph produces a bounded fetch plan for a configured
backend; execution uses the existing egress/consent mechanisms. A self-hosted
node and a third-party explorer are different observers even if both return the
same transaction bytes. A browser-side calculation that queries a public
explorer is not query-private.

| Source | Useful capability | Constraint to expose |
| --- | --- | --- |
| Existing book/inventory/cache | Own histories and previously retrieved graph records | Missing neighbours are unknown, not unspent |
| Bitcoin Core | Raw transactions/blocks, own-wallet prevouts, chain state | Probe actual version, `txindex`, block/undo availability and pruning |
| Core 31 with `txospenderindex` | Historical output-spender lookup | Probe index availability/sync; older or index-less mempool lookup is not historical coverage |
| Self-hosted Esplora | Transactions, address/script history and outspends | Server index coverage/tip and request budgets |
| Configured Electrum | Script history and transaction retrieval | It is not a universal outpoint-spender API; requests reveal queried script hashes to that server |
| LWK/Elements | Liquid topology and wallet-specific unblinded values | Keep absent amounts/assets confidential; never persist new secret material for analysis |
| Own Lightning node / wallet export | Own payments, invoices, channel lifecycle and local forwarding | It cannot observe unrelated remote payment paths |
| Imported label/evidence bundle | Local exact matches with reproducible dataset digest | Source quality, age, conflicts and redistribution terms |

Core 31's historical spender support is documented, not implemented by this
proposal. It should be capability-probed rather than inferred from a version
string alone. The official RPC specifies the index-dependent default and full
spending-transaction option. [Core spender RPC][core-spender] [Core raw TX RPC][core-tx]
Esplora exposes transaction structure, confirmation status and outspends through
separate interfaces; the adapter must preserve that distinction. [Esplora API][esplora]

## Cross-rail results that are useful and defensible

| Route | Useful evidence and answer | Honest stopping point |
| --- | --- | --- |
| Bitcoin → another owned Bitcoin wallet | Exact outpoint spend, local ownership, fee, timing and competing observations | Collaborative input/output assignment may remain ambiguous despite known wallet deltas |
| Bitcoin ↔ Liquid peg | Verify the peg-in claim's Bitcoin transaction/inclusion proof; represent peg-out request and Bitcoin settlement separately | Federation settlement is not a simple same-coin edge; hidden Liquid amounts require appropriate wallet data |
| Bitcoin/Liquid ↔ Lightning swap | Lockup, claim/refund, source-qualified payment hash, provider record, fees and missing lifecycle legs | Cooperative Taproot key-path spends do not expose a script-path preimage link |
| Bitcoin ↔ Lightning channel | Funding/closing/splice facts and own node accounting; verified signed public gossip where available | A 2-of-2-looking output alone does not prove a Lightning channel; public gossip does not disclose private payments |
| Lightning payment ↔ later receipt | Own endpoint records and valid protocol/provider correlation | No arbitrary end-to-end path or certainty from matching amounts/timing |
| Exchange deposit ↔ withdrawal | Own platform exports/API records and chain settlement, explicitly labeled custodial history | Omnibus custody breaks a unique physical coin path; platform history is required |

Liquid retains visible transaction topology while hiding confidential amounts
and assets. Additional connected wallets can reveal the user's own values,
not everyone else's. Elements documents both the peg workflow and audit access.
[Elements peg workflow][elements-peg] [Confidential Transactions][elements-ct]

Lightning gossip can bind an announced funding output to node keys. Onion
routing deliberately limits route knowledge; blinded paths limit it further.
Own-node facts remain valuable without pretending to reconstruct remote
payments. [BOLT 7][bolt7] [BOLT 4][bolt4]

Boltz's script-path and cooperative key-path protocols supply different
evidence. Verify contract/output/lifecycle data instead of assuming a shared
hash always proves a unique route. [Boltz claims][boltz-claims]
[Boltz verification][boltz-verify]

## Further analytical capabilities

Forward/backward traversal, reachability, bounded alternative paths, fan-in,
fan-out and reconvergence should precede new scoring. Then add peel/batch/
consolidation hypotheses using multi-transaction evidence. Address clustering
must allow exclusions and contradictory change candidates; aggressive unions
can create false superclusters. [BlockSci clustering][blocksci-clustering]
[Peel-chain research][peel]

Payjoin deliberately invalidates common-input, round-amount and script-type
change assumptions. A detector can recognize evidence or a suggestive shape;
it cannot certify that an ordinary-looking transaction is non-collaborative.
[BIP 78][payjoin]

For entropy, compute compatible input/output assignments under published fee
and participant assumptions. Return exact/model-bounded/timed-out/unsupported
states, candidate sets and deterministic links *under those assumptions*.
This is not an ownership probability. Do not add independent transaction
entropies across a path. Subsequent reuse/reconvergence can eliminate prior
assignments and requires recomputation. [Boltzmann methodology][boltzmann]

Entity work should begin with user evidence and local, versioned TagPacks:
issuer, network, exact subject, digest, observation date, confidence, expiry and
conflicts. Default cluster-defining labels to false. A dataset label is not
current ownership proof, legal culpability or a reason to relabel every
reachable coin. Screening can report direct/indirect contact with a named
dataset and its limitations, not an opaque taint verdict. [GraphSense tags][tags]

PSBT work should reuse the same graph after validating the supplied previous
transactions/output references. Support formats explicitly; require consistent
amount/script evidence and report what the signer would disclose to each
observer. No signing or coin-selection implementation is needed for analysis.
[PSBT v0][psbt0] [PSBT v2][psbt2]

## Agent workflow and acceptance examples

The assistant starts with a subject and a question, queries the deterministic
result, and cites returned evidence references. Missing input uses the shipped
connection/import/attachment handoffs. A fetch, evidence edit or accounting
review remains its own typed, consented action. The model may propose a
hypothesis; only recorded observations and validated rules can establish it.

The interface should support questions such as:

- Which locally known ancestors fund this output, and where does tracing stop?
- Which formerly separate public clusters would this PSBT link?
- Does this CoinJoin output later reconnect to a pre-mix cluster, and through
  which observed edges rather than inferred input/output allocation?
- Which parts of this Bitcoin → Liquid → Lightning → Bitcoin route are native
  proofs, own provider records, reviewed assumptions or missing observations?
- What can the exchange infer from this deposit using public data and the
  evidence I disclose, versus what only my connected wallets tell Kassiber?
- What changed after a newly connected intermediate wallet, a replacement
  transaction, a reorg or a corrected entity label?

Acceptance fixtures must include positives *and* near-identical negatives:
identical txids/scripts on different networks; co-spend with and without
collaboration evidence; unknown Payjoin-shaped transactions; partial source
coverage across siblings; fee paid by a foreign participant; RBF alternatives;
orphaned blocks; pruned/index-less Core; stale caches; Liquid hidden/known and
foreign-asset values; native HTLC versus copied import metadata; cooperative
Taproot with/without provider evidence; Lightning private-route gaps; entropy
budget exhaustion; conflicting labels; and output pagination that does not
alter aggregates. Use independent quantity/path oracles, egress-denial tests,
and existing Docker regtest instead of rehearsed prepared links.

## Dependency choices

Reuse the existing watch-only BDK/LWK/Core adapters and SQLite first. BlockSci
is a valuable algorithm reference but declares itself unmaintained since 2020;
its GPL-3.0/C++ deployment is a poor default desktop dependency. Its old memory
figures are historical, not a current hardware estimate. [BlockSci repository][blocksci]

Boltzmann and Copexit offer useful implementation references, subject to
per-artifact license and bounded runtime review. Copexit's client-side analysis
does not remove the privacy implications of its default explorer retrieval.
Port validated techniques, not its heuristic count or confidence claims.
[Copexit methodology][copexit]

Broader EVM/token/bridge analysis would need separate account-state, event-log,
execution-trace and asset semantics plus appropriate nodes/indexers. It is a
distinct future adapter/product scope, not something to simulate with Bitcoin
outpoint keys. A whole-chain entity database is likewise a separate optional
dataset/index workload; all connected watch-only wallets alone cannot supply it.

[core-spender]: https://bitcoincore.org/en/doc/31.0.0/rpc/blockchain/gettxspendingprevout/
[core-tx]: https://bitcoincore.org/en/doc/31.0.0/rpc/rawtransactions/getrawtransaction/
[elements-peg]: https://elementsproject.org/elements-code-tutorial/sidechain
[elements-ct]: https://elementsproject.org/elements-code-tutorial/confidential-transactions
[bolt7]: https://github.com/lightning/bolts/blob/master/07-routing-gossip.md
[bolt4]: https://github.com/lightning/bolts/blob/master/04-onion-routing.md
[boltz-claims]: https://api.docs.boltz.exchange/claiming-swaps.html
[boltz-verify]: https://api.docs.boltz.exchange/dont-trust-verify.html
[blocksci-clustering]: https://github.com/citp/BlockSci/wiki
[peel]: https://www.usenix.org/conference/usenixsecurity22/presentation/kappos
[payjoin]: https://bips.dev/78/
[boltzmann]: https://gist.github.com/LaurentMT/e758767ca4038ac40aaf
[tags]: https://github.com/graphsense/graphsense-tagpacks/wiki/GraphSense-TagPacks
[blocksci]: https://github.com/citp/BlockSci
[copexit]: https://github.com/Copexit/am-i-exposed/blob/main/docs/privacy-engine.md
[esplora]: https://github.com/Blockstream/esplora/blob/master/API.md
[psbt0]: https://bips.dev/174/
[psbt2]: https://bips.dev/370/
