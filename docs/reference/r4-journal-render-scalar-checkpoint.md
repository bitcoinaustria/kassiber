# R4-11 scalar prerequisite checkpoint

This local continuation starts at `de05eaa90` on
`t3code/r4-11-render-fidelity`. **The requested scalar-fidelity work is not
finished, and R4-11 remains unaccepted.** This commit establishes scalar
primitives and corrects compound-value coercion. It does not complete the
renderer-wide replacement of serde values.

The requirement is to preserve Python scalar types, exact output bytes and
refusal order without adding checks to ordinary render loops. A lossy bridge
could replace a surrogate, change an integer or non-finite float, reorder a
mapping used by `str()`, or consume an identity before Python would. The scoped
steps here are to reuse the existing native Python value algebra, freeze
Python-derived witnesses, correct the renderer's compound coercion, and verify
that boundary with named mutations and the inherited cases.

## Implemented boundary

`crates/journals/src/scalar.rs` uses the existing `kassiber_pyjson::Value` and
`PyStr` types. Its raw decoder uses the CPython-compatible `kassiber_pytext`
scanner. Native strings can contain actual surrogates, integers remain wide,
and NaN and both infinities remain floats. The native encoder emits compact,
sorted ASCII JSON. Python truth, text and representation operations retain the
scalar distinctions. The oracle snapshot adapter tags native non-finite floats
and Decimals separately from decoding; it does not interpret literal user
objects as scalar tags.

The checked conversion to serde refuses actual surrogates, non-finite floats,
Decimals, non-string or repeated keys, and mapping order that serde would
discard. Refusal means the caller must retain the native value. This conversion
is a prerequisite for a cold renderer; it is not wired into a complete fallback.

The existing renderer now uses Python representation for compound values where
it previously used JSON spelling. This corrects anchors and profile methods
containing lists, booleans, nulls and nested mappings. Ordinary scalar coercion
retains the existing path. The typed renderer still receives serde mappings, so
it cannot recover insertion order already discarded by its input decoder.

No dependencies, custody branch rules, tax computations, persistence formats,
external I/O or Python reference behavior change.

## Witnesses and verification

The new immutable Python-derived corpora are:

- `render-scalars.json`: 26 native-value cases, checking exact raw encoding,
  Python text and representation, scalar kind, truth, snapshot bytes and checked
  serde admission. Actual surrogates and literal backslash-u text are distinct.
- `render-scalar-refusals.json`: six raw decoder refusals, comparing the complete
  exception projection, including family, message and all optional attributes.
- `render-scalar-coercions.json`: 18 real Python `build()` captures, comparing
  complete state bytes and all 36 UUID tape consumptions.

The original 383 cases, the 39 audit/carry cases and the 550 response-refusal
cases are retained unchanged. The refusal cases compare both cold traversal and
typed-decoder-first routes, including every declared identity.

All **41 all-feature release journal tests** pass, including the new scalar
checks and existing persistence/rollback tests. All **six Python oracle/capture
tests** pass, rebuilding the original and new corpora deterministically.
Workspace formatting and scoped all-target/all-feature release clippy with
`-D warnings` pass. The required classifier update leaves the inventory
unchanged, and CI preflight passes all **57 tests**. These checks do not replace
the repository quality gate.

All **ten new mutation recipes** are killed, with passing baselines and no
survivors or build failures:

- `lossless_scalars_match_python_bytes_and_types` kills eight recipes covering
  string text, NaN truth, wide integer text, sorted encoding, non-finite and
  surrogate admission, mapping order, and non-finite snapshot identity.
- `scalar_coercions_match_python_rendering` kills the compound coercion recipe.
- `lossless_scalar_decode_refusals_match_python` kills decoder-family drift.

The unchanged recipes live in the separate
`crates/journals/scripts/scalar_mutations.json` catalog to avoid overlapping the
parallel audit's catalog edits. Applicability passes for all ten. The local
source-bound summary records the unchanged recipe relocation from the combined
catalog used during execution. It binds each kill to source/fixture hashes and
its log. Mutation source and build trees are deleted after summarizing.

