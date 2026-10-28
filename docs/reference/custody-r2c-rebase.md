# R2c rebase and interpretation audit

The local `t3code/r2c-interpretation-4` branch rebases `50120f69` onto the
components fixes at `16b5b57d`. Python remains authoritative; snapshot schema 1
is unchanged and product responses still list `interpretation` as skipped.

## Rebase

The four interpretation commits are retained as `e4184742`, `32ed9eca`,
`c7018146` and `98fc8528`. Only two files conflicted:

- `mutations.json`: retain all 167 fixed evidence/components entries and all
  139 interpretation entries, giving a 306-entry rebased baseline.
- `custody-engine.md`: retain the components review's corrected counts and
  historical-verification qualifier, plus every interpretation checkpoint.

Code, generators and text helpers merged without conflicts. The fixed component
semantics were retained. Inventory and contracts were regenerated with
`tests.rust_port.classify_tests update` and `tests.rust_port.contracts update`;
neither generated output needed a change.

## Audit findings

Recorded matching used `wide()` on raw amounts, including null/placeholder
values. It now mirrors Python's raw numeric comparison, preserving TypeError
for truthy strings/containers and supporting unbounded numeric comparisons.
The later Lightning principal equality uses fallible integer conversion.

Fan-out claim revalidation used `wide()` on a synthetic raw amount and read it
after the requested-amount short circuit. It now converts at Python's exact
point, including conversion failure when the requested amount is nonpositive.
Every target and wallet-uniqueness check must precede those conversions, and
multiple fan-out groups preserve first-seen exception precedence rather than
sorting group ids. Mixed-invalidity witnesses pin both admission boundaries.

Rowless native audits used `wide()` on raw amounts/fees and converted them
before checking eligibility. They now convert only eligible candidates, retain
null/falsy defaults, propagate Python exception families, and use unbounded
integer sums. Virtual channel-source audits follow the same conversion rules.

Recorded fan-out preserved refusal versus success but returned display messages
instead of Python exception families. Integer failures now preserve TypeError
or ValueError; split pricing preserves decimal.InvalidOperation and Overflow.
Frozen direct-deriver tests compare the family, not merely whether an error
occurred. The overflow witness must exceed Emax **after division**.
A multi-source residual witness additionally confirms the existing Python-sorted
quantity-hash order, independently of pair encounter order; that behavior was
already correct and is now protected by a named mutation.

The only remaining interpretation `wide()` calls read canonical observation
amount/fee integers in `claims.rs`. Every case conversion uses the CPython text
helpers, including the previously ASCII-only payment-hash lowercasing. Pair
chronology already delegates to the corrected components timestamp parser and
propagates its OverflowError; the 45,477 CPython timestamp vectors remain active.

The payment-hash helper change is equivalent under its preceding ASCII-hex
gate; the existing hash-case mutation remains its witness. Decimal error
variants sharing Python's InvalidOperation family use one match arm. A zero
denominator is unreachable because split pricing requires positive integer
principal; the decimal formatter's padding cap exceeds every arithmetic result
permitted by its Emax. No artificial kill is counted for those paths.

Fourteen new compiler-boundary vectors and eight transfer-predicate vectors pin raw
conversion, eligibility, ordering, unbounded quantities and refusal families.
One additional direct fan-out vector pins decimal division overflow. No Python
production module or snapshot fact was changed.

## Mutation additions

The catalog has 318 entries: 167 evidence/components, 139 retained
interpretation entries, and these twelve audit regressions. Existing mutation
targets were adapted to the corrected code without removing any rule.

| Mutation | Named killing tests |
| --- | --- |
| `R2c-audit-recorded-raw-type` | `recorded_cpython_transfer_predicates` |
| `R2c-audit-integer-family` | `pair_audit_bad_amount`, `fanout_boundary_bad_type` |
| `R2c-audit-falsy-integer` | `pair_audit_null_fee` |
| `R2c-audit-fanout-raw-order` | `pair_fanout_conversion_order` |
| `R2c-audit-rowless-raw-amount` | `pair_audit_raw_strings` |
| `R2c-audit-rowless-raw-fee` | `pair_audit_raw_strings` |
| `R2c-audit-rowless-conversion-order` | `pair_audit_skipped_bad_amount` |
| `R2c-audit-fanout-decimal-family` | `fanout_boundary_bad_decimal` |
| `R2c-audit-fanout-overflow-family` | `fanout_boundary_overflow_value` |
| `R2c-audit-fanout-admission-order` | `pair_fanout_wallet_admission_order`, `pair_fanout_target_admission_order` |
| `R2c-audit-fanout-group-order` | `pair_fanout_group_admission_order` |
| `R2c-audit-implicit-hash-order` | `pair_implicit_source_order` |

