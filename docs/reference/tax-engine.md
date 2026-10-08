# Tax engine

Kassiber's tax engine is a Rust crate (`tax-engine/core`) called from Python
through a PyO3 binding (`tax-engine/python`, imported as `kassiber_tax`). It
replaces RP2 under [plan 20](../plan/20-kassiber-tax-engine.md). This page owns
the engine contract: the request and response format, the Python layer the
adapter calls, and the decimal rules both sides follow.

## Where the engine sits

Journals run on `KassiberTaxEngine` (`kassiber/core/engines/ledger.py`). For
every book with projected rows it builds a schema-2 request
(`ledger_request.py`), which runs the configuration checks and refuses shapes
a finalized projection cannot have. It sends the book to the engine's
`ledger` operation only when the engine implements every capability the book
uses (`ROUTED_CAPABILITIES`, a subset of `CAPABILITIES`: `generic`,
`generic_carry`, `at`, `at_swap`). Plan 20's phase 4 moves them one at a
time; so far `generic` and `at` are routed, so a generic book without a
carrying-value Bitcoin rail pair and an Austrian book without a
carrying-value cross-asset pair run on the engine, and `ledger_response.py`
renders the engine's journal ledger into `TaxEngineLedgerResult`: uuids,
profile, wallet and account ids, row times, the legacy floats, and each
fragment's row facts. Every other book runs on the phase-3 adapter,
`kassiber/core/engines/rp2.py`, which turns the finalized custody projection
into journal entries with the engine underneath it in place of RP2's
calculation. The adapter keeps its gates, quarantines, journal composition,
reviewed overrides, and fee reporting rules.

The adapter reaches its calculation backend through `_get_rp2_modules()`.
That returns either the RP2 modules or `kassiber.core.engines.native`, which
mirrors the subset of RP2's interface the adapter uses:

| Name | Native meaning |
|---|---|
| `Configuration`, `AbstractCountry`, `AccountingEngine`, `AVLTree` | Run settings: assets, wallets, the holder, the country policy, and the single accounting method |
| `InTransaction`, `OutTransaction`, `IntraTransaction` | Engine entries. The constructor validates through the engine and raises `EngineValueError` with RP2's message for the same input |
| `TransactionSet`, `InputData` | Per-asset entry lists, in insertion order |
| `compute_tax` | One engine call for one asset; returns a `ComputedData` view |
| `RP2Decimal` | `EngineDecimal`, a `Decimal` subclass with the comparison rule below |
| Austrian country | `validate_input_data`, `compute_tax_for_assets`, and `classify_disposal`, backed by the engine |