The existing ten-case boundary repro still has four matches and six differences:
four actual surrogate cases, opaque descriptions, and ignored holding decimals.
Five additional Python-derived repros record two compound mapping-order
differences and raw NaN/positive-infinity/negative-infinity workspace values
that still fail the renderer's serde input decoder. These failures remain
explicit acceptance blockers.

## Final CPU campaign

One campaign uses **five alternating pairs**, as requested, pinned to CPU 0.
The retained ninth-continuation baseline probe has SHA-256
`cf31fc7b920548329c71bc86438646ece940808b3dc364f4d5f98047bbe7ff8b`.
Both probes first match the complete Python state bytes for both 20k books.
No build or other test job in this worktree overlaps the comparison. The span
continues to exclude response decoding, canonical encoding, persistence and
state destruction. The local comparison script permits the requested five
pairs; the checked-in comparator's seven-pair minimum is unchanged.

| Book | Baseline best CPU (s) | Head best CPU (s) | Baseline median (s) | Head median (s) |
| --- | ---: | ---: | ---: | ---: |
| Generic 20k | 0.233867 | 0.230187 | 0.320256 | 0.292746 |
| Multi-wallet 20k | 0.157629 | 0.151618 | 0.183616 | 0.171543 |

This run does not recertify the settled 0.12 s generic boundary; the unchanged
baseline also misses it. The five head samples are lower at both best and
median than this baseline, under shared-host load near 42–44. These samples do
not establish a fidelity regression or replace final integrated CPU acceptance.
There is no additional timing campaign or speed optimization here.

The local evidence lives in `target/r4-fidelity/`. Its verification index binds
the final source, fixtures, probes, copied operands, named mutation kills,
check logs, repros and all CPU samples. Verification build trees are deleted
after summarizing; probes, operands, logs and summaries remain local.

## Remaining work

1. Integrate the native value carrier through profiles, projection rows, entries,
   source provenance, holdings and joins. The current typed input and output
   models still use serde values and Rust strings; the scalar primitive tests
   do not certify that integration or finish requested item 1.
2. Repair the actual surrogate repros for workspace ids, entry descriptions,
   source provenance and pass-through rates, retaining literal backslash-u
   witnesses and exact error messages that themselves contain surrogates.
3. Carry raw NaN and infinities through every consumed and ignored stage, with
   exact Python refusal families, messages, precedence and UUID consumption.
   Primitive raw decoding alone does not complete requested item 3.
4. Return successful states from lazy response decoding, including opaque
   descriptions and malformed decimals on ignored unknown-wallet holdings.
5. Complete shape, refusal and UUID fidelity, including mapping insertion order
   in compound coercions, cumulative arithmetic and competing failures. Add and
   kill named mutations for each newly supported route and rerun the frozen and
   refusal suites at each verified boundary.

The separately assigned branch audit and the checkpoint's complete renderer,
persistence and custody mutation campaigns, generated-input/shadow lanes,
integrated loaders and repository quality gate remain outside this prerequisite
commit. The new focused mutation kills do not certify those campaigns.

## Native ledger continuation

This local continuation starts at `b3fabebb9` on
`t3code/r4-11-render-fidelity-2`. **R4-11 remains unaccepted.**

`crates/journals/src/native.rs` adds a comparison-only ledger reader over the
native scalar algebra. Profiles, projection rows, entry fields, fragment
provenance, carry overlays, holdings, audits, quarantine details, compatibility
payouts and summaries retain native values. It shares the renderer's existing
Decimal formatting and msat operations. Response fields are decoded when Python
consumes them, and the caller still owns every entry identity. No command,
production authority, persistence format, dependency or external I/O changes.

The reader returns an owned `native::Ledger`. Its oracle snapshot keeps native
nonfinite floats and Decimals distinct from literal objects. `native::Error`
retains a native message, including actual surrogates in interpolated error
text. The existing typed renderer and diagnostic probe remain unchanged.

