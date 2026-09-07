# Local chain analysis

Open **Extras → Chain analysis** (`/chain-analysis`). The investigation workspace
is available without developer mode. It reads the active book and its local
reference observations. Opening the graph runs one bounded local overview; a
transaction or Privacy Mirror investigation link runs its selected local query.
Editing controls requires **Run**. PSBT and dataset links do not run a graph
query. None of these entry actions contacts an explorer, syncs a wallet,
acquires evidence or starts an entropy job.

The query bar holds the starting point and **Run**. Perspective, direction,
query type, path target, chain/network filters, budgets and optional layers sit
under **Advanced**. Invalid hidden fields reopen the disclosure before browser
validation focuses them. The executed-context strip shows effective visibility,
domain and coverage, independently of edited controls. One result selector opens
findings, paths, patterns, clusters, attribution, coverage or advanced tools;
canonical deep links retain their exact subview. The inspector appears only
for a selection. **Tools** opens PSBT inspection, attribution datasets, saved
investigations, explicit evidence acquisition and exports.

Settings → Bitcoin → Local analysis stores the default PSBT network. An explicit
handoff network takes precedence, and each inspection pins its network at mount;
changing the default never relabels existing results. The graph's chain/network
filters remain query scope, not permission to access any backend.

Transaction details keep their Mempool-style input/output diagram. They also
reuse compatible transaction references already stored in the same profile,
including Chain Analysis acquisitions, without a network request. References
must agree on chain, network, transaction identity and known physical facts;
conflicting or over-limit sources leave the original graph unchanged. Reusing
these references never grants wallet ownership or journal custody authority.

Ownership in transaction diagrams requires locally matched scripts or exact
outpoints within the same chain and network. An unmatched leg stays unknown;
an amount match is only a possible receipt. A return to an input-owning wallet
is distinct from change, which also requires explicit change-branch evidence
and an ordinary spend without a known collaborative boundary. These annotations
do not change accounting.

An exact transaction-row detail shows that wallet leg's principal and fee.
The table and chart keep their net transfer projection; they are not substituted
for the source record when following a row link.

Known same-asset transfers between an exchange account and an on-chain wallet
show an exchange withdrawal/deposit summary with named custody endpoints.
The default diagram follows the on-chain wallet leg. A distinct exchange reference
appears inline; duplicate blockchain references and booking timestamps are omitted.
Exchange-reported fees stay separate from miner fees. Ambiguous importer kinds retain the generic
transfer view, and swaps keep their route selector.

An exchange trade without blockchain input/output data remains an accounting
record without a physical flow diagram. Unlinked purchases and sales show a
compact trade receipt; fiat trade values require exact exchange-execution
provenance and never substitute a market valuation. Missing counts stay unknown, imported
fees do not become miner fees, and the detail view omits empty technical fields.
The investigation link follows the displayed transaction and requires a scoped
blockchain identity.

[Privacy Mirror](privacy-mirror.md) is the concise public-observer summary of
this same engine. `analyze_snapshot` composes an immutable canonical index after
observer projection; Mirror adds personal relevance, executed-check coverage and
bounded solver summaries. It has no separate graph, detector or grade. Deep
links preserve the physical subject and observer/domain; PSBT and dataset tools
stay in this workbench.

Graph interaction lives in `ui-tauri/src/components/evidence-graph/` and is shared
with Source Funds. Route adapters provide labels, visual edge styles and exact
local identities; the renderer does not fetch data or interpret ownership,
heuristics or provenance. Chain Analysis keeps its existing observation adapter;
Source Funds supplies reviewed report edges independently.

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
  partitions, including explicit participant-fee scenarios. Results include
  exact counts, conditional link counts and computation limits. Incomplete
  enumeration publishes only a lower bound and withholds normalized linkage.
  Payjoin/joint payments use the proposal comparison described below; they do
  not fit the independent-flow partition model. Missing/confidential amounts
  remain unavailable. This is model uncertainty, not a calibrated privacy score.
- **Transaction features:** one versioned pipeline for stored observations,
  acquired transactions and PSBTs. It extracts version, sequence/RBF/relative
  lock constraints, effective absolute locktime, script mixes, witness shapes,
  observed signature encodings, repeated amounts and output ordering. Every
  feature carries availability and evidence. Rules suppress missing or
  contradictory prerequisites; low-R or ordering observations do not identify
  a wallet vendor. Legacy privacy hygiene/linkage reuse this pipeline.
- **Coverage and frontier:** missing observations, unavailable successor
  coverage, conflicting/retracted observations, stale custody evidence, filters
  and exhausted budgets. A branch stopping does not prove an origin or an
  unspent output. Retracted spenders remain historical evidence but do not
  compete with a live replacement. Unresolved current competitors block value
  inference, including exact conditional entropy counts.

