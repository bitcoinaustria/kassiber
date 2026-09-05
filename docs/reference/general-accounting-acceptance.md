# Organizational accounting acceptance record

Status: local CLI/Agent-only delivery; full release acceptance still pending
(2026-09-05). The full UI prototype is preserved, not part of this delivery.
This is a technical coverage record, not a statement of the organization's
actual legal obligations or complete source population. The scope is defined in
[plan 17](../plan/17-general-accounting-and-private-ai-spec.md); the executable
contracts are documented in [general accounting](general-accounting.md).

## Pilot facts still required

The real organization has not yet supplied a confirmed fiscal/assessment year,
legal/tax route (including whether K2 applies), accounting regime, VAT/withholding
facts, full transaction categories, bank export sample, first opening date,
opening balances/open items, prior assessments or carryforwards. EUR/calendar
year/Vienna and 2025 fixture values are synthetic test choices, not inferred
facts. Each actual category must be mapped to a capture/posting/tax/close fixture
before calling the organization's workflow complete.

## Implemented technical lanes

| Lane | Record/authority and workflow | Verification anchors |
| --- | --- | --- |
| Existing private users | No GL enrollment, encryption migration, chart or K2 setup required | `test_accounting_integration`, existing full Python/UI regression and fast regtest |
| CLI/Agent-only delivery | No new accounting route, Settings switch or forms; explicit encrypted-book enrollment; exact local consent in CLI/existing Assistant | `test_accounting_capabilities`, `test_cli_accounting_consent`, Assistant consent tests; recovery branch retains the former UI |
| Durable scoped tasks | Explicit source selection, proposal rules, evidence assignments, atomic prepare/post, independent close/tax/export receipts, restart and idempotent retry | `test_accounting_tasks`, `test_accounting_agent_tasks`, real fresh-process `test_accounting_cli_tasks` |
| Exact double entry | Draft → reviewed post; immutable lines; account ledgers; trial balance/P&L/balance sheet; reversal | `test_accounting_ledger`, `test_accounting_account_ledger`, `test_accounting_posting_batch` |
| Bank receipts/payments | Retained canonical CSV, row identities, partial allocation, separately evidenced control totals | `test_accounting_bank`, `test_accounting_integration` |
| Payables/receivables | Retained source and posted control line; partial settlement; effective-dated metadata corrections | `test_accounting_schedules` |
| Structured manual schedules | Asset/depreciation/accrual/tax/restricted-fund records with exact values and provenance; no invented specialist calculator | `test_accounting_schedules` |
| Crypto recognition/settlement | Source claims and economic roles, real retained RP2 execution/cutoff/pool artifacts, ordinary GL drafts | `test_accounting_sources`, `test_accounting_calculation_capture`, `test_accounting_artifacts`, `test_accounting_projection` |
| Opening and transfers | Historical opening quantities/basis, fiat balances, transit across cutoff, settlement without duplicate recognition | `test_accounting_projection`, `test_accounting_projection_adversarial` |
| Book vs tax value | Retained valuation layers, actual consumed lots, partial release and transfer carry, exact cumulative currency rounding | `test_accounting_valuation`, `test_accounting_projection_adversarial` |
| Cash basis | Selected liquidity accounts, physical counts, exact partial payment allocations, income/expenditure distinct from accrual P&L | `test_accounting_cashbook`, `test_accounting_cash_adversarial` |
| Multi-year close | Shared close-readiness controls, immutable prior snapshots, reopen cascades, independent arithmetic verifier | `test_accounting_close_readiness`, `test_accounting_ledger`, `test_accounting_integration` |
| Retained evidence | SQLCipher bytes/text/reviews, bounded uploads, explicit local OCR or manual transcription, no remote fallback | `test_accounting_evidence`, `test_accounting_document_text`, `test_accounting_document_ocr`, `test_accounting_document_jobs` |
| Scoped AI | Exact selected disclosure, provider/book/revision binding, one-use tokens, no history/tools, preview→human approval→drafts | `test_accounting_ai_context`, `test_accounting_daemon_ai`, `test_accounting_ai_proposals`, `test_accounting_ai_result_tokens`, provider broker tests |
| AT filing preparation | 2025 K2 + K2kv/K2a/K2b/K11/K12/K12a; applicability/unknown/N/A states; specialist review; assessment-year aggregation | `test_accounting_jurisdiction`, `test_accounting_tax_workpapers` |
| Portable recovery | Real encrypted tar/age archive restores bytes, extraction, open items, source artifacts, projections and two close revisions | `test_accounting_backup_roundtrip` |
| Packaged runtime | Built PyInstaller sidecar launches; bundled AT resources load against encrypted fixture; real Poppler worker uses only pipes | `test_accounting_packaged_smoke` with explicit `KASSIBER_FROZEN_SMOKE_BIN` |