The immutable `render-native-ledger.json` witnesses are derived by calling the
unchanged Python `ledger_response.read_ledger` or `empty_result` on every
existing render operand. The adapter preserves the original wallet mapping
keys and injects the declared identity tape at `journal_entry_id`. All **990**
ledger states/refusals match exactly, including **383** frozen cases, **39**
audit/carry cases, **550** response-refusal cases and **18** scalar coercions.
This separate ledger oracle includes the complete intra-audit output, which
`CustodyJournalBuilder.build()` omits from its returned state. The existing
corpora are unchanged.

All **42** all-feature release journal tests, the six Python oracle/capture tests,
workspace formatting and scoped all-target/all-feature release clippy pass.
All **ten** focused recipes in `crates/journals/scripts/native_mutations.json`
are killed by `native_ledger_matches_python_consumption_and_scalars`, with a
passing unmutated baseline and no survivors or build errors. Their source-bound mutation results and verification logs are retained locally under
`target/r4-fidelity-2/`.

This is a ledger boundary, not completion of requested item 1. Native
builder-state assembly, diagnostic joins and the probe's raw decoder/handoff
remain open. The four end-to-end surrogate repros therefore remain open even
though the native ledger can carry those scalar types. The new ledger oracle
does not certify malformed projection shapes, all wallet mapping shapes,
nonfinite values in every consumed/ignored stage, or the complete native
shape/refusal/UUID campaign. Transfer quantities still use the existing
`JournalQuantity` snapshot adapter, and cross-asset projection shape consumption
is not certified by this boundary. Successful native ledger results still need to be
returned through the complete builder boundary. Python reference behavior is
unchanged; no reference bug was identified.


### Continuation verification and final CPU pairs

The required classifier update leaves the inventory unchanged, and CI preflight
passes all **57 tests**. All **188** existing and new recipes apply; only the ten
new native-ledger recipes were executed in this continuation. The mutation
source and build trees are deleted after their source-bound summary is saved.

The single final campaign uses seven alternating back-to-back pairs on CPU 0
against the retained `8a9492199` probe, whose SHA-256 is
`3f0e6b8134ee075133ed906194491cec4f5e0389a7298419172ee981ffafec72`.
Both probes first match complete state bytes on both identical copied 20k
operands. No build or other test overlaps the campaign.

| Book | Baseline best CPU (s) | Head best CPU (s) | Baseline median (s) | Head median (s) |
| --- | ---: | ---: | ---: | ---: |
| Generic 20k | 0.236217 | 0.229761 | 0.284954 | 0.275336 |
| Multi-wallet 20k | 0.205968 | 0.227631 | 0.318648 | 0.353668 |

Both probes miss the 0.15 s target. The retained baseline has stage timing
instrumentation; this head probe uses default features. That build-feature
difference limits the comparison. These samples measure the existing typed
builder, because the new native ledger reader is not yet wired into its probe.
They do not certify the new reader's performance or establish a fidelity
regression. No second timing campaign or speed optimization is performed.

`target/r4-fidelity-2/verification-index.json` binds the final source, fixtures,
mutation kills, passing logs, probes, copied operands and all CPU samples.
Verification build trees are removed after summarizing. The probes, operands,
logs and summaries remain local. No push, PR or external comment is performed.


## Native builder assembly continuation (partial)

This local continuation starts at `8225d58c1` on
`t3code/r4-11-render-fidelity-3`, with the three branch-audit commits from
`de05eaa90..71307a23f` cherry-picked first. **R4-11 remains unaccepted.**
This is a builder assembly checkpoint, not completion of the requested scalar
fidelity or acceptance campaigns.