The same adapter code therefore drives both backends, which is what makes the
comparison exact. Product code has no setting or environment variable for the
backend. Tests compare production with a frozen copy of the adapter running
on RP2, `tests/tax_oracle/` (see [testing](testing.md#tax-engine-backends)).
Journal entries record the backend that computed them in their calculation
metadata: `"engine": "kassiber_tax"` for the native backend, `"rp2"` for RP2.

## Decimal rules

- Every engine value is a decimal with a sign, an integer coefficient, and an
  exponent, following Python's `decimal` module (General Decimal Arithmetic).
  Exponents are part of the result: `3.0E+2` and `300` are different outputs.
- Arithmetic uses 32 significant digits with round-half-even. Exact results
  keep their ideal exponent; inexact ones are rounded to 32 digits.
- Comparisons follow RP2: `a` and `b` compare by rounding `a - b` half-even to
  13 decimal places, so values within `5E-14` are equal. `<` is "not `>=`" and
  `<=` is "not `>`".
- Kassiber pins its own Python decimal context to the same precision
  (`kassiber/__init__.py`), for every thread. Before that, the precision of
  journal processing depended on which thread first imported RP2. Importing
  RP2 also trapped `FloatOperation` on that thread; the native backend does
  not, so mixing floats and decimals no longer raises there.

## Boundary format

The binding exposes JSON-in, JSON-out functions. Decimals cross the boundary in
the canonical form `[-]<digits>E<exponent>`, for example `-12345E-3`, which
Python's `Decimal()` parses back to the identical value. `check_entry` and
`compute` carry `"schema_version": 1`; `ledger` carries `"schema_version": 2`.
`engine_version()` returns the crate version (0.2.0).

`kassiber/core/engines/native_request.py` builds these documents with the
standard library only: the canonical decimal encoding, the timestamp fields as
RP2 parses them, and the entry text. The binding releases the GIL while the
engine runs.

### `check_entry(request) -> response`

```json
{
  "schema_version": 1,
  "operation": "check_entry",
  "config": {"assets": ["BTC"], "exchanges": ["Cold"], "holders": ["Main"]},
  "entry": "<entry>"
}
```

Runs one entry through the checks of RP2's constructor in RP2's order, with
its 13-place comparisons and messages. `config` holds the configuration's
names for the membership checks; without it they are skipped. The response's
`entry` holds the values the constructor stores and derives: `unique_id`,
`notes`, `transaction_type` (the lowercase value), `spot_price`, `crypto_fee`,
`fiat_fee`, the kind's amounts (`crypto_in`, `fiat_in_no_fee`,
`fiat_in_with_fee`; `crypto_out_no_fee`, `crypto_out_with_fee`,
`fiat_out_no_fee`, `fiat_out_with_fee`; `crypto_sent`, `crypto_received`, with
`null` for other kinds), `crypto_balance_change`, `crypto_taxable_amount`,
`fiat_taxable_amount`, `is_taxable`, and `is_earn`. `is_taxable` is `null`
when evaluating it raises; RP2 evaluates it lazily.

### `compute(request) -> response`

```json
{
  "schema_version": 1,
  "operation": "compute",
  "country": {"kind": "generic", "long_term_days": 365},
  "method": "fifo",
  "assets": [{"asset": "BTC", "entries": ["<entry>", "..."]}]
}
```

`operation` is one of:

- `compute`: RP2's `compute_tax` for exactly one asset.
- `validate`: the country's `validate_input_data` over every asset of the
  run. Austria checks the `at_swap_link` pairs in RP2's order with its
  messages; other countries check nothing. Success is
  `{"schema_version": 1, "ok": true}`.
- `compute_multi`: the country's `compute_tax_for_assets` over every asset.
  Without a swap pair (and for every non-Austrian country) it answers
  `{"schema_version": 1, "ok": true, "handled": false}` and the caller runs
  `compute` per asset, as the adapter does. Otherwise it runs the Austrian
  multi-asset runner, which carries Neuvermögen swap basis into the
  incoming lots, and answers `"handled": true` with `assets` for every asset
  in request order.

`validate` and `compute_multi` take the assets in the adapter's order, each
named once (a repeated name is a `RequestError`). `country` is
`{"kind": "generic", "long_term_days": N}` or `{"kind": "at"}`, whose holding
period is `sys.maxsize`. `method` is one of `fifo`, `lifo`, `hifo`, `lofo`,
`moving_average`, and `moving_average_at`. Entries are in insertion order;
the engine re-runs their constructor checks, then reproduces `compute_tax`,
including its cross-set duplicate-row check and the
`Internal error: AVL tree has no root node` failure when no lot is visible.
Every operation and method is implemented, for both countries.

An entry is one constructor call with the arguments as given:

| Field | Meaning |
|---|---|
| `kind` | `in`, `out`, or `intra` |
| `row`, `unique_id` | The adapter's row number (RP2's `internal_id`, the entry's identity) and its transaction id; `null` stands for `None` |
| `asset`, `transaction_type`, `notes` | As given to the constructor; an `intra` entry has no `transaction_type` (it is always `MOVE`) |
| `exchange`, `holder` or `from_exchange`, `from_holder`, `to_exchange`, `to_holder` | Wallet labels and the profile label |
| Amount fields | `in`: `spot_price`, `crypto_in`, `crypto_fee`, `fiat_in_no_fee`, `fiat_in_with_fee`, `fiat_fee`. `out`: `spot_price`, `crypto_out_no_fee`, `crypto_fee`, `crypto_out_with_fee`, `fiat_out_no_fee`, `fiat_fee`. `intra`: `spot_price`, `crypto_sent`, `crypto_received`. Each canonical or `null` (not passed) |
| `ts` | `raw` (`str()` of the argument), `us` (absolute microseconds since the Unix epoch), `offset_s`, `date` and `year` in the timestamp's own offset, `vienna_date`, `display` (`str(datetime)`), and `iso` (`datetime.isoformat()`). When parsing failed, `raw`, `error` (RP2's message), and `error_class` (`TypeError` for a non-string, otherwise omitted) instead |
| `text` | `str(transaction)`, which RP2 embeds in its negative-balance and duplicate-entry messages |