Test filenames above are module stems under `tests/`. UI models/components and
native export guards have additional tests under `ui-tauri/`. A passing row
demonstrates its fixture only, not arbitrary legal compliance or missing-source
completeness. Core support remains usable with AI disabled.

## Historical pre-stack verification checkpoint (2026-09-05)

These results were recorded before the dependency-stack integration. At that
time `codex/general-accounting` was at `b83ea695` plus uncommitted follow-up;
that work and subsequent opt-in fixes are now preserved in `21552bca`.
The old counts and manifest below do not describe the newer combined branch.
No accounting PR has been published and nothing here has been merged to `main`.

- Full UI suite: 131 files, 997 tests passed. Local socket tests required the
  normal approved execution environment; the sandbox-only EPERM run is not
  counted as a passing run.
- Final full Python suite (`.venv/bin/python3 -m pytest -q`): 4,039 passed,
  38 skipped and 337 subtests passed in 905.16 seconds. The earlier failing
  command-catalog run is superseded by this complete post-fix run. Optional
  dependencies/platform tests remain skipped where unavailable; two packaged
  smoke tests were additionally run against the explicit built binary below.
- Native Rust library suite: 120 tests passed, including export permission
  invalidation across lock/unlock, same-root round trips, failed/pending scope
  changes and actual daemon replacement.
- TypeScript build check passed; ESLint returned zero errors and 45 existing
  warnings. Production Vite and provider-broker bundles built successfully.
- Fast regtest: 39 tests passed. The encrypted backup/restore fixture passed.
- A fresh PyInstaller 6.20.0 Apple Silicon sidecar built from the frozen source;
  both real-artifact smoke tests passed with
  `KASSIBER_FROZEN_SMOKE_BIN` explicitly selecting that artifact. This is not a
  signed/notarized desktop release or an interactive UI test.
- Independent subagent reviews produced concrete fixes and passing regression
  cases for source coverage, lot valuation/reversal, minor-unit rounding, cash
  selection gaps, AI consent/result tokens, provider cancellation, retained tax
  export hashes and native export epochs. They are distinct from the requested
  final Claude review, which remains incomplete.

Frozen implementation/test manifest (excluding documentation):
`cc9db0c413d5c2a370afd061ba8a586f5eb106d93437bbd0d9fda67d8404f493`.
The command used at that historical working-tree checkpoint was:

```sh
git ls-files --cached --others --exclude-standard -z kassiber tests ui-tauri pyproject.toml uv.lock | sort -z | xargs -0 shasum -a 256 | shasum -a 256
```

## Combined dependency integration (2026-09-05)

The preserved accounting checkpoint `21552bca` and exact PR #542/#543 heads
are combined in `0601d26` on `codex/accounting-stack-integration`; see
[the stack record](../plan/18-general-accounting-pr-stack.md) for ancestry.
The following checks ran in a separate integration worktree, not the original
preview checkout:

