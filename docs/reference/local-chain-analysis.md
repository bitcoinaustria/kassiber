# Local chain analysis

Open **Extras → Chain analysis** (`/chain-analysis`). The investigation workspace
is available without developer mode. It reads the active book and its local
reference observations; opening it does not contact an explorer or sync a wallet.

## Investigating a movement

Start with an overview or enter a transaction ID, outpoint, address, wallet ID
(`wallet:<id>`), local transaction record or canonical graph node ID. Choose
Bitcoin or Liquid and the network when an identifier is ambiguous. Trace forward
to destinations, backward to sources, or find paths between two subjects. Select
a node to inspect its evidence and continue the trace. Pan, zoom and fit the
graph; the table-like evidence panels retain the exact data behind it.

The engine distinguishes:

| Connection | What it establishes |
| --- | --- |
| Creates / spends | A physical output or an input referencing an exact outpoint in one chain/network |
| Custody | A current native or reviewed economic relation from the canonical journal, including cross-rail movements |
| Hypothesis | Reversible common-input, reuse, change or explicit cluster-defining label inference with stated premises |

A physical path establishes transaction connectivity. It does not identify which
input paid a particular output, establish a common owner, or trace uniquely
identified satoshis. Hypothesis edges overlay the selected graph; enabling them
does not turn them into physical path evidence. Stale journal relations remain
inspectable but cannot be traversed. Draft components and missing intermediate
connections never become authoritative edges.

Owner, public and disclosed observer views separate the user's private knowledge
from the public transaction graph. Selecting a view is an analytical assumption,
not permission to export it. Unknown confidential amounts stay unknown; they do
not become zero or prevent exploration of otherwise visible topology. Amounts
are exact decimal msat strings. Asset and network domains remain distinct.

## Analytical panels

- **Findings and clusters:** address/script reuse, common-input and change
  hypotheses with collaborative-transaction exclusions and reversible premises.
- **Patterns:** fan-in, fan-out, reconvergence, postmix reconvergence and
  amount-backed peel-chain candidates over multiple transactions.
- **Exposure:** direct and bounded indirect paths to locally sourced labels,
  with the claim, revision and connection types. A mixer label describes a
  source's attribution; it is not a verdict about the user or their funds.
- **Entropy:** actual bounded enumeration of compatible paired input/output
  partitions. Every participant group must pay a nonnegative mining fee, and
  the model assumes no transfers between participant groups. Results include
  exact counts, conditional link counts and computation limits. Incomplete
  enumeration reports a bound, not an exact number. Payjoin/joint-payment
  semantics, missing amounts and confidential values are explicitly unsupported
  by this partition model. It is not a calibrated privacy score.
- **Coverage and frontier:** missing observations, unavailable successor
  coverage, conflicting/retracted observations, stale custody evidence, filters
  and exhausted budgets. A branch stopping does not prove an origin or an
  unspent output.

Known collaborative metadata and observed equal-output shapes constrain
heuristics. A shape detector cannot prove that every CoinJoin or Payjoin was
recognized. Having intermediate wallets improves private ownership and custody
evidence; it does not reveal other participants' private allocation. Cooperative
Taproot swaps and private Lightning routing still need the corresponding local
or reviewed economic evidence.

## Expanding local observations

The acquisition panel first computes a plan using an explicitly selected
configured backend. The plan binds the book snapshot, query, routing identity
and bounded effects. Applying it is a separate action. A changed plan or book
requires a fresh preview. There is no implicit public-backend fallback.

| Backend | Acquisition coverage |
| --- | --- |
| Bitcoin Core | Raw transactions; historical forward spends require a synchronized Core 31 `txospenderindex`. Arbitrary historical transaction reads may need `txindex`; pruned/missing history remains visible. |
| Esplora / Liquid Esplora | Transaction graphs and per-output spend history supplied by that selected indexer |
| Electrum | Raw ancestor transactions; this query has no universal historical spender index or independent confirmation proof |

Acquisition checks the network genesis before fetching the subjects. Bitcoin's
standard networks and Liquid mainnet have built-in consensus pins. Custom Liquid
networks require `genesis_hash` through the CLI/API. Default limits are three
transaction hops and 50 transactions; hard maxima are ten hops and 200
transactions. Requests and response size are bounded. A 45-second scheduling
deadline stops new requests and response chunks; pending reads have an inactivity
timeout of at most eight seconds. Partial results state why each branch stopped.

