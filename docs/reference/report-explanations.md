# Capital-gains explanations

The capital-gains report's **Explain** action shows the exact RP2 gain fragments
that contributed to one persisted result. Each fragment includes proceeds,
consumed basis, gain, quantity, source pricing provenance and whole-source fees.
Amounts are decimal strings, never recalculated from rounded display values.
Source fees describe the complete source event; they must not be summed once per
fragment. The contribution proceeds are RP2's fee-inclusive fragment result.

RP2 selects lots and computes any pool unit-basis override. The explanation
retains that output in `journal_entries.calculation_json` while the canonical
journal is built. It does not replay the engine on read. In pooled methods the
selected acquisition is a matching anchor, not a claim that its original price
is the pool's consumed basis. Acquisition basis and the unit override remain
separate. Custody decisions are the existing semantic lineage projection;
physical wallet outputs are not asserted to correspond to tax lots.

Each row carries an explanation reference binding the database instance,
workspace, profile, journal input version, processed timestamp and journal entry.
The desktop offers links to the calculation and individual frozen source nodes.
These links open the explanation directly, without the report screen's automatic
journal refresh. A different book, changed input or rebuilt journal fails closed;
links do not silently follow a new calculation. Source nodes display the retained
transaction evidence for that calculation version. Custody links are bounded to
500 inspected decisions, with truncation explicit. Context includes incoming
canonical eligible custody moves back through the result wallet history to the
earliest retained engine source. Later moves and unrelated wallet branches are
excluded. These links describe custody context, not physical ownership of tax lots.

CLI: use `kassiber --machine reports capital-gains`, then pass a row's exact
`explanation_reference` JSON to
`kassiber --machine reports explain-capital-gain --reference '<JSON>'`.
The desktop daemon kind is `ui.reports.explain_capital_gain`, taking
`{"reference": {...}}`. It is not an AI tool, performs no automatic maintenance
or external requests, and reads within one SQLite snapshot. The existing AI
capital-gains summary omits these local references.

Legacy journals report `engine_detail_unavailable` until explicitly rebuilt.
Mismatched contributions report `engine_detail_mismatch`. Missing/stale journals
and custody blockers are errors; existing quarantines remain explicit in the
response, including price gaps. An explanation reconciles its displayed row,
not transactions excluded by quarantine. This first path does not explain every
aggregate report cell or produce financial AI narration.

For synthetic carrying-value acquisitions, the explanation follows eligible
reviewed relations from `journal_custody_projection_relations`, including
cross-rail economic conversions. Each acquisition's `inherited_basis` includes
those relations and the outgoing disposal's reconciled, retained RP2 calculation,
recursively following further synthetic acquisitions. This exposes the original
acquisition price and whole-source fees even when the later sale names only a
synthetic rail-entry lot. These are complete historical source calculations;
Kassiber does not allocate their individual lots or fees to the later sale.

Inherited detail can be unavailable or fail reconciliation independently of the
displayed sale. It is never substituted with a same-asset pool or inferred from a
zero gain. Reads share the result's pinned journal snapshot and stop at eight
carry levels or 100 relation/calculation records, with explicit truncation. No
additional calculation or source data is persisted by the explanation read.