- 108 focused CLI/capability/catalog tests passed.
- 517 accounting and dependency-boundary tests passed, 3 skipped, with
  6 subtests passed.
- Four additional real custody-review/GL/SQLCipher/backup integration scenarios
  passed in `test_accounting_review_integration.py`.
- TypeScript check and all 149 UI test files passed: 1,110 tests.
- Native library tests passed: 120 tests, using the locked offline dependencies.
- Bounded independent review checked the merge resolutions and new tests/docs.
  It found a Windows smoke-test environment mismatch and stale verification
  wording. Both were corrected; the affected CLI smoke module then passed
  all 76 tests on macOS. This is not a Windows runtime test or a full review of
  all inherited accounting changes.
- That complete Python gate finished with 4,255 passed, 38 skipped, 430
  subtests and two failures in unchanged Linux publisher tests under macOS
  Bash 3.2. The same failures reproduced on the dependency baseline; with
  installed Bash 5 on PATH, all 14 targeted publisher tests passed. No
  publisher implementation was changed.

Per-cut branch extraction, full combined CLI/agent/packaged acceptance and
AF-1 through AF-5 remain open. No existing dependency PR or main was changed.

## CLI/Agent-only pivot verification (2026-09-05)

Recovery checkpoint: `fbfce410` on `codex/accounting-ui-preserved-20260905`.
Current delivery branch: `codex/accounting-cli-agent`, in the separate
integration worktree; the original preview checkout is unchanged.

- A later full Python run finished with 4,289 passed, 38 skipped, 430 subtests
  and one failed assertion that still prohibited every accounting AI tool.
  This assertion now proves that **exactly** the four reviewed opaque task
  tools are exposed, while all other accounting kinds stay absent.
- After the pivot, the complete accounting/CLI-chat/private-agent selection
  passed: **513 tests, 3 skipped, 3 subtests**. Sandbox-only attempts denied
  local daemon/socket access; the recorded passing run used approved local
  test access with synthetic books/providers.
- All **137 UI test files / 1,021 tests** passed, including the retained exact
  Assistant consent and private provider isolation tests. Dedicated accounting
  UI tests were removed with that recoverable UI, not counted as passed.
- TypeScript passed. ESLint: zero errors, 50 existing warnings.
- Production Vite build passed (existing large-chunk advisory remains).
- Offline native library: **115 tests passed**. Native accounting save-picker
  code was removed; supervisor scope protections remain.
- The fresh-process CLI task and worklist rerun passed **9 tests** after
  replacing UI view targets with executable read-only CLI action/payloads.
  This includes synthetic prepare/post/close/export verification/retry and
  K2 finalization/export; it is not a real organization or hosted-model pilot.
- Selected financial assistance is reachable through hash-bound
  `chat --accounting-selection`, using the existing no-history disclosure
  and result grants. The combined chat/consent/disclosure regression passed
  **144 tests and 3 subtests** with loopback-only synthetic providers.
  After final UTF-8/error/diagnostic hardening, its focused suite passed
  **34 tests**, including real argparse defaults and no financial data in
  errors/public diagnostics. Selected-mode transport is scripted against
  real encrypted grant/proposal modules, not a real external-model run.
- Command capability/entrypoint checks: **81 tests passed**. Updated task/
  AI-core/consent/worklist checks: **113 tests and 4 subtests passed**.
- Autoreview local dry-run selected the requested Claude Fable 5.1/high
  target successfully; it did not invoke the model or constitute code review.
  Actual external review remains blocked as recorded below.
- The full repository gate has not been rerun on the final pivot. No full
  green release gate, updated packaged binary, completed Fable review, or
  published PR is claimed by these focused results.

The author-approved UI removals and retained security code are recorded in
`AGENT_OVERWRITES.md`. No database or financial records were deleted.

## Remaining delivery gates and deliberate exclusions

### Agent-first outcome gates

