# Journal reproducibility and replay bundles

Two ways to answer "are these numbers right?" without trusting the stored
journals on faith:

1. **Reproducibility check** (`kassiber journals verify`, daemon
   `ui.journals.verify`, assistant read tool `ui_journals_verify`): recompute
   the active book's journals from its current inputs in memory and compare
   them with the stored journals. Local, read-only, writes nothing.
2. **Replay bundle** (`kassiber diagnostics replay-bundle export|verify`,
   daemon `ui.diagnostics.replay_bundle.export`): an explicit, encrypted export
   of one journal run's pseudonymized inputs and outputs that anyone with the
   passphrase can re-run and diff with the same Kassiber, or a newer one,
   without the book, its descriptors or a network.

Both share one bounded diff format and one rule set for what counts as an
output.

## Reproducibility check

The check never writes to the book. It copies the open book into a private
in-memory SQLite database with SQLite's online backup (the same copy review
simulation uses; an encrypted book's copy gets a fresh random key), runs the
production `journals process` pipeline on that copy, and compares the copy's
journal tables with the book's. Running the real pipeline, instead of a
second implementation, is the point: the check exercises exactly the code that
wrote the stored rows.

Status values:

| `status` | Meaning |
| --- | --- |
| `identical` | Every compared row matches byte for byte (after dropping volatile columns). |
| `different` | At least one row differs; `differences` lists the first `limit`. |
| `stale` | The book's inputs changed since processing (input version, transaction count, calculation semantics), or processing now would first repair inputs (remove overlapping addresses, exclude duplicates) or price transactions from the rates cache. An overlap processing leaves for review does not make the book stale. Process first; this is not a diff. |
| `not_processed` | Journals were never processed. |
| `no_transactions` | Nothing to compare. |
| `refused` | Recomputation refused (the same refusal `journals process` would give, e.g. a custody blocker); `error` carries its code and message. |

When rows differ the check recomputes a second time on a fresh copy and
reports `deterministic: true|false`: whether two recomputations agree with
each other. `false` points at nondeterminism in the code rather than stale or
damaged stored rows.

### Compared outputs

| Table | Row key | Context reported |
| --- | --- | --- |
| `journal_entries` | entry `id` (derived from transaction, wallet, type, asset, ordinal) | transaction, wallet, year |
| `journal_tax_summary` | year, asset, transaction type, gains type | year |
| `journal_wallet_holdings` | wallet, asset | wallet |
| `journal_account_holdings` | account, asset | — |
| `journal_quantity_balances` | location kind, location id, asset | wallet when the location is one |
| `journal_quantity_postings` | `posting_id` | transaction, year |
| `journal_quantity_issues` | `issue_id` | year |
| `journal_custody_decisions` | `decision_id` | transaction, year |
| `journal_custody_economic_relations` | `relation_id` | transaction, year |
| `journal_quarantines` | transaction | transaction |

Volatile columns are dropped before comparison: `created_at`, random row `id`s
of the holdings and tax-summary tables, `workspace_id`/`profile_id` (constant
per book). Values compare by SQLite type and exact value; REAL by bit pattern,
so `0.1` and `0.1000000000000001` differ.

### Diff format

```json
{"status":"different","deterministic":true,"limit":20,"truncated":false,
 "tables":[{"table":"journal_entries","stored":6,"recomputed":6,"differing":1}],
 "by_wallet":{"wallet":1},"by_year":{"2024":1},
 "differences":[{"table":"journal_entries","change":"changed","key":"…",
   "transaction_id":"…","wallet_id":"wallet","year":"2024",
   "columns":[{"column":"cost_basis_exact","stored":"100.00","recomputed":"90.00"}]}]}
```

`change` is `changed` (same key, different columns), `missing` (stored row
the recomputation does not produce) or `unexpected` (recomputed row not
stored). `limit` defaults to 20 and is capped at 200; counts always cover every
row.

## Replay bundle

### Contents

A bundle is one file, `<name>.kassiber-replay`, an age passphrase envelope
(the same scrypt envelope and work-factor ceiling as `.kassiber` backups,
encrypted in memory; no plaintext copy is written) around a JSON document:

