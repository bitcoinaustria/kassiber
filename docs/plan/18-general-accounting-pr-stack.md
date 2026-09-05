# General accounting: live dependency triage and two-PR stack

Updated: 2026-09-05. This supersedes the earlier planned seven-cut breakdown.
The accepted full scope remains [spec 17](17-general-accounting-and-private-ai-spec.md),
but publication is not merge approval or completed organizational acceptance.
EBICS, payment initiation, payroll and FinanzOnline transmission remain excluded.

## Live triage

| PR | Decision |
| --- | --- |
| [RP2 #48](https://github.com/bitcoinaustria/rp2/pull/48) | Merged as `7b0dd6771c611e83451cd7f97782af0c15382197` after correcting acquisition versus cutoff basis and unsafe per-wallet replay. All 35 hosted checks passed. |
| [Kassiber #542](https://github.com/bitcoinaustria/kassiber/pull/542) | Merged as `384c5f785dd68c36ea7e7be0ad3fe1ef059053d2` after 25 successful exact-head checks and one intentional release-publication skip. Pins merged RP2, adapts holdings and invalidates old pool journals once. |
| [Kassiber #543](https://github.com/bitcoinaustria/kassiber/pull/543) | Merged at `bbd983cb824de4ea51c685248b0d3fba5347d8d2` after fixing CLI consent at `6fc7c4e8` and passing full local/CI gates. Independent of #542. |
| [Kassiber #455](https://github.com/bitcoinaustria/kassiber/pull/455) | Closed as superseded by #543's guided N:M, conversion, suspense and revision flow. No unique must-salvage feature found; branch retained. |
| [Kassiber #428](https://github.com/bitcoinaustria/kassiber/pull/428) | Separate altcoin overview/import experiment, deferred. Its cash-symbol validation was extracted into independent [#544](https://github.com/bitcoinaustria/kassiber/pull/544), not used to justify merging the overlay. |
| [Kassiber #544](https://github.com/bitcoinaustria/kassiber/pull/544) | Narrow cash-currency validation salvage merged at `f43d27f88bb9fd3f7c3acd141b6ae3145f43c029` after exact-head CI passed. Not an accounting-stack dependency. |
| [Kassiber #431](https://github.com/bitcoinaustria/kassiber/pull/431) | Separate native macOS client; defer outside CLI/Agent accounting. |
| [Kassiber #136](https://github.com/bitcoinaustria/kassiber/pull/136) | Separate modular overview layout; defer outside this stack. |
| [Kassiber #547](https://github.com/bitcoinaustria/kassiber/pull/547) | Separate release/signing lane on main, not an accounting dependency. Bounded Standards/Spec reviews found no code blocker at `70309f7`; actual notary credentials, OpenPGP enrollment, production protections and clean-Mac/Touch ID activation remain unverified. |
| [Kassiber #548](https://github.com/bitcoinaustria/kassiber/pull/548) | Independent dependency security patch merged as `115a5186a6ea6ea888b9bb87562c547288fc54cb` after the full local gate, independent Standards/Spec review and 25 successful hosted checks. Keeps frontend release quarantine; remaining `fast-uri`/`qs` patches await explicit exception or maturity. Restacked accounting inherits this patch. |

The original reproductions are retained on
[RP2 #48](https://github.com/bitcoinaustria/rp2/pull/48#issuecomment-5552699773)
and [Kassiber #542](https://github.com/bitcoinaustria/kassiber/pull/542#issuecomment-5552699913).
The corrective review and verification are recorded on
[RP2 #48](https://github.com/bitcoinaustria/rp2/pull/48#issuecomment-5553903308).
The historical blocker is resolved by the merged dependency, not by dropping
pool support. #545 also adopts the distinct report basis in retained captures,
retains the real execution sequence once across RP2's report replay, and excludes
explicit future receipt references from cutoff inventory using RP2 itself.
Adapter version `cutoff-prefix-v2` rejects stale calculation reuse without
rewriting immutable history. This does not establish organizational acceptance.

## Actual accounting cuts

| Order | Branch | Responsibility |
| --- | --- | --- |
| [#545](https://github.com/bitcoinaustria/kassiber/pull/545) | `codex/accounting-core-cli` | Complete deterministic local accounting domain and CLI: encrypted separate ledger/evidence, bank/open items/schedules/cash, retained RP2 sources/calculations, projections/openings/valuations, statements/close, jurisdiction workpapers, durable local tasks, recovery and internal daemon/document-worker contracts. No accounting AI tools, provider changes or new UI. |
| [#546](https://github.com/bitcoinaustria/kassiber/pull/546) | `codex/accounting-scoped-agent` | Exact selected-disclosure AI, guarded draft proposals, opaque scoped task tools, CLI assistance, minimal existing-Assistant approval, provider isolation/cancellation and native security protections. No dedicated accounting screens or broad renderer accounting allowlist. |

These are two cohesive extracted branches, not seven artificial layers whose
eager imports or schemas reference absent modules. The coupled financial
domain remains together. Each cut must pass independently; the full stack must
also preserve the previously tested composition.

PR #543 landed independently. #542 is merged after the dependency correction;
the current stack is `main -> #545 -> #546`.
Both accounting PRs remain draft while full outcome acceptance is unresolved.
No auto-merge is armed around those acceptance gates.
If any base changes, restack and rerun scope/basis/consent integration tests.

## Preserved checkpoints

- Original common base: `ec31078cd2ed61b09afbc3007044f49c477b6fee`.
- Original #542 head: `a6fa122d8be79a62dc135b6863e9cec1f781ea4f`.
- Original #543 head: `d7d505fb3c45f20222a8b6e19ca737c0782a2333`.
- Original combined dependency checkpoint: `5371c851465736068590aea77d4d6c50a38018c3`.
- Preserved pre-split accounting: `codex/accounting-presplit-20260905` at `21552bca`.
- Full UI recovery: `codex/accounting-ui-preserved-20260905` at `fbfce410`.
- Frozen pre-triage CLI/Agent integration: `61c20f07`.
- First core cut: `7ce7f9b319b7ba5e2efcadc8f46789ee987ec903`.

The original preview checkout and its live book are untouched. The old combined
dependency branch is a local recovery checkpoint, not a replacement PR.

## Security and reuse seams

#543's canonical review plan/apply/receipt owns custody interpretation, price
overrides and exclusion. It does not authorize GL posting, close or tax
finalization. Every financial task transition retains its own scope/revision/
payload checks and fresh approval. CLI blanket or session approval must not
silently grant either custody review or accounting task application.

General chat does not consume selected-financial-disclosure grants: those stay
tool-free, no-history, one-use and provider/book/revision-bound. Ordinary agent
tools return opaque task state, not the local financial approval payload.
Accounting evidence stays inside SQLCipher; do not copy it into ordinary
plaintext attachments or disclose it implicitly to remote providers.

The existing native export and book-change protections remain. Vite/native
renderer allowlists stay equal, with no direct accounting operation exposure;
the removed accounting visibility setting is not re-advertised. Private users
remain unenrolled and need no general-accounting configuration.

## Verification and unresolved gates

The follow-up combined candidate is `c6ce832c` over core `0568757e`, including
merged #548. It adds exact local task amendments/Bitcoin preparation, durable
agent mutation receipts, explicit local export delivery and matching minimal
CLI/Assistant consent. A bounded independent merge review is clean. A previously
observed intermittent `accounting_requires_encryption` in the actual mixed-source
agent fixture is still unresolved; 640 instrumented green repeats do not clear
that hold. The full final gates passed: core has 4,322 Python tests / 425 subtests
and 1,004 UI tests; the agent implementation has 4,539 Python tests / 432
subtests and 1,026 UI tests. Both pass TypeScript and ESLint with zero errors;
39 optional Python skips and 50 existing lint warnings remain on each cut.
Fresh exact-source agent CLI and sidecar builds each pass three real packaged
smoke tests. Provenance and limits are in the acceptance record; the older
results below are historical checkpoints, not these final runs.

After the RP2 correction, clean full gates pass on core `0d80356d` (4,275
Python passes, 38 optional skips, 423 subtests, 134 UI files / 1,004 tests)
and scoped-agent `ddd208ae` (4,418 Python passes, 38 optional skips, 430
subtests, 137 UI files / 1,021 tests). Both include TypeScript and ESLint
with zero errors and 50 existing warnings. Modern Bash was used for the
unchanged Linux publisher tests. These runs include merged #544 and the
`a17ffae5` cutoff/capture correction, not the obsolete failing adapter.

The core extraction passed 459 focused tests (3 optional skips). Independent
partition review verified all 100 accounting action classifications, zero
accounting AI exposure, retention and worker lifecycle; 156 tests plus a real
locked-daemon scenario passed. These are partition checks, not exhaustive
accounting certification.

The pre-triage full integration gate completed with 4347 passes, 38 skips and 430
subtests, but two failures exposed stale Vite-only accounting allowlist entries.
The extracted agent cut omits those entries. Its full gate passed at `b9b31474`:
4365 Python tests, 38 skips, 430 subtests; TypeScript; ESLint with zero errors
and 50 existing warnings; 137 UI files / 1021 tests. The native suite passed
115 tests. Restack `02dfd273` has the identical tree; another 140 CLI/consent/
scope/catalog tests passed after resolving the overlapping consent hunks.
The external Claude review remains blocked by code-egress approval and is not
represented as completed.

Record exact final gate results and published PR links in the
[acceptance record](../reference/general-accounting-acceptance.md). The real
organization's confirmed facts, mixed-source100-record user-effort benchmark,
AF-1 through AF-5, current packaged runtime and full organizational acceptance
remain open. A synthetic K2 export is not a completed real filing.