The strengthened outcomes in [spec 17 section 7.2](../plan/17-general-accounting-and-private-ai-spec.md#72-required-work-saving-outcomes)
are mandatory for the full stack. These are acceptance requirements, not executed tests.
For a pass, record fixture identity, exact source revision, actual tool/provider
path, manual comparison, measured results and review evidence. Synthetic data
must not be represented as the organization's confirmed pilot population.

| Gate | Required proof | Current status |
| --- | --- | --- |
| AF-1: Whole period | At least 100 mixed source records; complete selected-population coverage; correct routine proposals without repeated data entry; exceptions and user effort measured | 123-bank-row core case implemented; mixed-source/user-effort benchmark still pending |
| AF-2: Actual artifacts | Approved agent actions produce and verify the close, finalized K2/annex working papers and selected package; missing facts/denials remain honest partial outcomes | Pending end-to-end agent proof |
| AF-3: Reused decisions | Explicit rule approval; next matching case cites the rule; nonmatches, conflicting rules, another book, revocation and stale proposals fail safely | Core rules and tests exist; full measured agent outcome pending |
| AF-4: Same-task exceptions | Answer/new evidence resolves affected items and continues work; independent results survive; restart does not duplicate actions or resurrect consent | Explicit CLI assignments/restart tests exist; complete mixed-case agent outcome pending |
| AF-5: Private workflow | Agent resolves approved private assignment cases and verifies recalculated portfolio/basis reports without GL/company/K2 setup | Pending end-to-end agent proof |

### Other open gates and scope limits

- Agent-first completion is required by spec 17 sections 7.1 and 7.2. The current
  selected-context helper deliberately accepts one tool-free question and
  applies reviewed draft/document suggestions. Separate durable task tools now
  orchestrate bounded steps without financial disclosure. Full typed capability
  coverage and measured end-to-end outcomes remain open; neither the old
  source/test manifest nor the new synthetic tests prove every AF outcome.
- Finish the final frozen-input structured accounting/security review.
  Requested Claude CLI Fable 5.1 high review returned a session-limit 429, not a
  completed model review. Do not treat that attempt as approval or silently
  substitute another model. A later external-review attempt was blocked by the
  code-egress approval boundary; no subsequent Fable review has completed.
  The attempted helper invocation was `autoreview --engine claude --model
  claude-fable-5-1 --thinking high --mode commit --commit HEAD` against an
  isolated bounded snapshot, not a review of the local merge checkpoint alone.
- The 2026-09-05 isolated real-daemon browser check initially found an
  Accounting-route `app_error` without an active book. That empty-context case
  was fixed in checkpoint `21552bca`. The original preview was then checked
  with a synthetic book for book creation, accounting visibility on/off and
  the encrypted-book guard. This is limited UI proof, not complete accounting
  or agent acceptance. The later combined dependency branch has not received
  a new interactive or packaged acceptance pass; component/native unit tests
  do not substitute for it.
- Real optional Tesseract execution remains untested on this machine because
  it is not installed; synthetic worker and real Poppler tests are separate.
- Sensitive CLI providers and native PDF/OCR parsing are fail-closed on Windows
  until process-tree cancellation is tested there. Scoped HTTP AI, UTF-8 and
  reviewed manual transcription remain alternatives; no hidden hosted fallback.
- The canonical bank interchange is not a verified adapter for the unknown
  pilot bank export. Confirm that format before claiming the pilot import is done.
- Delivery now uses the [dependency-aware PR stack](../plan/18-general-accounting-pr-stack.md).
  PR #542 and #543 exact heads are combined locally with the preserved accounting
  checkpoint; both remained open on GitHub at integration. Per-cut PR extraction
  and final acceptance remain separate work. No main merge is authorized.
- EBICS, payment initiation, FinanzOnline transmission, automatic official-PDF
  filling, invoice issuing, consolidation and speculative further countries are
  excluded. K1/K3 receive an explicit unsupported route, not disguised K2 output.