A field that does not belong to the entry's kind is a `RequestError`, as is a
`ts` whose date and year disagree with its instant and offset.

A successful response lists, per asset:

| Field | Meaning |
|---|---|
| `in_transactions` | Visible lot rows, in lot-list order |
| `in_fiat_in_with_fee` | Effective acquisition basis per lot row |
| `gain_losses` | `event` (`kind`, `row`), `lot` (row or `null`), `crypto_amount`, `fiat_cost_basis`, `proceeds`, `fiat_gain`, `unit_cost_basis_override`, `long_term`, and for Austrian books (any method) either `at_category` or `at_category_error` |
| `yearly` | `year`, `transaction_type`, `long_term`, `crypto_amount`, `fiat_amount`, `fiat_cost_basis`, `fiat_gain_loss`, in RP2's order |
| `open_positions` | `row`, `sold_percentage`, `fiat_in_with_fee` |
| `balances` | `exchange`, `holder`, `final_balance`, `acquired_balance`, `sent_balance`, `received_balance`, in RP2's order |

`at_category` is the `AtDisposalCategory` value RP2's `classify_disposal`
returns for the row: `income_general`, `income_capital_yield`, `neu_gain`,
`neu_loss`, `neu_swap`, `alt_spekulation`, or `alt_taxfree`.
`at_category_error` (`class`, `message`) is the error it raises instead, for
example on conflicting `at_regime` markers of a lot that a lot method never
parsed. The Python layer raises it when the adapter classifies that row, so
the run aborts only where RP2's would.

A failure is `{"ok": false, "error": {"class": ..., "message": ...}}`. The
class is `ValueError`, `TypeError`, or `RuntimeError` for RP2's errors, which
the Python layer raises as `EngineValueError`, `EngineTypeError`, or
`EngineRuntimeError` with that message; `InvalidOperation`, `DivisionByZero`,
or `Overflow` for the `decimal` exceptions RP2's arithmetic raises (for
example a comparison of values of `1E+19` or more), with CPython's message;
`RequestError` for a malformed request (including a timestamp without the
`vienna_date` an Austrian rule needs); and `Unsupported`, reserved for a
method or operation a build does not implement (none at present). The last
two are caller or engine bugs and block the report, as a Rust panic does:
the binding turns a panic into `RuntimeError`.

### `ledger(request) -> response`

