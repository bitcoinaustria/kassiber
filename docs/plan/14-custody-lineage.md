# Custody Lineage And Missing-Wallet Reconciliation

**Status:** Approved implementation target; executable work is tracked in
[`TODO.md`](../../TODO.md).
**Scope:** One profile is one legal owner. Kassiber reconciles that owner's
Bitcoin custody history; it does not model shareholders, fractional interests,
or a general ledger.
**Core invariant:** Every observed quantity is represented exactly once, while
unresolved custody never becomes a taxable event.

## Problem

Bitcoin custody history is not a stable list of wallet labels. Long-lived users
and organizations rotate multisig signers, replace descriptor policies, migrate
between script types, use privacy wallets, and later consolidate into new
operational wallets. An old policy can remain historically relevant after its
wallet is retired. Some intermediate policy material may be lost entirely.

A representative history is:

```text
Multisig A -> Multisig B -> Samourai Deposit -> Premix/Postmix
           -> missing Whirlpool history -> Operative C -> Multisig D
```

An on-chain transaction proves that quantities moved. An imported descriptor
can identify scripts covered by that policy. Neither proves that every wallet
ever belonging to the profile has been imported, nor that an unmatched output
left the legal owner's custody.

The current journal pipeline asks one row stream to answer three different
questions:

1. what physical quantity moved;
2. where the profile's quantity is now; and
3. whether the movement is taxable.

That coupling creates a false choice: retain a source-wallet balance that is
known to have left, or book an unresolved destination as an external disposal.
It also allows several interpreters to claim, withhold, restore, or synthesize
the same rows before RP2 sees them.

This plan separates physical evidence, custody interpretation, quantity
projection, economic classification, and tax projection. RP2 remains the tax
calculator; it receives only finalized tax events.

## Product Rules

### One profile, one owner

A profile represents one person or legal entity. A wallet assigned to that
profile is an assertion that the profile owns 100% of the relevant quantity.
Signer rotation, collaborative signing, or a change from one multisig policy to
another changes control policy, not the legal owner. Fractional ownership and
company-share ownership are outside this design.

### Open-world wallet discovery

Kassiber must never ask for or store a global declaration such as "all wallets
ever owned have been imported." Such a declaration is unknowable, becomes
stale, and could incorrectly promote every unmatched output to confirmed
external ownership.

Kassiber may verify narrow technical facts:

- a specific receive/change policy was imported;
- a specific policy was scanned through an index, height, or observation time;
- a wallet export reported an issuance bound at a particular time;
- a retired policy remains indexed for historical recognition; or
- no unresolved or conflicting gap is known in currently imported evidence.

The empty state is therefore **No known custody gaps**, with the qualification
that Kassiber cannot know whether unimported wallets exist. It is never
"ownership complete" or "all wallets present."

### Automation and review

- When complete imported policy evidence recognizes both sides of a custody
  movement, carry quantity and basis automatically.
- Ordinary unmatched outflows with no plausible return may remain
  `external_presumed`; reports must distinguish and count this presumption.
- A plausible return through missing history becomes `custody_candidate` and
  withholds the affected presumed tax classification.
- A candidate never activates automatically. One reviewed N:M bridge may close
  a complete missing-wallet interval and carry basis durably.
- A reviewed retained amount and an unresolved residual are independent. The
  retained amount may carry custody quantity while only the residual remains
  suspense. Because tax basis is pooled across wallets, later tax results that
  could change when the residual is classified remain provisional and
  report-blocked; custody finality is not tax-basis finality.
- New evidence may reopen a dismissed candidate or contradict a reviewed
  bridge. Kassiber records a new revision and never silently rewrites authored
  history.

## Vocabulary And State Model

### Custody domain

The profile's legal-owner boundary. A movement inside the domain carries basis;
a movement outside it still needs an economic/tax classification.

### Custody location

A wallet or protocol location at which the profile's quantity is observed or
reviewed to reside. Examples include a retired multisig, Samourai Postmix,
an LND node, a Liquid wallet, an Ark VTXO wallet, or a named untracked historical
wallet. A location is not itself an owner.

### Policy epoch