## Verification

Rust 1.99.1 and locked Python 3.11, with `MATURIN_NO_INSTALL_RUST=1`; Cargo
uses the offline cache. All books, mutation trees and recordings use `/tmp`.

| Check | Result |
| --- | --- |
| Workspace fmt / clippy with warnings denied | Passed |
| Workspace tests excluding `kassiber-net` | 686 passed |
| Full mutation catalog | 272 baseline selections passed; all 318 killed, no survivors, build failures or empty selections |
| Generated comparisons | 10,000 books; configured permutations passed |
| Recorded integrity | 195 snapshots, zero problems |
| Recorded Rust comparisons | 195 requests and 195 permutations, zero differences |
| Custody Python modules under census | 503 passed, 2,655 subtests passed; four daemon failures below |
| Ownership/on-chain/label/chronology Python modules under census | 133 passed, 12 subtests passed |
| Inventory / contracts regeneration and checks | Passed; generated output unchanged |
| Inventory, contracts, ledger, report and CLI-agent contract tests | 368 passed, seven subtests passed |
| Frozen interpretation vectors after final decimal correction | Passed |
| Ledger JSON feature contract | All 145 outputs byte-identical |

The 2,000 evidence books retain 345 successful reconciliation books / 690
receipts, 138 duplicate-slot books, and 69 books for each tampered-hash branch.
The generator still asserts active counts for every component/interpretation
scenario; successful controls were preserved during rebase.

| Generator | Books | Active | Blocked | Inactive | Additional permutations |
| --- | ---: | ---: | ---: | ---: | ---: |
| Components | 2,000 | 474 | 1,629 | 13 | 2,000 |
| Samourai | 2,000 | 1,176 | 332 | 492 | 68 |
| Recorded pairs | 2,000 | 789 | 176 | 1,035 | 92 |
| Recorded fan-out | 2,000 | 1,244 | 126 | 630 | 94 |

Component columns count components rather than books; a book may contribute
multiple components. The evidence corpus additionally runs 58 permutations.

The four daemon failures are the same stable owner-lock restriction documented
in the [components review](custody-r2b-review.md#verification): two tests in
`CustodyComponentDaemonSurfaceTests` and two in
`CustodyGapDaemonProtocolAcceptanceTests`. A focused reproduction captured
the daemon's internal-error envelope: `_owner_lock_root` calls `os.chmod` on
`~/.kassiber/run`, which the sandbox rejects as read-only, before custody runs.
No production guard was relaxed and no owner-lock path was redirected.
The complete census and custody-shadow shards remain for the orchestrator;
this report does not claim those lanes or the full quality gate passed.

## Next slice investigation

No graph/ownership/consolidation port is included in this audit checkpoint.
An isolated `/tmp` Cargo probe compiled `elements` 0.25.3 and its full dependency
closure offline. No dependency was added to Kassiber, so no runtime, lockfile or
license inventory changed.

A reference-generated public transaction with one nonuniform-txid input and
explicit valued/fee outputs decodes in both Python and `elements`. Directly
substituting the consensus decoder would nevertheless break current Python
semantics:

| Input variation | Python reference | `elements` 0.25.3 |
| --- | --- | --- |
| Remove the final four locktime bytes | Accepts | UnexpectedEof |
| Set the witness flag byte to 2 | Accepts as no witness | Bad witness flag |
| Encode input count 1 as `fd0100` | Accepts | NonMinimalVarInt |
| Append a trailing byte | Refuses | Refuses |

The next implementation needs frozen decoding vectors and explicit compatibility
handling around the mature decoder, including confidential fields, issuance,
peg-ins and witness structure. Do not silently tighten these reference semantics
or replace a raw fact with a Python decoding verdict. Then port exact graph
conflict labels, ownership-index/scope behavior, ownership derivation and
multi-source consolidation; compose consolidation -> fan-out -> ownership and
add successful Bitcoin/Liquid generators with per-scenario active assertions.
Each ported branch still needs a killed mutation or documented equivalence.