The schema-2 entry point takes the finalized projection as typed rows and
pairs (`tax-engine/core/src/ledger/request.rs`): the policy, the
configuration names, the holder, the wallets, per row its anchor, amounts in
msat, exact rate and value, normalized kind, text, Austrian overrides,
privacy-hop flag, hold, chain-order evidence, and its time twice: as RP2
parses it (`ts`) and as Kassiber's Austrian rules read it for the
Neuvermögen cutoff (`cutoff_instant`: `datetime.fromisoformat` with `Z` as
UTC and a naive time taken as UTC, `{"us": ...}`, or `{"error": ...}` with
the `ValueError` message the regime tracker raises), and the move and
cross-asset pairs. It carries facts, never marker text. Every type
rejects unknown fields and missing ones (a field without a value is `null`,
not left out; only a failed timestamp may omit `error_class`), and another
schema version is a `RequestError`.
`operation` is `ledger`, `prepare`, or `manifest`; `manifest` returns the
engine's names and tables (`manifest.rs`), which
`tests/test_tax_ledger_contract.py` compares with Python's. `ledger` returns
the journal ledger (`ledger/run.rs`, `compose.rs`): after the final prepare
pass it runs the lot engine over each asset with an inbound entry (the
country's checks, the accounting-method gate, then the multi-asset hook or
each asset's own computation) and composes acquisitions, realized entries
with their gain/loss fragments, transfer legs, holdings, and the tax summary,
as the adapter did; quarantines, the transfer audit, and the rows the generic
carry rewrote come with them. `prepare` is a diagnostic for
tests: it stops after the final prepare pass and returns each asset's
intermediate stages: the normalized stage (`ledger/normalize.rs` with the
Austrian regime tracker, `ledger/regimes.rs`: the events and moves the walk
books with their Austrian regimes, elections, and regime flows, their
order, normalize's quarantines, and the time from which the asset's lot
state is tainted) and the prepared
stage (`ledger/order.rs`, `walk.rs`, `describe.rs`: the lot engine's
entries as v1 entry documents with their composed notes and text, the
gates' quarantines, and the transfer audit with the legs' description). A
failed `prepare` also lists the stages its pass reached. Both operations
answer for a generic book without the Bitcoin rail carry and an Austrian
book without a carrying-value cross-asset pair (whose first prepare pass is
its final one), and `Unsupported` for the others until those parts are
ported. On an Austrian book `ledger` also composes each fragment's category
after the row's reviewed overrides, and runs the multi-asset swap runner
when the entries' notes pair `at_swap_link=` markers a user typed into a
row's text (design §4.2). A failure is
`{"ok": false, "error": {"stage", "asset"?, "inner": {"class", "message"}}}`;
`kassiber/core/engines/errors.py` raises what the adapter raised at that
stage, wrapped where the adapter wrapped it.

### Fixtures

`tax-engine/scripts/gen_engine_fixtures.py` runs RP2 as the oracle, building
its inputs exactly as the adapter does, and writes two files under
`tax-engine/core/tests/data/`:

- `generic_fixtures.json`: engine requests with RP2's responses for the lot
  methods: the spec scenarios, seeded random histories, and constructor
  checks.
- `at_fixtures.json`: the moving averages and Austria. Each scenario is one
  adapter run over every asset of a book, recorded as its `validate`,
  `compute_multi`, and per-asset `compute` steps with RP2's results,
  including `classify_disposal` for every Austrian row. It holds the spec
  scenarios, edge probes, and seeded random histories (swaps, pools, regimes
  and routing, Wahlrecht, fees, earn types, cutoff and Spekulationsfrist
  boundaries, and injected errors).

`tax-engine/core/tests/engine_fixtures.rs` reports every differing field by
case and path. The generator is deterministic for a seed; larger stress sets
can be checked through `KASSIBER_ENGINE_FIXTURES` and `KASSIBER_AT_FIXTURES`.
`tax-engine/core/tests/ledger_text.rs` checks the engine's own rendering of
`str(transaction)` (`ledger::text`, for the phase 4 ledger operation, which
builds its entries without a request `text`) against the RP2 text these
fixtures store.

`ledger_fixtures.json`, from `tax-engine/scripts/gen_ledger_fixtures.py`, holds
whole-ledger goldens: finalized projections with the frozen oracle's ledger,
the one production must give, the schema-2 request production's builder
makes of each, and the engine's response for each book production routes to
it ([projection goldens](testing.md#projection-goldens)).
`tax-engine/core/tests/ledger_fixtures.rs` reads every stored request into
the typed request and writes it back unchanged, replays every stored
response, and checks that `prepare` answers for the books it implements and
lists the stages a failed pass reached.
`tax-engine/core/tests/ledger_properties.rs` checks invariants of the
`ledger` operation on seeded generic and Austrian books (plan 20
Verification 1): the same request gives the same bytes; msat are conserved
per asset and per move; every book succeeds and each wallet's journal
replays to its holding without going below zero; and pricing an unpriced
inbound row (a purchase, income, or a move's in leg) quarantines no new
row, except the shortfall a released earlier sale causes, which the oracle
shares (TODO "Keep pricing from adding quarantines"). An Austrian book may
instead end in the Neu shortfall abort, rarely, and pricing may release
one, as on the oracle (TODO "Stop an Austrian Neu shortfall from aborting
the whole report"); its entries start with their regime, its moves' regime
flows conserve and match their legs' descriptions, and its categories
follow the reviewed overrides.

## Privacy

The engine performs no I/O, network access, or logging and keeps no state
between calls. The native backend does not write RP2's temporary configuration
file.