A versioned wallet-recognition policy with chain/network, descriptor or
address-source identity, receive/change branches, observed/valid horizon,
derived/scanned bounds, evidence provenance, and active/retired/missing state.
Coverage is always policy- and time-scoped. A high gap limit is strong discovery
evidence but not proof that an offline wallet never issued a higher index.

### Physical event

A canonical protocol event assembled from one or more immutable source
observations. Identity is protocol-qualified:

- Bitcoin/Liquid: chain, network, and canonical txid;
- Lightning: node/source-qualified native event identity;
- provider or custodial records: source-qualified stable identity.

Several wallet/import rows may observe one event. An observation is evidence,
not the event itself.

### Custody continuity claim

A versioned claim that selected source and destination quantities remained
inside the same custody domain, possibly through one or more missing locations.
It supports 1:1, 1:N, N:1, and N:M boundaries. Reviewed bridges compile to the
existing custody-component authored substrate; the normal UI does not require
users or AI to author low-level legs and allocation JSON.

### Classification states

| State | Meaning | Quantity behavior | Tax behavior |
| --- | --- | --- | --- |
| `internal_verified` | Imported policy/native evidence recognizes retained custody | Move/carry basis automatically | Non-taxable custody movement; actual fees remain separate |
| `internal_reviewed` | A user approved a durable missing-history bridge | Move/carry the reviewed quantity | Non-taxable custody movement; residuals remain separate |
| `external_confirmed` | Explicit reviewed payment, disposal, gift, loss, or other external evidence exists | Leave profile custody | Emit only the finalized selected economic event |
| `external_presumed` | Unmatched outflow has no currently plausible owned return | Leave observed wallet; disclose presumption | May enter tax projection under the documented default, but remains revisable |
| `custody_candidate` | Deterministic evidence suggests a possible missing-wallet return | No authored change until review | Withhold the affected presumed external event |
| `custody_suspense` | Quantity is known to have left an observed location but its custody destination/classification is unresolved | Reduce observed wallet and retain explicit suspense quantity | Never enter the finalized taxable-event projection |
| `conflicting` | Active interpretations or evidence disagree | Preserve all claims; choose none silently | Block affected quantity and reports |

These are interpretation states, not one overloaded quarantine flag. Report
readiness is separate: `clear`, `warning`, or `blocked`. A warning can disclose
presumptions; candidate, suspense, and conflict states block only their affected
tax quantities and any downstream lots that require them.

## Architecture

```text
canonical current observations
 + immutable authored evidence snapshots
              |
              v
canonical physical events + policy epochs
              |
              v
candidate custody claims from every interpreter
              |
              v
single custody arbitrator
       |                    |
       v                    v
quantity projection     economic classification
       |                    |
       v                    v
wallet/profile views    finalized tax-event projection
                            |
                            v
                           RP2
```

### Immutable authored evidence

New durable interpretations bind to a stable quantity-core hash and store the
canonical evidence-detail payload they reviewed. The current `transactions`
table remains the mutable compatibility/current view. Importer updates that no
durable claim referenced do not need permanent historical copies; authoring or
revising a claim transactionally snapshots the exact evidence payload first.
Reprocessing compares the current quantity/detail hashes with those immutable
snapshots, so quantity contradictions and evidence enrichment are visible
without retaining an unbounded history of irrelevant importer refreshes.

Raw graph caches may still be re-derived and remain device-local. A canonical
event records which current observations and authored evidence snapshots
support it and rejects contradictory chain/network or native identities.

### One arbitration layer

Every existing interpreter produces candidate quantity claims instead of
directly rewriting RP2-bound rows. This includes exact same-txid transfers,
owned-output derivation, fan-out/consolidation, manual pairs, direct payouts,
cross-asset routes, channel lifecycle, swaps/refunds, and active custody
components.

The arbitrator applies an explicit evidence order:

1. active reviewed custody component/bridge;
2. exact native event evidence;
3. verified policy recognition valid for the event;
4. reviewed pair or payout;
5. declared deterministic accounting convention;
6. heuristic candidate, which cannot book automatically; and
7. custody suspense.

It enforces:

- each source quantity slice is claimed exactly once;
- each destination quantity slice is credited at most once;
- every fee/loss is allocated once;
- overlapping active claims become `conflicting`;
- partial claims leave an explicit residual rather than consuming it;
- an active component atomically replaces all covered anchors; and
- a failed or incomplete claim becomes suspense/withholding, never fallback
  taxation.

No downstream phase may restore, revive, or reinterpret a quantity already
decided by the arbitrator.

### Quantity projection

The quantity projection reports what is physically observed and where custody
is known or uncertain. A blocked tax classification does not undo a known
source-wallet debit. For example:

```text
10.0 BTC left Multisig B
  9.9 BTC -> reviewed retained custody in Operative C
  0.1 BTC -> custody suspense
```

Multisig B decreases by 10 BTC; Operative C receives 9.9 BTC with preserved
basis lineage; 0.1 BTC remains explicit suspense. Operative C can spend its
observed 9.9 BTC, but any tax result whose lot selection could change when the
earlier residual is classified remains provisional and report-blocked.

### Economic and tax projection

Custody facts do not encode country policy. Economic classification decides
transfer, sale, purchase, swap, gift, loss, fee, income, or unresolved status.
The tax-input builder accepts only finalized classifications and converts them
to RP2 primitives. RP2 continues to own lot math and jurisdiction-specific tax
computation. A quantity projection can therefore be final while a later tax
result is still provisional: classifying an earlier suspense slice can change
profile-wide FIFO/LIFO/HIFO/LOFO or moving-average state. Kassiber blocks the
affected tax output rather than inventing a country-neutral basis reservation.

`external_presumed` is a deliberately visible usability default, not ownership
proof. A later candidate invalidates the affected derived tax input, marks
journals stale, and produces a filed-report change warning when applicable.
Reviewed `external_confirmed` facts remain authoritative until explicitly
superseded.

## Wallet And Privacy-Round Handling

### Complete policy history

When Kassiber has the relevant Deposit, Premix, Postmix, Badbank, and later
wallet policies, it automatically recognizes the profile's inputs and outputs:

- Tx0 is an atomic transformation of recognized Deposit inputs into recognized
  Premix/Badbank outputs plus attributable miner/coordinator fees;
- mix/remix rounds carry aggregate profile-owned quantity across recognized
  outputs without claiming an unobservable input-to-output sat mapping; and
- mix-out into another imported profile wallet is an automatic internal move.

Equal-denomination outputs belonging to other CoinJoin participants are never
claimed merely because their amounts match. Deterministic lot allocation among
the profile's postmix outputs is labeled an accounting convention, not chain
evidence.

### Missing Whirlpool history

Kassiber does not require reconstruction of every hidden round. It compares
known boundaries across the complete imported history:

```text
10.0 BTC leaves Multisig B
9.9 BTC arrives one year later across Operative C receipts
0.1 BTC remains unexplained
```

The deterministic engine should suggest a bounded N:M custody bridge even
across a year or more. Time reduces confidence but is never a hard rejection.
The retained amount cannot exceed selected source quantity. A return above the
source amount splits the excess into a separate acquisition/income candidate.
The unexplained residual is never silently called a fee merely because the
arithmetic balances.

One approval creates a durable `internal_reviewed` bridge for 9.9 BTC and a
separate 0.1 BTC suspense/classification task. The 9.9 BTC quantity is available
in Operative C immediately, while affected later tax results remain provisional
until the earlier 0.1 BTC is classified. Recovering old policies later
revalidates the bridge against native evidence: consistent evidence upgrades
its support, partial evidence is recorded, and contradiction blocks review
without silently rewriting the approved claim.

## Long-Horizon Candidate Generation

This is a separate engine from the short-window swap matcher. It searches the
full imported history and supports bounded 1:N, N:1, and N:M grouping without
an unbounded O(n^2) scan.

Candidate generation is deterministic and reproducible. Independent score
dimensions include:

- returned/source quantity coverage;
- absolute and percentage residual materiality;
- time distance, used as a score rather than a cutoff;
- old-wallet retirement and new-wallet activation chronology;
- direct ancestry or graph completeness;
- Samourai/Whirlpool tags, policy roles, and denomination patterns;
- already claimed source/destination slices;
- unrelated commercial/acquisition evidence; and
- competing candidate count and score margin.

