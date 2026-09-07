# Privacy Mirror

Privacy Mirror summarizes the locally observable exposure of the active profile.
[Chain Analysis](local-chain-analysis.md) is the investigation workbench. They use
one immutable observation index and one implementation of structural rules,
clustering, multi-hop patterns, public attribution and conditional entropy.
Mirror does not maintain its own graph interpreter, weighted score or detector
catalog. Settings → Privacy retains the separate configuration-posture report.

## Surfaces and boundaries

- Desktop: Privacy Mirror (Extras, no developer gate) opens with one headline
  without counts. Any finding whose relevance is your spend or your output wins
  (severity plays no role); otherwise the backend summary decides between no
  local transactions, an unavailable assessment, no owned outputs, surrounding
  activity only, or no findings within the examined evidence. Surrounding
  findings never upgrade an unassessed book. Personal findings stay primary;
  surrounding activity and the executed checks are collapsed disclosures, while
  coverage status, gaps and truncation remain visible in the headline card.
  Findings open Chain Analysis with the public observer, physical subject and
  chain/network preserved and land on the matching subview. Transaction details
  use the same focused workbench link rather than loading the entire profile's
  Mirror report.
- CLI: `kassiber reports privacy-mirror` returns the same report through the
  shared report service. JSON is schema version 2 inside the existing envelope;
  table/CSV output lists findings and executed-check coverage.
- Assistant: `ui_reports_privacy_mirror` is a read-only local tool. The general
  page prompt reads this safe projection; finding-specific prompts first use
  `ui.chain_analysis.ai_context` to bind opaque references to the displayed
  snapshot. A changed snapshot requires a fresh report.
- PSBT v0/v2 preflight, Payjoin comparison, local dataset management, graph
  exploration and detailed partition scenarios live in Chain Analysis. The
  CLI `reports psbt-privacy` retains its reduced local-inventory what-if adapter
  over the shared parser, features and observer components.

These reads do not sync, discover endpoints, contact nodes, fetch missing
history, select coins, sign, broadcast, rebuild journals or change accounting.
Network acquisition stays an explicit, consented workbench operation. Selecting
an existing connection is not permission to contact it implicitly.

## Evidence and relevance

`core.privacy_mirror` projects `chain_analysis.analyze_snapshot` using the public
observer. Observer visibility is applied before traversal and analytics. Private
wallet ownership only selects relevance: **own spend**, **owned output**,
**received context**, or **nearby context**. Private branch/change metadata,
source-funds narratives and private labels do not become public links.

Mirror groups canonical findings and hypothesis edges by their transaction
anchor. Each result keeps its rule code, evidence authority, assumptions,
limitations, canonical source references and snapshot-bound investigation query.
Counterparty structure is contextual information, not a warning against the
receiving wallet. Own-spend common-input/change hypotheses and script reuse can
need attention, but they remain conditional. Physical spends establish
connectivity, not common ownership, input-to-output allocation or taint.

CoinJoin/Payjoin evidence suppresses common-input and change ownership
assumptions. An imported private collaboration marker is an observer exclusion,
not proof that a passive observer can identify the protocol. Public structural
candidates remain hypotheses. Missing, stale and conflicting observations are
coverage limitations rather than current actionable findings. Duplicate wallet
observations do not multiply physical transactions or linkage findings.

Public attribution uses only locally imported public dataset claims. A sourced
label or path to one is inspectable context, not verified identity, payment or
misconduct. No labels available locally means attribution is unavailable; it
never means an outside analyst has no labels.

## Coverage and computation limits

The report compares examined transactions with locally available transactions,
not the whole blockchain. It lists missing, stale and conflicting nodes, actual
rule-family coverage and stopping reasons. It returns at most 100 findings from
an overview bounded to 2,000 nodes, 6,000 edges and depth 12; counts and omitted
work stay visible. Empty or unowned evidence is unavailable. A report without
priority findings is not a privacy guarantee.

Conditional partition counts use the shared solver on at most 12 selected
transactions, prioritizing own spends. Computation is capped at 5,000 states and
25 ms per transaction, and 300 ms overall. Exact results, proved lower bounds,
unsupported models and omissions remain distinct. Liquid confidential values,
unknown amounts and joint-payment assumptions cannot be filled in by a score.
These counts describe an explicit model, never ownership probabilities. The
workbench can inspect the same transaction with a larger explicit budget and
participant-fee scenarios.

## Audience projection

The desktop daemon receives local physical references for navigation. CLI
exports and AI dispatch instead receive book/process-scoped `ca-ref:` handles
through the existing Chain Analysis provider projection. They preserve rule
codes, counts and limitations while excluding raw chain identities and dataset
prose. Descriptors, xpubs, raw wallet configuration, endpoints, credentials,
derivation paths and raw PSBT bytes are not part of this report.

Legacy private provenance and reviewed source-funds projections remain separate
from this public-observer report. They do not grant public attribution or
accounting authority. See [source-of-funds review](source-of-funds-review.md).