- `manifest`: format `kassiber.replay-bundle` version 1, the watermark text,
  creation time, app version, journal calculation semantics version, the
  database schema reference, tax country, gains algorithm, fiat currency, the
  book's journal input version, row counts, the list of included and excluded
  data classes, the reproducibility-check status at export time, the
  `stored_check` (stored figures against the recomputation), the
  pseudonymization summary (always faithful; see below), and the SHA-256 and
  byte length of each part. The manifest is generated like the parts: its
  tax settings, input version and `as_of` come from the projected inputs
  (parsed and printed again, or pseudonymized), the export time is parsed
  and printed again, and the rest are constants; nothing is copied from the
  book.
- `parts.inputs`: the custody snapshot the journal pipeline loads (profile
  settings, wallets reduced to id/kind/account and network/Samourai facts,
  transactions, rates actually used, reviewed components, gap reviews, pair
  dismissals, ownership index, evidence snapshots), projected (see below).
- `parts.outputs`: the canonical outputs computed from those inputs: journal
  entries, wallet and account holdings, quantity balances, the per-year tax
  summary and quarantines, as persistence stores them: quantities in integer
  msat; entry fiat value, unit cost, cost basis, proceeds and gain/loss in both
  their REAL and exact-text columns; the entry calculation decoded as the gain
  explanation reads it; REAL holdings and tax-summary values.
- `parts.stored`: the same tables read from the book's stored journals, with
  identifiers mapped like the inputs: the figures the user saw, including a
  wrong one under investigation.

Parts are stored as exact text so their hashes cover the bytes `verify`
reads.

### What the inputs keep

One rule: **generate, don't copy**. Nothing from the book enters the bundle as
its original bytes, except values re-serialized from a strictly parsed type.

The inputs are a projection of the custody snapshot by an explicit,
hand-written schema
([`crates/replay/src/projection.rs`](../../crates/replay/src/projection.rs)).
The snapshot loader already reduces wallet configuration to network and
Samourai facts, so descriptors, xpubs, Silent Payments material, SLIP77
blinding keys, backend URLs and credentials never enter it; BDK/LWK observer
rows, chat content, attachments and metadata records are never loaded.

Every snapshot section (profile, wallets, transactions, anchors, rate rows,
custody component headers, legs, allocations, economic terms and
commitments, gap reviews, pair dismissals, evidence snapshots, migration
issues, quarantine records, receipt sources, Lightning channel records, loan
legs, the book-network binding, observations and references, the gap
worklist) and every document held in one (`raw_json`, evidence payloads,
`evidence_json`, wallet configuration, reference payloads) has its own
field list. A field the list does not name is dropped. A named field is
generated by its class
([`typed.rs`](../../crates/replay/src/typed.rs),
[`pseudonym.rs`](../../crates/replay/src/pseudonym.rs)):