The generator uses indexed quantity/time buckets, bounded group sizes, bounded
beam/candidate counts, pagination, and deterministic tie-breaks. It emits
reason codes, alternatives, residuals, and downstream impact. It never creates
an authored bridge.

Candidate promotion uses a versioned deterministic threshold. Amount and time
similarity alone are insufficient: a blocking `custody_candidate` also needs a
meaningful chronology/privacy/topology signal and a clear margin over competing
groups. Lower-scoring relationships may appear as non-blocking search hints
while the transaction remains `external_presumed`. Conversely, the flagship
10 BTC out / 9.9 BTC return after one year must promote when the old/new wallet
chronology and Whirlpool evidence align and no comparable competitor exists.
This distinction prevents a large book full of ordinary receipts from becoming
permanently unreportable because of weak numerical coincidences.

A dismissed suggestion remains auditable. It reopens only when a material
evidence-version change improves or contradicts it.

## Desktop And CLI Workflow

### Custody gaps surface

Add a first-class **Custody gaps** surface rather than requiring normal users to
work in Swap Matching or edit component JSON. It contains:

- **Needs review**, **Resolved bridges**, and **Dismissed** queues;
- a wallet-lineage timeline showing active/retired policies and missing
  intervals;
- reason codes, competing candidates, quantity coverage, time span, and
  evidence grade;
- an explicit **Why Kassiber suggested this** explanation;
- a guided N:M bridge wizard;
- separate residual classification;
- before/after quantity, basis, tax, and affected-year preview;
- revision, supersession, dismissal, and evidence-driven reopening; and
- the qualified empty state **No known custody gaps**.

The bridge preview states concrete effects, for example: presumed disposals and
acquisitions removed, quantity/basis carried, later disposals unblocked,
residual left in suspense, and filed periods changed. Activation requires
explicit consent after this server-validated preview. Low-level custody
component JSON remains an expert/debug view.

The CLI exposes the same deterministic scan, list, preview, create/revise,
dismiss, and history operations with machine envelopes and dry-run support.

### Filed-report changes

If new evidence changes a classification used in a saved/filed report, retain
the original snapshot and show affected periods, before/after classification
and gains, and an amendment warning. A generic stale-journals badge is
insufficient.

## AI Tooling And Privacy

Deterministic code finds and validates candidates. AI is optional assistance
for grouping, explaining alternatives, proposing residual questions, and
drafting a bridge. Model output is never evidence and never activates a write.

Read-only AI kinds:

```text
ui.custody.gaps.list
ui.custody.gaps.review_context
ui.custody.lineage.snapshot
ui.custody.coverage.snapshot
ui.custody.bridge.preview
ui.custody.bridge.impact
```

Consent-gated kinds:

```text
ui.custody.gaps.scan
ui.custody.bridge.create
ui.custody.bridge.revise
ui.custody.bridge.supersede
ui.custody.gap.dismiss
```

New desktop-invoked kinds follow the Python, Tauri, and Vite allowlist lockstep
documented in `AGENTS.md`. AI tools remain separately capability-scoped and
schema-validated.

AI-safe candidate packets include bounded source/return summaries, exact
decimal-string quantities, time span, chronology flags, reason codes,
competing-candidate summaries, residual, and downstream impact. They exclude
descriptors, xpubs, addresses, scripts, outpoints, participant graphs, backend
URLs, tokens, and raw wallet files.

Detailed cross-wallet linkage analysis is sensitive because it reconstructs
relationships privacy tools intentionally obscure publicly. The deterministic
scan runs locally. A local model may receive richer local review context. A
remote provider receives only a redacted packet under the existing provider
privacy/receipt controls; it never receives raw mix graphs or wallet policy
material. The feature works fully without AI.

Store the deterministic candidate version, facts made available to AI,
provider kind, generated draft, user edits, and final consent in the local
audit trail. AI chat history remains separate and is not evidence.

## Persistence, Migration, And Replication

Schema evolution is additive and preserves authored history:

- immutable evidence snapshots referenced by authored claims plus canonical
  event references;
