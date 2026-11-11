# kassiber-pyjson

CPython's `json.dumps` output, byte for byte, and `repr(float)`, for the
Rust backend ([plan 21](../../docs/plan/21-rust-backend.md)). Kassiber hashes
and stores text that `json.dumps` wrote; recomputed commitments are compared
with stored ones, so Rust must write the same bytes.

```rust
use kassiber_pyjson::{dumps, Key, Options, Value};

let payload = Value::Dict(vec![
    (Key::from("schema_version"), Value::from(1)),
    (Key::from("amount_msat"), Value::from(150_000_000_u64)),
]);
// json.dumps(payload, sort_keys=True, separators=(",", ":"))
let text = dumps(&payload, &Options::new().compact().sort_keys(true))?;
assert_eq!(text, r#"{"amount_msat":150000000,"schema_version":1}"#);
```

`Value` mirrors the Python object: `None`, bools, unbounded ints, floats
(infinities and NaN included), strings that may hold surrogates (`PyStr`), bytes,
`Decimal` (from `kassiber-pydecimal`), lists, and dicts as ordered pairs
with `str`, int, float, bool, or `None` keys. `Options` mirrors the keyword
arguments: `separators`, `indent`, `ensure_ascii`, `sort_keys`, `allow_nan`,
and `default=str`. The crate docs list what matches CPython 3.11.

`Value::Bytes` preserves raw SQLite BLOBs. JSON encoding refuses them with
`Object of type bytes is not JSON serializable`, unless `default=str` selects
Python's bytes repr. `json_bytes_string` separately detects UTF-8/16/32 and
preserves surrogate code points for consumers that call `json.loads(bytes)`.

## Where it refuses

Every refusal Python makes is an `Error` naming the exception, in Python's
order when a value has several problems. Three refusals are this crate's:

- `NanKeyOrder`: `sort_keys` over two or more numeric keys, one of them NaN.
  Python sorts them with an inconsistent comparison; reproducing that would
  mean reimplementing its sort.
- `TooDeep`: nesting beyond 1,000 lists and dicts. CPython stops at its
  recursion limit, which counts its caller's frames too, so a document a
  few levels short of 1,000 may still fail in Python.
- `Surrogate`: with `ensure_ascii=False` the result would hold a surrogate.
  Python returns such a string, but encoding it as UTF-8 for a hash, a file,
  or SQLite raises `UnicodeEncodeError`, which is where every Kassiber
  caller ends up.

Out of scope: `skipkeys`, `check_circular`, `cls`, callables other than
`str` for `default`, and objects other than the ones above. Kassiber passes
none of them. Converting a `Decimal` to a float before dumping, as
`json_ready` does, is the caller's step.

## Call variants in the Python backend

This survey records which option sets the Python backend used, so it explains
which bytes the stored data holds. Python was removed under
[plan 22](../../docs/plan/22-python-removal.md); the module paths below exist
at the `archive/python-backend` tag. On 2026-10-26 a survey script
(`scripts/survey_json_dumps.py`, also only at that tag) classified every
`json.dumps` and `json.dump` call in `kassiber/` by its keyword arguments. It
found 253 calls in 21 written variants, which are 14 option sets once
explicit defaults (`ensure_ascii=True`, `sort_keys=False`) and `json.dump`
are folded in:

| Calls | Option set | `Options` |
| ---: | --- | --- |
| 106 | `sort_keys=True` | `new().sort_keys(true)` |
| 47 | compact, `sort_keys=True` | `new().compact().sort_keys(true)` |
| 29 | compact | `new().compact()` |
| 27 | defaults | `new()` |
| 13 | `indent=2, sort_keys=True` | `new().indent(2).sort_keys(true)` |
| 9 | compact, `sort_keys=True, ensure_ascii=False` | `...compact().sort_keys(true).ensure_ascii(false)` |
| 6 | compact, `sort_keys=True, allow_nan=False` | `...compact().sort_keys(true).allow_nan(false)` |
| 5 | `indent=2` (the machine envelope) | `new().indent(2)` |
| 4 | compact, `sort_keys=True, default=str` | `...compact().sort_keys(true).default_str(true)` |
| 3 | `sort_keys=True, default=str` | `new().sort_keys(true).default_str(true)` |
| 1 | compact, `ensure_ascii=False` | `new().compact().ensure_ascii(false)` |
| 1 | compact, `sort_keys=True, allow_nan=False, ensure_ascii=False` | |
| 1 | compact, `sort_keys=True, default=str, ensure_ascii=False` | |
| 1 | `default=str` | `new().default_str(true)` |

"Compact" is `separators=(",", ":")`. The script also counts calls whose
function hashes (44 by its heuristic). These are the hashed or persisted
outputs a port has to keep byte-identical:

| Site | Option set | Output |
| --- | --- | --- |
| `core/custody_evidence.py` `_hash_payload` | compact, sort, `ensure_ascii=False` | evidence SHA-256, and the encoded text is stored |
| `core/chain_observer/provenance.py` `canonical_graph_hash` | compact, sort, `ensure_ascii=False`, `default=str` | graph hash |
| `core/chain_observer/provenance.py` `canonical_observed_quantity_hash` | compact, sort | quantity hash |
| `core/chain_observer/provenance.py` observer id and kind columns | defaults | stored JSON |
| `core/custody_gap_reviews.py` `candidate_fingerprint`, `authored_claim_fingerprint` | compact, sort | gap fingerprints |
| `core/custody_quantity_runtime.py` `_issue_id` | compact | issue ids |
| `core/sync_backends.py` `esplora_stats_fingerprint`, the membership backend key, `raw_json` columns | sort | fingerprints and stored JSON |
| `core/sync_backends.py` address-set and backend fingerprints | compact, sort | fingerprints |
| `core/custody_filed_reports.py` `_json` columns, `artifact_content_sha256` | compact, sort; compact, sort, `ensure_ascii=False` | stored JSON, export manifest hash |
| `fingerprints.py` `make_transaction_fingerprint` | sort | transaction fingerprints |
| `envelope.py` `emit` | `indent=2` | CLI machine output |

A new option set needs vectors before Rust code relies on it. The repository
no longer has a CPython oracle for new cases; add them through the
frozen-golden flow in [plan 22](../../docs/plan/22-python-removal.md#generators-become-frozen-goldens).

## Vectors

`tests/data/json_vectors.txt` holds about 21,000 cases from CPython 3.11
(made once by a fixed-seed generator, under 3 MB, one case per line):
about 17,000 floats (specials, every power of two and ten with its
neighbours, the layout boundaries, 2,500 exact ties, random bit patterns,
and money-like decimals) and about 4,300 `json.dumps` calls: every control
and printable ASCII character and selected non-ASCII, astral, and
surrogate code points in both `ensure_ascii` modes, int boundaries and the
digit limit, every key type with and without `sort_keys`, `Decimal` text,
other indents and separators, error precedence, Kassiber-shaped payloads
for each surveyed option set, and random nested documents for all 64
combinations of the varied options. `tests/json_vectors.rs` checks every
line and fails when a combination has too few cases.

The file is a frozen golden: it is locked test data
(`tests/test-data.lock`, fetched by `scripts/fetch-test-data.sh`), and nothing
in the repository regenerates it. The generator,
`crates/pyjson/scripts/gen_json_vectors.py`, exists only at the
`archive/python-backend` tag.

```sh
scripts/fetch-test-data.sh
cargo test -p kassiber-pyjson
```

`KASSIBER_JSON_VECTORS` points the test at another vector file of the same
format, for example a larger stress set made with that generator at the tag:

```sh
KASSIBER_JSON_VECTORS=<path to json_stress.txt> \
    cargo test --release -p kassiber-pyjson --test json_vectors
```