`native::build` now returns a complete comparison state, and `native::finish`
accepts the owned native ledger for the diagnostic joins. The state borrows the
custody facts and assembles its canonical oracle snapshot outside the build
span. Pass-through rates, blockers, transfers, quantity facts, warnings and
ignored identities keep their native scalars. Reconciliations use the existing
oracle dataclass projection convention. Native balance diagnostics retain
surrogate identities and exact wide integer totals, loan/channel/suspense reason
precedence, sorted differences and the unique-reason rule.

Finite transfer quantities now remain native Decimals through this state,
without serializing through serde. Nonfinite transfer quantities retain the
existing explicit decimal carrier. Native audit-pair indexing now preserves
the finalized projection's `tuple index out of range` refusal, as the merged
branch-audit witnesses require.

The immutable `render-native-builder.json` corpus adds **30** production Python
`build()` captures with deterministic UUID tapes. It includes actual and literal
surrogate workspace values, entry descriptions, source provenance and
pass-through rates; raw NaN and both infinities in those fields; opaque list,
mapping and null descriptions; mapping insertion order in methods and anchors;
ignored malformed/surrogate/NaN unknown-wallet holding decimals; and a surrogate
wallet identity in the balance join. Raw custody float operands are restored
in the captured input after the oracle snapshot adapter runs, so they reach
the native decoder as float tokens rather than literal `$float` objects.

The native builder test compares complete state bytes or every refusal
attribute and cause, plus exact UUID consumption, across **1,254** cases:
383 original cases, 39 audit/carry cases, 550 response refusals, 18 scalar
coercions, all 234 merged branch witnesses and the 30 new builder captures.
The prior corpora are unchanged. Python product behavior is unchanged; no
Python reference bug was identified.

### Explicit remaining boundaries

The ownership review join still calls the existing serde graph helpers.
Its comparison adapter canonicalizes mapping order and refuses native scalars
that serde cannot represent. Those admission refusals are an implementation
limit, not a claim of Python-equivalent refusals. The no-quarantine path uses
native values directly. Arbitrary graph scalars, graph mapping-order coercion
and ignored malformed graph operands still need a native consumption path.
Consequently requested item 1 is only partially complete.

The diagnostic `journal-render` probe is unchanged and still uses serde for
raw input and the typed builder. The native builder's surrogate witnesses do
not close the four end-to-end surrogate repros. The probe decoder/handoff,
raw nonfinite consumed/ignored-stage campaign, complete successful lazy
response campaign, arbitrary shape/wallet-map/arithmetic/refusal-order/UUID
campaign and final typed loader/custody integration remain open.

No full renderer/persistence/custody mutation campaign, 25,030-input run,
census/shadow or fresh recording campaign, integrated SQLite/SQLCipher
campaign, repository quality gate, final CPU comparison against `8a9492199`
or acceptance record is claimed by this continuation. Existing SQLCipher
fixture tests are narrower than the required integrated campaign. No timing
campaign is run against this partial implementation.


### Verification at this boundary

All eight recipes in `crates/journals/scripts/builder_mutations.json` are
killed by `native_builder_matches_python_states_refusals_and_uuid_tapes`, after
a passing unmutated baseline. They cover pass-through rates, canonical sums,
wallet key coercion, suspense precedence, ambiguous reasons, reconciliation
projection, native finite transfer quantities and tuple refusal text. There
are no survivors or compiler failures. All 324 current journal, persistence,
branch, scalar, native-ledger and builder recipes apply; only the eight new
builder recipes were executed in this continuation. The local source-bound summary checks
original-worktree hashes after execution and binds each named kill to its
recipe, fixtures and log. Mutation source and build trees are deleted after
the summary is saved.

All **47** all-feature release journal tests and all **eight** Python
oracle/capture tests pass, including deterministic rebuilding of the new
captures. Workspace formatting and scoped all-target/all-feature release
clippy for custody and journals pass with `-D warnings`. The required classifier
update leaves the inventory unchanged, and the required CI preflight passes
all **57 tests**. These checks do not replace the repository quality gate.

The local evidence is under `target/r4-fidelity-3/`. The verification index
binds the changed source, fixtures, catalog and check logs; the verification
build tree is removed after summarizing. No push, PR or external comment is
performed.