| Class | Emitted |
| --- | --- |
| Amount, integer, boolean | parsed strictly (sign, digits, fraction, exponent; JSON numbers as numbers) and printed again; anything else is replaced by a label pseudonym |
| Time | parsed as `YYYY`, `YYYY-MM-DD` or `YYYY-MM-DD[T ]HH:MM[:SS[.fraction]]` with `Z`, `±HH:MM`, `±HHMM` or no zone, range-checked and printed again; anything else (an IPv6 address, a trailing word, any non-ASCII character) is replaced |
| Identifier: local row ids, wallet and component ids, a wallet's event namespace, and every outside id (`external_id`, txids, outpoints, addresses, scripts, payment hashes, channel and node ids, asset ids, raw hex, pricing and payout references) | always a keyed pseudonym of the whole value, through one mapping so references stay consistent. Only explicitly parsed public structure survives: hex keeps its length and case; `txid:vout` keeps a canonical `vout`; a bech32/bech32m string with a valid checksum keeps an allowlisted address HRP (`bc`, `tb`, `bcrt`, Liquid and Silent Payments HRPs) or invoice HRP (`lnbc`, `lntb`, `lntbs`, `lnbcrt`, `lnsb`) with its amount; a base58check string with a valid checksum keeps its version character. Uniform hex gets no exemption; only the listed public constants (genesis hashes, Liquid policy assets, the all-zero hash) stay as they are |
| Content digest (observation, evidence and commitment hashes, fingerprints, row hashes, inventory digests) | a keyed pseudonym; the digests the engine compares are then recomputed from the projected content |
| Enum (directions, assets, chains, networks, transaction and wallet kinds, leg roles, pricing fields, Austrian categories, component states, policies, account codes other than `treasury`, …) | the constant from the list written in the schema; anything else is replaced. Every value the engine compares a field against is listed (`projection::ENGINE_CONSTANTS`, each pinned to a source file that names it and checked by `tests/projection_coverage.rs`), so replacing a value never changes the computation |
| Tax setting (`tax_country`, `gains_algorithm`, `cost_basis_pool_scope`, fiat currency; a calculation's `method` and pricing currency; quarantine reasons) | the registry constant it spells: countries, methods, pool scopes and fixed currencies come from the tax-country registry (`crates/tax-countries`), quarantine reasons from the custody stages, the tax engine (`QuarantineReason::ALL`) and the countries' refusal codes, so a country the registry learns is carried without a schema edit |
| Swap and HTLC evidence in `raw_json` (provider names, swap flows and statuses; the HTLC spend proof and its observer) | a known spelling printed again from its constant (`crates/replay/src/swap.rs`); swap ids, txids and witness items are identifiers, amounts amounts; anything else is a keyed label, which keeps equal values equal |
| Marker (`notes`) | exactly the residual markers the engine interprets: `reviewed_residual:` followed by `retained_custody`, `network_fee`, `external_payment`, `external_disposal`, `external_gift`, `external_loss` or `suspense_continuation`; anything else is authored text |
| Authored text (labels, names, descriptions, counterparties, review notes, change reasons, gap and dismissal reasons, ownership warnings) | never copied: a label pseudonym of the whole value |

`location_ref` keeps the prefix of the local references the review writes
(`network-fee:`, `reviewed-custody-suspense:`) as a constant and pseudonymizes
the id after it like any other id. Codec tags are admitted only as `$decimal`
and `$float` with a valid amount inside; any other `$…` key is dropped. The
ownership index is projected by its shape: map keys and the id in scoped
`key: [chain, network, id]` entries are identifiers, `[wallet_id,
wallet_label]` pairs pseudonymize both, match records have their own field
list, unknown sections are dropped.

`crates/replay/tests/projection_coverage.rs` keeps the schema complete: every
column of the tables the loader reads with `SELECT *` (from the current
schema fixture) and every field the loader emits for the frozen test books
must be classified, recursively into `raw_json`, evidence documents and
wallet configuration, deliberately dropped columns included. A synthetic
document carrying every field the custody interpreters read (graph and
confirmation parsers, network gate, Lightning channel interpreter, reviewed
residual reader, loader configuration facts), optional branches included,
must be fully classified even when no frozen book carries it. Every kept
marker and local reference prefix must still be read or written where the
schema says. A new column or field fails the test instead of passing
silently.
A second guard keeps the hand-written fixture from drifting: it scans the
interpreters' sources for every literal key they read (`x["key"]`,
`get(x, "key")`, `.field("key")`) and requires each to be classified or named
as a key of a structure the engine builds itself (pairs, claims, holds,
reconciliations, the finalized projection), with that list reviewed by hand.
A third requires JSON held as text (`raw_json`, evidence, configuration) to
project exactly like the same JSON value, for every class, so no class is
classified but left unimplemented on the text path.

The key is 32 random bytes generated per bundle and zeroed after export, so
pseudonyms are consistent inside one bundle (links survive) and cannot be
reversed or linked across bundles.

Chain observations that were current get new observation digests, and
reviewed custody components whose evidence matched get commitments (and
authored evidence snapshots) to the projected evidence, so reviews keep
applying.

### What the outputs keep

