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