Known collaborative metadata and observed equal-output shapes constrain
heuristics. A shape detector cannot prove that every CoinJoin or Payjoin was
recognized. Having intermediate wallets improves private ownership and custody
evidence; it does not reveal other participants' private allocation. Cooperative
Taproot swaps and private Lightning routing still need the corresponding local
or reviewed economic evidence.

## Conditional CoinJoin computation

The `independent` scenario requires each flow group to fund its outputs plus a
nonnegative mining fee. `coinjoin_intrafees` additionally permits explicitly
bounded net fees received/paid by each group. The solver verifies that at least
one globally conserved mining-fee allocation exists and counts each compatible
partition once. Fee-only participants and complete protocol-specific maker/
taker roles are not modeled. `generic`, `whirlpool`, `joinmarket` and `wabisabi`
are descriptive scenario labels; selecting one never supplies presumed fees
or proves that the transaction followed that protocol.

Amounts and counts use integer arithmetic and decimal strings. A value-class
dynamic program shares computation among equal-valued inputs/outputs while
preserving every labeled linkage count. It makes no independent-denomination
or greedy change-ownership approximation. The workspace compares scenario
results using exact fractions; differences remain conditional on the model.

Default budgets are 200,000 work units and one second. Explicit maxima are
2,000,000 units, 30 seconds, 128 inputs/outputs, 65,536 subset patterns per side
and 1,000,000 memo cells. Preparation is included in these bounds. Large unequal
populations may exhaust them. Repeated-value tests include exact 128×128 results
with 335-digit counts and a multi-denomination 11×11 adversarial case; this is
not a guarantee that every transaction of those dimensions is tractable.

## PSBT preflight and Payjoin proposals

The PSBT workspace accepts a native-selected binary/Base64/hex PSBT v0 or v2
with an explicit Bitcoin network. Investigation links preserve that network;
either file selection can be cleared to start over or inspect only the original.
Scripts alone cannot
verify a network. The shared parser validates map framing, v0/v2 constraints,
v2 locktime resolution and zero sequences, non-witness UTXO transaction hashes,
outpoint indices and witness/non-witness amount/script consistency. Supplied
UTXOs are evidence about the proposal; chain membership and unspentness still
require node observations. Missing amounts, final sizes and fees stay unknown.

Original/proposal comparison reports changed inputs, outputs, fees, features
and findings. Explicit BIP78 parameters enable Payjoin checks, including
preserved original inputs, output substitution/fee-contribution constraints,
receiver contribution and supported receiver signature commitments. P2WPKH and
P2SH-P2WPKH receiver commitment checks use embit; unsupported script verification
remains unavailable. Exact additional-input size caps require final stacks.
Negotiation, ownership and full consensus script execution are not asserted,
and the tool never returns signing authorization or broadcasts anything.

PSBTs use the same conditional entropy engine. Their decoded byte commitment
binds even changes in private maps or same-template output scripts. Raw PSBTs,
derivations, xpubs, keys and witness stacks never enter analysis results or the
database. Only safe categorical features survive acquisition's discard boundary.

## Expanding local observations

The acquisition panel first computes a plan using an explicitly selected
configured backend. The plan binds the book snapshot, query, routing identity
and bounded effects. Applying it is a separate action. A changed plan or book
requires a fresh preview. There is no implicit public-backend fallback.
Dispatch uses the exact backend and seed-transaction snapshot used to validate
the plan, so a later configuration or wallet change cannot redirect the
approved request. HTTP uses only the explicitly configured proxy, ignoring
system/environment proxy discovery and bypass rules, and refuses redirects.
`KASSIBER_NO_EGRESS=1` blocks acquisition before any request or initial Electrum
handshake, including loopback, and is rechecked between requests.

| Backend | Acquisition coverage |
| --- | --- |
| Bitcoin Core | Raw transactions with available prevout values/scripts from verbosity 2; historical forward spends require a synchronized Core 31 `txospenderindex`. Arbitrary historical transaction reads may need `txindex`; unavailable undo/pruned/missing history remains visible. |
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
data. Comparisons return added, removed and changed nodes, edges, findings,
exposure, clusters, patterns and feature snapshots, plus coverage changes. Case deletion
does not delete the underlying observations.

Labels are local attribution claims with an explicit source, category,
confidence and revision. Edits/deletions check the current revision and append
history. `cluster_defining` defaults to false. A structured import accepts up to
5,000 labels atomically; it does not silently rewrite existing claims. No entity
dataset is downloaded or assumed to be authoritative.

## Local attribution datasets