- policy epochs and evidence provenance;
- versioned custody claims/bridges and per-quantity allocations;
- derived candidate, quantity-projection, and tax-input rows; and
- dismissal/reopening and filed-report impact history.

Preserve original transaction ids, metadata history, attachments, pairs,
direct payouts, custody-component lineage/revisions, and replication identity.
Existing authored pairs, payouts, and components become compatibility inputs to
the single arbitrator; migrate them to equivalent revisions where lossless.
Derived journals, candidates, and reports rebuild locally.

Authored bridges, revisions, dismissals, and their evidence references join the
positive replication allowlist. Derived candidates/projections do not sync.
Concurrent active bridges or revisions remain visible and ineffective as
`conflicting` until reviewed. Out-of-order replay cannot activate a bridge
before its transaction, wallet, observation, and revision dependencies exist.

Migration emits a bounded before/after report. Fully resolved books preserve
expected tax output. Known broken/ambiguous histories may intentionally change
from disposal/acquisition to retained custody or suspense; every changed event
and affected report period is listed.

## Test Strategy

### Flagship OG treasury fixture

```text
2015 acquisition -> Multisig A
2018 A -> Multisig B wallet roll
2020 B -> Samourai Deposit -> Tx0/Premix/Postmix
2022 Postmix -> Operative C
2023 C -> real external vendor payment
2024 C -> Multisig D wallet roll
```

Required variants:

1. **Complete policies:** all internal transitions are automatic; only actual
   fees leave custody before the vendor payment; C and D receive correct basis;
   no review is required.
2. **Missing Whirlpool:** 10 BTC leaves B, 9.9 BTC returns to C across multiple
   receipts one year later, one reviewed bridge carries 9.9 BTC quantity, and
   0.1 BTC remains suspense. C's later disposal preview uses the reviewed
   bridge, but its tax result remains blocked when resolving the earlier 0.1
   BTC could change global lot selection.
3. **Recovered policy:** later imported Whirlpool evidence validates or blocks
   the reviewed bridge without rewriting it.
4. **False friend:** unrelated 9.9 BTC revenue and competing returns do not
   auto-bridge and are shown as alternatives.

### Executable invariants

- source quantity equals internal destinations plus external, fee/loss, and
  suspense quantities;
- a quantity slice has at most one effective active interpretation;
- candidate, suspense, or conflict quantities never become finalized taxable
  RP2 events;
- suspense reduces the observed source-wallet quantity;
- a partial reviewed bridge carries only its finalized retained quantity;
- reprocessing is deterministic, idempotent, and import-order invariant;
- evidence-version changes invalidate or revalidate dependent derived claims;
- no chain/network or Bitcoin-exposure boundary is crossed implicitly; and
- a failed active component cannot fall back to a sale or acquisition.

### Coverage matrix

Tests cover BIP44/49/84/86 receive/change policies, multipath descriptors,
multiple accounts, multisig/miniscript rotation, retired policies, finite
address lists, multiple script types per xpub, high issued indexes, wrong
birthday/scan horizon, duplicate wallet observations, and recovered history.

On-chain cases cover ordinary payments, pure self-transfer, unindexed change,
all outputs unrecognized, payment plus owned change, 1:N/N:1/N:M, batching,
PayJoin, CoinJoin, multi-wallet inputs, OP_RETURN, fee conventions, RBF, CPFP,
reorg, graphless imports, and public graph backfill.

Rail cases cover Liquid value/blinding gaps, BTC/L-BTC swaps, Boltz claims and
refunds, Lightning opens/closes/force-close sweeps, dual funding, missing node
history, and duplicate adapter observations. Future Ark/Bark adapters are not
implemented here; an adapter contract test proves a new Bitcoin layer can emit
native identity, quantity/exposure, parent/spend relations, custody state,
finality/exit state, fees, and evidence provenance without tax-module code.

Migration fixtures span databases before and after the transfer overhaul,
active and conflicting components, manual pairs, direct payouts, exclusions,
replication replays, and filed reports. Differential tests require old/new
equality for known-correct books and a named explanation for every deliberate
change. Property tests generate bounded graphs for conservation, exclusivity,
permutation invariance, supersession monotonicity, and no unresolved-to-tax
path. Performance tests enforce bounded full-history candidate generation.