Outputs are generated the same way. Every output table has a closed column
schema ([`outputs.rs`](../../crates/replay/src/outputs.rs)), and the entry
`calculation` is decoded into its own closed schema (fragments, sources,
pricing; the derived `engine_event_id` is dropped). Each cell is parsed by its
class and printed again: amounts and msat integers, years and times, enums
from fixed lists (entry types, tax-summary types, gains types, location kinds
and constant locations, `treasury`), quarantine reasons and the calculation's
method and pricing currency from the registry and the tax engine, identifiers through
the same keyed mapping as the inputs. The calculation's pricing provenance
copies the transaction's fields, so it keeps the input schema's values and
otherwise the pseudonyms the inputs got; custody-gap hold locations
(`gap-capacity-hold:<digest>`, `gap-hold:<gap>:<digest>`) keep their shape
with the digest mapped. Row keys and context are rebuilt from the projected
columns. Whole-value pseudonyms (`id-…`, `label-…`) keep 128 keyed bits.

- Stored outputs (`parts.stored`) go through the same projection. A stored
  cell that does not parse as its class, or a column or calculation field
  the schema does not name, refuses the export with
  `replay_bundle_unsafe_stored`; the error names the table and column, never
  the value.
- Recomputed outputs (`parts.outputs`) may only contain identifiers the input
  projection generated; anything else refuses with
  `replay_bundle_unsupported`. `verify` runs the same closed schema over what
  it recomputes.

Both runs go through the custody, fusion, engine and render steps
`journals process` stores from (`process::render_loaded`), so the bundle's
outputs are what processing would store, for every registered country: REAL
zeros read as SQLite stores them, and quarantines of transactions outside
the profile are dropped as persistence drops them. Export reads the source,
the stored journals and the local check in one read transaction.

Export then computes the outputs twice, from the real snapshot (projected
through the mapping) and from the projected inputs. When they disagree,
ignoring derived 40+ character hex identifiers such as event ids that embed
observation digests, the export is refused with `replay_bundle_unfaithful`,
and nothing is written. A bundle therefore always reproduces the book's own
computation; a book whose numbers depend on a value the schema replaces
cannot be exported until the schema learns it.

### Consent and encryption

Export requires explicit confirmation of what the file holds: exact amounts,
dates, rates and tax settings of one book, with pseudonymized identifiers and
labels. The CLI needs `--confirm`; the desktop shows the statement in a dialog
with a passphrase and confirmation field. The bundle is always encrypted with
a passphrase of at least 8 characters. There is no unencrypted variant: exact
amounts and dates identify a person as reliably as an address, and an
unencrypted file invites posting it to a public issue tracker. Share the
passphrase over a different channel than the file.

Nothing is written except the bundle. The CLI writes `--file` (never
overwriting, owner-only permissions); the desktop daemon writes
`exports/diagnostics/replay-<time>.kassiber-replay` in the active project and
returns the path, so the renderer never names one. The CLI reads the
passphrase from `--bundle-passphrase-fd` or a no-echo prompt (twice), never
from the command line; machine and non-interactive runs must use the
descriptor. Export refuses books whose journals are not current
(`journals_stale`) so the bundle matches the figures the user sees, and it
performs no network I/O. The assistant can run the check but cannot export a
bundle.

### Verify

`kassiber diagnostics replay-bundle verify FILE` needs no book or project
(it migrates, selects and routes none). It decrypts the bundle in
memory, checks every part's SHA-256 against the manifest (a mismatch fails
with `replay_bundle_corrupt`, as do inputs the pipeline cannot read, such as
a profile without an id), recomputes the outputs from `parts.inputs` and
diffs them against `parts.outputs` with the format above (`status` is
`identical`, `different` or `refused`; `stored` in a difference is the
bundle's recorded value; `deterministic` reports a second recomputation).
A `stored` section compares `parts.stored` with the same recomputation in
the same format (derived digest identifiers masked), so the recipient sees the
discrepancy the user reported even when the bundle itself reproduces. The
report repeats the bundle's versions, tax settings, counts, export-time checks
and pseudonymization summary, so a newer Kassiber's difference can be
told from a damaged book.
It opens no book, reads no configuration and performs no network I/O.

## Implementation map

- [`crates/journals/src/reproduce.rs`](../../crates/journals/src/reproduce.rs):
  the check, the canonical tables, the diff.
- [`crates/replay`](../../crates/replay): pseudonymizer, bundle export and
  verify.
- [`crates/backup`](../../crates/backup/src/encryption.rs): the age envelope.