Dataset packs are separate from authored book labels. Import local CSV/JSONL
through the generic, am-i-exposed (`address,entity,source`) or Maru92 column
adapter. Supply a manifest containing `dataset_key`, `name`, `version`, `chain`,
`network`, `source`, `license`, `attribution_method` and `visibility` (`public`
or `private`). Optional source/observation/validity metadata retain provenance;
they are the importer's declarations, not a verification of the source's claim.
In particular, historical common-input clustering is a heuristic methodology.
Dataset licenses are independent of adapter code licenses; no external data is
bundled or automatically downloaded.

Preview streams and validates the entire source and returns its SHA-256,
counts and bounded sample. Import requires that hash. Records stage invisibly
in batches of 2,000; only the complete, matching source activates atomically.
Replacing an active dataset also requires its `expected_active_id`. Failures or
cancellation retain the prior active version and leave inspectable incomplete
imports that can be discarded in bounded batches. Complete versions retain
their provenance and can be revoked with `expected_revision`.

Exact domain-qualified transaction/outpoint/script indexes establish matches;
validated addresses normalize to scripts. No truncated hash or Bloom match
grants attribution. Conflicting claims retain separate source records. Queries
support a subject plus chain/network or an exact entity label within a dataset,
with state-bound pagination and explicit historical status. Validity windows
apply at lookup time, not automatically at the transaction's date. Public
observer results admit only public datasets and exclude private book labels.

Limits are 8 GiB/source, 50 million claims and 64 KiB/record. Matching examines
at most 100,000 observed subjects and 5,000 claims; graph analytics have their
own visible bounds. It never loads an entire attribution pack into graph RAM.
A generated one-million-row test imported in 14 seconds with approximately
20 MB additional peak RSS on the development Mac; four matches needed one
indexed lookup. Those measurements are examples, not service guarantees.

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
kassiber --machine chain-analysis entropy TXID --scenario @fee-scenario.json --max-states 2000000 --max-duration-ms 30000
kassiber --machine chain-analysis psbt analyze --file proposal.psbt --network main
kassiber --machine chain-analysis psbt compare --before original.psbt --after proposal.psbt --network main --payjoin @negotiated-parameters.json
kassiber --machine chain-analysis psbt entropy --file proposal.psbt --network main --scenario @fee-scenario.json
kassiber --machine chain-analysis datasets preview --file claims.csv --manifest @manifest.json --adapter am_i_exposed
kassiber --machine chain-analysis datasets import --file claims.csv --manifest @manifest.json --adapter am_i_exposed --expected-sha256 PREVIEW_SHA256
kassiber --machine chain-analysis datasets query --dataset-id DATASET_ID --label 'Exact entity name'
kassiber --machine chain-analysis datasets revoke DATASET_ID --expected-revision 1
kassiber --machine chain-analysis datasets discard INCOMPLETE_DATASET_ID
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

In-app tools include `ui.chain_analysis.query`, `.entropy`, `.cases.*`, `.labels.*`,
`.psbt.*`, `.datasets.*` and `.acquire.{plan,apply}`. `.entropy.start`,
`.psbt.entropy.start` and dataset `.preview.start`/`.import.start`/`.discard.start`
return computation receipts; `.jobs.get` reads progress/results and `.jobs.cancel`
requests cancellation. The CLI uses equivalent synchronous operations. Two
workers, sixteen retained receipts and one-hour expiry bound process memory;
dataset jobs have a thirty-minute work limit. Locking/switching projects clears
file grants and cancels work. A late cancellation does not relabel a committed
import as cancelled. Results describe the submitted snapshot.

Native file selection grants only a purpose/book-scoped token, expiring after
thirty minutes. File identity is checked before bytes/EOF reach the parser;
import additionally checks the reviewed content hash before activation. AI
schemas accept selected PSBT tokens, never paths or raw PSBT text. Mutations
require chat consent; acquisition and dataset import require once-only consent
with server-recomputed effects. Acquisition artifacts and dataset import/
preview provenance are available only to on-device providers. A provider called
"local" but hosted on the LAN does not count as on-device.
Acquisition cannot inherit session-wide consent: every application needs a
new once-only approval. Denial, cancellation, timeout, changed scope and stale
plans make no node request. Opening the page, planning, inspecting PSBTs and
importing local datasets do not contact node or attribution services. Source
URLs inside dataset manifests remain metadata and are never fetched.

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
Core 31 Docker node with `txindex` and `txospenderindex`. Five scenarios cover
missing intermediate recovery and saved-case differences, four-script PSBT/Core
parity, two independently signing collaborative wallets, a receiver-signed
Payjoin, and actual relative/absolute lock enforcement. It removes only its own
container on exit; see [the live oracle contract](testing.md#local-chain-analysis-oracle).

This is not a global Bitcoin index, a commercial attribution database, an
automatic background watchlist, or an arbitrary-chain analytics platform.
Research rationale and primary
protocol references remain in [the architecture research](../plan/17-local-chain-analysis.md).