Fetched bytes are sanitized and saved atomically as profile-scoped reference
observations, separate from wallet sync, accounting rows and custody decisions.
Witness stacks, preimages, private wallet material and backend credentials do
not enter the graph. Confirmation/spend status records what the backend observed
at acquisition time; refresh is explicit. Reacquiring observations can change a
saved-case comparison without rewriting the original case.

## Saved investigations and labels

Save records an immutable query result after the server recomputes and checks
its snapshot fingerprint. The fingerprint includes observation, ownership,
custody and active-label inputs; the stored result also has a content-integrity
digest. Load a historical case or compare it against another case/current local
data. Comparisons return added, removed and changed nodes and edges. Case deletion
does not delete the underlying observations.

Labels are local attribution claims with an explicit source, category,
confidence and revision. Edits/deletions check the current revision and append
history. `cluster_defining` defaults to false. A structured import accepts up to
5,000 labels atomically; it does not silently rewrite existing claims. No entity
dataset is downloaded or assumed to be authoritative.

The graph/result can be exported as JSON or CSV from the workspace. Exports are
explicit files containing the chosen investigation data. The new local tables
live in the project's SQLite/SQLCipher database and do not enter authored
cross-device replication.

## CLI and agents

All interfaces call the same focused core service. Examples:

```sh
kassiber --machine chain-analysis overview
kassiber --machine chain-analysis trace TXID --chain bitcoin --network main --direction backward --depth 8
kassiber --machine chain-analysis path TXID_A TXID_B --direction forward
kassiber --machine chain-analysis entropy TXID --max-states 200000
kassiber --machine chain-analysis cases list
kassiber --machine chain-analysis cases save --title 'Withdrawal route' --query @query.json --expected-snapshot-id SNAPSHOT
kassiber --machine chain-analysis cases compare CASE_ID
kassiber --machine chain-analysis labels import --document @labels.json
kassiber --machine --output plan.json chain-analysis acquire plan TXID --backend my-core --chain bitcoin --network main --direction both
kassiber --machine chain-analysis acquire apply --plan @plan.json
```

`query.json` contains the query object from an analysis result. CLI document
inputs accept a JSON object or `@path`; a machine response envelope is unwrapped
when supplied to a subsequent operation. `labels.json` has `{ "items": [...] }`
with `subject`, `chain`, `network`, `label`, `category`, `source`, `confidence`
and optional `cluster_defining` per item. Commands accept explicit
`--workspace`/`--profile` selectors.

In-app tools are `ui.chain_analysis.query`, `.entropy`, `.cases.*`, `.labels.*`
and `.acquire.{plan,apply}`. Mutations require chat consent; acquisition apply
always requires once-only consent. Only on-device providers can receive and
apply acquisition artifacts. A provider called "local" but hosted on the LAN
does not count as on-device.

Remote providers receive a curated topology/amount projection with opaque,
process-and-book-scoped references, instead of public txids, addresses, outpoints,
wallet identities or attribution prose. They can use those references to
continue a query. References expire on daemon restart; re-reading a case creates
new references. Topology and amounts themselves may be identifying: this is
minimized disclosure, not an anonymity guarantee. The desktop's **Ask assistant**
handoff obtains the same projection before constructing the prompt. Read errors
and stale case failures obey the same identity boundary.

## Implementation and verification

`core/chain_analysis/` owns immutable evidence indexing, bounded traversal and
pure analytical algorithms. `chain_analysis_api.py` dispatches explicit
operations; storage, acquisition and AI projection have separate modules. The
CLI and daemon contain thin adapters. Accounting still comes from
`custody_journal`; no investigation-specific accounting interpreter exists.

Tests include independent topology and partition-count oracles, collaborative
counterexamples, cross-network isolation, confidential values, observer
visibility, stale/reorg/conflicting evidence, indexed scale, save/import
atomicity, concurrent revisions, provider projection and backend budgets.

`./scripts/integration-harness.sh chain-analysis` creates its own disposable
Core 31 Docker node with `txindex` and `txospenderindex`, mines actual movements
through an intermediate wallet, acquires them, then verifies a later spend in a
saved-case comparison. It removes only its own container on exit.

This is not a global Bitcoin index, a commercial attribution database, an
automatic background watchlist, or an arbitrary-chain analytics platform.
PSBT v2 preflight is still separate work. Research rationale and primary
protocol references remain in [the architecture research](../plan/17-local-chain-analysis.md).