Integration lanes include fast replay, Bitcoin Core, Electrum parity, Silent
Payments, Liquid/Boltz, CLN/LND, migration followed by journal rebuild, and
desktop bridge preview/activation without raw JSON editing.

## Implementation Gates

### Gate 0: Honest pre-split behavior

- preserve the conservative disposal plus hard quarantine before quantity and
  tax projection are separated;
- prove observed source debit, exact fee separation, and no later reuse of the
  spent source quantity; and
- treat any attempt to implement suspense by filtering RP2-bound rows as a
  stop event.

Suspense is not implementable correctly before Gate 1. Gate 2 or Gate 3 work
must not begin merely because their local APIs are easier to extend.

### Gate 1: Evidence and projection boundary

- add stable quantity identities, immutable authored evidence snapshots, and
  canonical physical events;
- define custody claim/projection types and the single arbitrator;
- split quantity projection from finalized tax-event projection;
- add custody suspense and the full-engine no-suspense-to-RP2 regression; and
- pass the flagship core and property tests.

Stop for an architecture audit before porting every interpretation path.

### Gate 2: Interpretation parity

- port exact transfers, ownership, fan-out/consolidation, manual pairs, direct
  payouts, swaps/refunds, channel lifecycle, and custody components;
- preserve known-correct differential outputs;
- implement long-horizon candidate generation and reviewed N:M bridges; and
- add an activatable, explicitly allocated `suspense` custody-component sink
  while preserving `unresolved` as a non-activatable incomplete-evidence role;
- block method-dependent later tax results until earlier suspense is
  classified; and
- remove withholding/restoration/fallback paths once parity tests pass.

### Gate 3: Product completion

- add CLI, daemon, desktop, localized UI, AI-safe tools, and privacy receipts;
- add migration, replication, filed-report impact, and audit history;
- pass the complete descriptor and missing-Whirlpool acceptance variants; and
- pass all integration lanes and the repository quality gate.

### Gate 4: Simplicity and final review

- delete superseded match/ownership arbitration and compatibility code that is
  no longer exercised;
- verify there is one owner of each invariant and no duplicate projection
  implementation in CLI, daemon, UI, or RP2;
- run one independent architecture/security/privacy review and one final
  merge-readiness review; and
- defer unrelated P2+ findings to `TODO.md` instead of expanding this work.

## Non-Goals

- replacing RP2 or changing Austrian lot/tax algorithms;
- a double-entry/general-ledger redesign;
- global proof that every wallet is imported;
- automatic activation of missing-wallet bridges;
- reconstructing unobservable CoinJoin input-to-output sat paths;
- fractional ownership, shareholder interests, or cross-profile co-ownership;
- new wallet/backend features unrelated to custody lineage;
- implementing Ark/Bark or other future-rail adapters in this work; or
- e-cash, issuer liabilities, or bearer-proof accounting.

## Terminal Stop State

This work is complete, merge-ready, and must stop when all of the following are
true:

- every observed quantity is represented exactly once;
- RP2 receives only finalized tax events;
- candidate, suspense, conflict, or failed-component quantities cannot become a
  fallback sale or acquisition;
- suspense affects observed quantity without fabricating tax;
- complete imported policy histories reconcile automatically;
- one reviewed N:M bridge preserves basis lineage durably across missing
  history;
- residuals remain separately reviewable and do not poison resolved principal;
- custody quantity finality and tax-basis finality are shown separately, with
  method-dependent later tax output blocked while earlier suspense can alter
  global lot selection;
- no global wallet-completeness claim exists;
- known-correct books retain expected results and every intentional migration
  difference is reported;
- existing authored history and replication semantics survive migration;
- desktop and CLI workflows do not require low-level JSON;
- AI remains optional, redacted, consent-gated, and non-evidentiary;
- legacy competing arbitration/restoration/fallback paths are removed;
- the flagship fixtures, invariant/property/migration/integration suites, and
  repository quality gate pass; and
- no issue-scoped P0/P1 remains.

At that point unrelated polish and future-layer implementations are filed as
separate backlog items. They do not extend this project.