## Native ownership graph continuation

This local continuation starts at `a30e13088` on
`t3code/r4-11-render-fidelity-4`. **R4-11 remains unaccepted.** This boundary
completes native scalar consumption in the ownership review join. It does not
wire the diagnostic probe or certify the remaining acceptance campaigns.

The native builder no longer serializes, sorts and reparses ownership operands
through serde. Native graph parsing, physical and inferred scopes, owned-script
and outpoint lookup, component exclusions, legacy grouping, candidate amounts
and metadata retain their Python scalar representation until consumption.
Actual surrogates remain distinct from literal backslash-u text. Compound
string coercions retain mapping insertion order; amounts remain arbitrary-width
integers. Inbound coercion refusals, graph-index overflow and malformed script
refusals propagate through the native error carrier. Unused graph, component
and metadata values do not acquire an admission refusal.

The parser projects only the graph fields consumed by this diagnostic join.
It merges complementary stored observations in Python's input/output order,
retains the first present script/value and requires valued outputs before
using graph ownership. Ignored leg asset/role/conflict metadata grants no
ownership or accounting authority. Liquid raw-hex decoding reuses custody's
existing local protocol decoder, after checking the consumed hex operand;
no dependency, I/O, product route or Python accounting behavior changes.

`render-native-graph.json` adds **181** production Python builder captures,
including **11** refusals. They cover actual/literal surrogate identities,
script keys and candidate fields; raw NaN and both infinities; ordered mapping
and list coercions; wide integers; graph output-index/value/script/asset/role
operands; malformed inbound amounts; configuration scope and ignored component
fields. Raw custody scalars are restored after the snapshot adapter so they
reach the native reader as their original tokens, including quarantine ids.
The existing six corpora remain unchanged. The native builder now compares
complete canonical states or all refusal attributes and causes, with exact
UUID consumption, across **1,435** outcomes.

### Verification and remaining work

The all-feature release journal suite passes all **47 tests**. Scoped
all-target/all-feature release clippy for custody and journals passes with
`-D warnings`; workspace formatting passes. All **613** all-feature release
custody tests and all **eight** Python oracle/capture tests pass, including
rebuilding the native graph captures from production Python. The required
classifier update leaves the inventory unchanged; required CI preflight passes
all **57 tests**.

All **ten** recipes in `crates/journals/scripts/native_graph_mutations.json`
are killed by `native_builder_matches_python_states_refusals_and_uuid_tapes`
after a passing unmutated baseline, with no survivors or compiler failures.
They pin surrogate preservation, script case conversion, the sats/msat factor,
mapping insertion order, legacy identity whitespace, graph-index overflow,
unknown output values, integer refusals, exact confidence and reason precedence.
All **334** renderer/persistence/branch/scalar/native recipes apply; only the
ten new graph recipes were executed for kills in this continuation.

Local evidence is retained under `target/r4-fidelity-4/`. The mutation summary
binds each recipe and named kill to source/test/fixture hashes and checks for
worktree drift. The verification index binds the passing check logs and proves
the six inherited render corpora are unchanged. No Python reference bug was
identified. No timing campaign, push, PR or external comment is performed.

Requested items 2–6 remain open: lossless probe decoding and the native handoff
(including all four end-to-end surrogate repros); raw nonfinite fidelity across
all consumed and ignored stages; successful lazy decoding through the probe;
complete shape, wallet mapping/order, arithmetic, refusal-order and UUID
fidelity; then the full renderer/persistence/custody mutation shards, all
25,030 inputs, census/shadow/fresh recordings, integrated SQLite/SQLCipher
campaigns, repository gate, paired CPU checks against `8a9492199`, and the
acceptance record. The focused native graph witnesses do not certify those
campaigns. Python retains production accounting authority.

## Lossless probe handoff continuation

