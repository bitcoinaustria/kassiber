# Privacy Mirror

Privacy Mirror is Kassiber's local privacy-analysis surface. It answers four
questions from the same reduced facts in the desktop GUI, CLI, and assistant:

- what is linkable
- who can plausibly infer it
- what local evidence supports that result
- what is unknown or would worsen in a future spend

It is advisory-only. It never signs, broadcasts, syncs wallets, fetches chain
data, refreshes tax journals, selects coins, or mutates accounting data.

## Surfaces

- Desktop: the dedicated Privacy Mirror page plus wallet-detail and
  transaction-detail panels.
- CLI: `kassiber reports privacy-mirror`.
- Daemon: `ui.reports.privacy_mirror`.
- Assistant tool: `ui_reports_privacy_mirror`, a read-only AI tool over the
  AI/export-redacted payload.

PSBT preflight analysis is available locally through the desktop PSBT panel and
CLI `kassiber reports psbt-privacy`. The AI tool does not receive raw PSBT
contents. Assistant answers may refer only to the redacted findings already in
the Privacy Mirror payload unless a future tool explicitly reduces a PSBT to
redacted findings first.

## Methodology

Privacy Mirror combines two existing local models:

- the watch-only linkage graph from local transaction and UTXO inventory
- the privacy-hygiene posture snapshot for backend, AI-provider, journal, and
  coverage facts

The linkage graph contributes cluster counts, adversary views, wallet rows,
transaction tells, UTXO rows, timeline events, evidence drilldowns, and
coverage gaps. The hygiene snapshot contributes local configuration posture,
privacy quarantines, off-device AI/backend counts, and limitations. The report
selects the worst current risk by severity first and then by available evidence,
so it can answer "what should I look at first?" without producing a standing
green reassurance state.

PSBT preflight uses the same local graph to score unsigned transaction inputs
and outputs: cluster-merge delta, per-adversary delta, blast-radius score,
change/fingerprint tells, and unknown inputs. What-if rows are bounded
simulations: receive reuse versus fresh receive and hypothetical consolidation.
They do not recommend which coins to spend.

## Evidence Levels

Every finding, row, unknown, and summary carries `evidence_level`:

- `exact`: directly counted from local stored rows, such as current UTXO rows or
  a known same-cluster spend.
- `derived`: inferred from deterministic local rules, such as common-input
  linkage or adversary summaries over the reduced graph.
- `unknown`: the local model cannot prove the claim because an input, source,
  branch role, graph edge, or coverage area is missing.

Assistant and CLI output keep these English values deterministic. The desktop UI
translates their labels in English and German.

## Degraded States

Unknown or degraded rows are first-class output. Common causes include:

- wallet sources without watch-only UTXO inventory
- imports that lack vin/vout detail
- unknown PSBT inputs
- stale or missing local sync coverage
- unsupported Liquid unblinding or source-proximity data
- privacy quarantines that need review

The UI should show the degraded state near the affected wallet, transaction,
UTXO, timeline row, or PSBT result. It should not hide uncertainty behind a
general "all clear" badge.

## Redaction

`ui.reports.privacy_mirror`, `kassiber reports privacy-mirror`, and
`ui_reports_privacy_mirror` are AI/export-safe by construction. They omit:

- addresses
- scripts and scriptPubKeys
- descriptors and xpubs
- backend URLs, tokens, auth headers, and cookies
- wallet config JSON and wallet files
- raw importer JSON and raw transaction JSON
- branch labels, branch/index values, and derivation paths

The local desktop GUI may still use existing first-party permissions elsewhere,
for example reveal flows that require local user acknowledgement or backend
settings screens that show operator-facing endpoint rows. Those local UI
permissions are separate from the Privacy Mirror payload and are never what the
assistant receives.

## Non-Goals

Privacy Mirror is not:

- coin selection advice
- a signing or broadcasting path
- a tax/accounting mutation
- an external lookup service
- a privacy guarantee
- a replacement for reviewing raw wallet software before spending

It is a local mirror over the evidence Kassiber already has.