The next verified boundary on `t3code/r4-11-render-fidelity-4` wires
`journal-render` to the lossless scalar decoder and native builder. Raw file
decoding precedes the measured build span; canonical snapshot encoding and
state destruction follow it. The probe retains native refusals and reports
exact UUID consumption in normal and timing-only modes. Its path cache retains
the lossless operands. No native product accounting route is enabled.

All **four** end-to-end actual-surrogate repros now match Python: workspace id,
entry description, source provenance and pass-through rate. The literal
backslash-u witness remains distinct. All **ten** cases in
`check-journal-render-boundary.py` pass, also covering opaque descriptions,
ignored malformed holding decimals, nonfinite detail, malformed integer
coercion and canonical-decimal refusal.

`scripts/check-journal-render-corpus.py` replays all seven frozen corpora through
the executable, then repeats each path to exercise cached operands. It compares
complete state bytes or every refusal attribute/cause and exact UUID use. All
**2,870 replies** match across **1,435 cases**: 691 successful states and 744
refusals, each checked after fresh decoding and a cached read. The example also
registers a Cargo test that checks all 1,435 outcomes in both normal and
timing-only modes using the probe's decoder and handoff.

### Verification and explicit stopping boundary

All **48** all-feature release journal tests pass, including the registered
probe test. Workspace formatting and scoped all-target/all-feature release
clippy for custody and journals pass with `-D warnings`. The final release
executable repeats the ten boundary repro matches and all 2,870 corpus replies.
All **four** recipes in `crates/journals/scripts/probe_mutations.json` are killed
by `lossless_probe_matches_python_states_refusals_and_uuid_consumption` after
a passing baseline, with no survivors or compiler failures in the accepted run.
The discovery recipe for the decoder initially called a private refusal
constructor and did not compile; changing it to the existing public conversion
and rerunning all four recipes closes that tooling error. The discovery run
is excluded from kill evidence.

The earlier graph-mutation run reused a Cargo build tree; a later
all-feature probe test exposed its last compiled mutation artifact despite the
restored source. That discovery baseline is excluded. Journal artifacts were
cleaned, the unmutated probe test passed, and the probe mutation run uses a
separate build tree. Final verification uses the original worktree build tree. The required
classifier update leaves the inventory unchanged, and the final required CI
preflight passes all **57 tests**. All **338** renderer/persistence/branch/scalar/
native/probe recipes apply; only the ten graph and four probe recipes are run
for new kills in this continuation.

`target/r4-fidelity-4/verification-index.json` binds the final source, fixture,
recipe, executable and passing-log hashes to the accepted mutation summaries
and final executable replies. Mutation and verification build trees are
removed after summarizing; the release probe, local logs and summaries remain.
No background job remains at handoff. No push, PR or external comment is
performed.

Requested items **3–6 remain open**. The successful lazy and nonfinite witnesses
above close their known repros, but they do not certify every consumed/ignored
nonfinite stage, the full successful lazy-decoding campaign, arbitrary shape,
wallet mapping/order, cumulative arithmetic, competing refusal order or UUID
fidelity. Full renderer/persistence/custody mutation shards, all 25,030 inputs,
census/shadow/fresh recordings, integrated SQLite/SQLCipher campaigns, repository
gate, paired CPU checks against `8a9492199` and the acceptance record are still
required. **R4-11 remains unaccepted.** No Python reference bug or product
accounting change is identified.

Reproduction uses the setup/environment specified above, with `TMPDIR`,
`CARGO_TARGET_DIR`, `CARGO_BUILD_BUILD_DIR` and `UV_CACHE_DIR` inside this
worktree's `target/`, Cargo offline and eight build jobs:

```sh
cargo test --offline --locked --release -p kassiber-journals --all-features -- --test-threads=1
cargo build --offline --locked --release -p kassiber-journals --example journal-render
uv run --locked python scripts/check-journal-render-boundary.py --probe PROBE --output target/r4-fidelity-4/probe-boundary
uv run --locked python scripts/check-journal-render-corpus.py --probe PROBE --output target/r4-fidelity-4/probe-corpus
```
