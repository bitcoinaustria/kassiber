# Adding an Exchange

Repeatable playbook for onboarding a **new exchange / broker / custodial
platform** into Kassiber. Use it when the user wants Kassiber to understand
exports from a provider it does not support yet (the supported set is listed in
[docs/reference/imports.md](../../../docs/reference/imports.md)).

This is a two-part flow:

1. **Intake** — a fixed interview the *user* can complete. It captures the
   facts and sample files needed to build a correct importer, and writes them
   to a tracked spec under `docs/exchanges/<slug>.md`.
2. **Implementation** — the agent turns a completed spec into a real importer
   by touching a fixed list of files, then verifies.

The point of the split is reliability: the intake makes sure no decision is
guessed, and the implementation checklist makes sure no touchpoint is missed.

> Do not start implementation from a half-filled spec — missing *answers*
> (custodial model, tax treatment, slug) are a hard blocker, so stop and ask.
> A sample that does not cover every row type is **not** a blocker: it is the
> normal case (see "Incomplete samples" below). What is non-negotiable is that
> the importer never *guesses* the tax semantics of a row it doesn't recognize.

---

## Before you build: do you need code at all?

Two things to settle first — they decide whether this is a five-minute job or a
code change.

- **No-code path (works today, no PR). Try this first — do not ask the user to
  retype or reshape anything.** The generic ledger importer reads an export *as
  the platform wrote it* by mapping its column names:

  ```bash
  kassiber wallets analyze-file --file export.csv   # headers, inferred plan, dry-run; no DB, no network, no AI
  kassiber wallets import-ledger --wallet <wallet> --file export.csv \
    --column-map '{"date":"Trade Date","type":"Action","amount":"Qty","fiat_value":"Total"}'
  ```

  Work the plan out from the headers `analyze-file` returns and re-run it until
  `next_step.action` is `import`. Add `type_map` when the file labels rows in
  another language or house style (`{"type_map": {"ACQ-MKT": "Buy"}}`); German
  column names and values are already recognized. **Check
  `row_kinds_from_amount_sign`** — true means no Type or direction column was
  recognized, so every row would import as a plain transfer picked by the
  amount's sign and a sale would book as a deposit with nothing rejected.
  You are mapping names, never values: every amount, date, and asset is still
  read from the file by the same normalizer, so a wrong mapping yields a
  rejected row rather than a wrong number. Full reference:
  ["Mapping the columns yourself"](../../../docs/reference/imports.md#mapping-the-columns-yourself---column-map).

  Each import is recorded as a run, so a wrong plan is recoverable:
  `kassiber imports list` then `kassiber imports rollback --batch <id> --confirm`
  removes exactly the transactions that run created.

  `wallets import-csv` / `import-json` remain the path for data the user is
  generating themselves rather than exporting. Build a dedicated `import-<slug>`
  importer only when you want *repeatable* imports of the raw provider export,
  exact execution pricing applied automatically, and a shareable connection in
  the desktop modal.
- **Asset scope.** Bitcoin (`BTC`, `LBTC`) is always imported. Other crypto
  assets are imported only into a wallet whose `asset_scope` is `crypto`
  ([plan 24](../../../docs/plan/24-crypto-assets.md)); stocks, ETFs, metals
  and indices never are. For an exchange the scope is a **filter** over the
  same events ([exchange contract](../../../docs/reference/exchange-contract.md)):
  a BTC row reads the same in both scopes, an event without Bitcoin is counted
  (`assets.skipped`, or `assets.unread` from an importer that reads only BTC)
  before its type is read, a BTC↔other-coin trade books as two paired `swap`
  legs in the `crypto` scope and is refused with the scope command in the
  `bitcoin` scope. Capture which assets the provider's files and API show.
- **Refusals.** Refuse ambiguity by name instead of solving it with machinery:
  an unknown type, an unstated fee, an amount finer than the book stores, a
  margin or derivatives row, two trades the export cannot tell apart. The whole
  import stops, naming the row, and nothing is stored.
- **Export format.** CSV is the supported shape for a dedicated importer.
  **XLSX/XLSM** → readable as-is on the generic-ledger path (`import-ledger`,
  native Rust reader); a *dedicated* importer still parses CSV, so ask for
  Save-As-CSV when you are writing one. **JSON** → use the generic `import-json` path or a
  JSON parser. **PDF/photo statements** → not parseable as data, but importable
  through the local-AI OCR draft path (`wallets preview-document` /
  `import-document`), which is loopback-only and quarantines by per-cell
  confidence. Never scrape a PDF by hand into numbers.

---

## Part 1 — Intake interview

Ask these in order. Record answers straight into the spec template
([docs/exchanges/TEMPLATE.md](../../../docs/exchanges/TEMPLATE.md)); copy it to
`docs/exchanges/<slug>.md` first. Keep secrets (API keys, account numbers) out
of the spec and out of chat.

### 1. Name and logo

- Display name (e.g. "Coinfinity") and a lowercase **slug** (e.g. `coinfinity`).
- The slug is load-bearing — it becomes the `<slug>_csv` source format, the
  `<slug>` wallet kind, the `import-<slug>` CLI command, and the
  `pricing_provider` string. Pick it once; it is hard to change later.
- **Logo:** find the provider's official brand mark, preferring a **vector
  (SVG)** so the connection tile stays crisp at any size (see implementation
  step 5). Look it up from their press/brand kit or site; record the source URL
  in the spec. If no clean logo is available, the catalog falls back to a
  generated monogram. Respect the provider's trademark usage terms.

### 2. Custodial or non-custodial?

This is the single most important question — it decides the integration shape.

- **Custodial** (the platform holds your BTC: Strike, 21bitcoin, Pocket): the
  platform's own ledger is a transaction source. Import it as an **active
  custodial ledger** (every BTC-side row becomes a Kassiber transaction).
  Withdrawals should pair with the receiving on-chain wallet so the tax
  engine carries basis out of the custodial balance. Pattern to mirror:
  21bitcoin / Strike.
- **Non-custodial** (you withdraw to your own wallet: most brokers like Bull,
  Coinfinity): the on-chain side is already tracked by a descriptor/xpub
  wallet, so the provider export is **order/execution evidence**, not a new
  balance source. Import it as **match-existing-only enrichment** (`relevant`
  mode) so buys gain exact pricing without duplicating the on-chain rows.
  Pattern to mirror: Bull Bitcoin / Coinfinity. Evidence imports only enrich
  rows that already exist, so the on-chain wallet must be synced **before** the
  evidence import or there is nothing to match.
- **Both** (Strike-style apps used as wallet *and* exchange): import the
  platform ledger in `full` mode but skip fiat-only rows, and let withdrawals
  pair with external wallets. Pattern to mirror: Strike.

> If non-custodial and the export carries no fiat execution prices at all, there
> may be nothing to build — the descriptor wallet already covers it. Confirm the
> export adds something (exact prices, fees, fiat legs) before writing code.

### 3. Tax-easy for Austria?

Capture, do not assume:

- Does the export carry **exact execution price, cost basis, and fees** per
  trade? If yes, those rows become exact `exchange_execution` pricing (no
  quarantine). If it only gives coarse/daily prices, that pricing is stored
  with provenance but **quarantined for review**, not treated as exact FMV.
- Does the provider withhold/report **Austrian KESt** (domestic-provider
  withholding)? Kassiber does **not** model withheld KESt metadata yet — record
  it in the spec's "Austrian notes" and surface it as a known gap; do not invent
  a column for it.
- Are there row types with **under-specified tax semantics** — transfers without
  prices, rewards/interest/income, cross-asset swaps? Those must **quarantine**
  at journal normalization, never be guessed into a zero-basis disposal. List
  each one in the row-type table.

### 4. Example reports with all row types

Ask the user for **real sample exports that exercise every row type the
provider can emit** — buy, sell, deposit, withdrawal, fee, reward/interest,
swap, reversal/cancel, Lightning vs on-chain, fiat-only, etc. One export rarely
covers them all; ask for several or a documentation list of row/type values.

- Save samples under `docs/exchanges/samples/<slug>/` **only if they are
  scrubbed** of personal data, or keep them out of the repo and reference their
  shape. Never commit account numbers, names, or balances.
- Fill the **row-type table** in the spec: one line per distinct
  `Transaction Type` (or equivalent) value, its meaning, the Kassiber `kind` and
  `direction` it maps to, whether it is imported / skipped / quarantined, and
  its **source** — `sample` (a real row exists) or `docs` (listed in the
  provider's vocabulary but not in any sample yet).
- **Enumerate from the documentation, not the sample.** The sample proves the
  column layout and parsing; the provider's docs give the full set of type
  values. A user's own history almost never exercises every row type, so the
  sample being incomplete is expected — fill the rest of the table from the
  docs and mark those rows `docs`.

### Incomplete samples

This is the normal situation, not a failure. Handle it like this:

- **Cover the table from documentation** so every documented type has a
  decision, even types the sample never hit. If neither sample nor docs pin a
  type's meaning down, record the open question and make the parser fail-safe on
  it (below) rather than blocking the whole importer.
- **The parser must fail closed on any unrecognized row type** — never assign
  a tax-bearing kind (`buy` / `sell` / `income` / `interest` / ...) to a row
  whose type is not in the known map. An exchange importer refuses the import,
  naming the row and its type; because the scope filter runs first, an
  unknown type in a coin the wallet does not keep is only counted and never
  stops a Bitcoin wallet. This guarantees an unseen row type can never become
  a *wrong* taxable event.
- **Keep one obvious place to extend the map.** The exchange's kind table
  (one `match` from the provider's type to the contract's `Kind`) is the
  single source of truth for type→kind; when a new type later shows up, mapping
  it is a one-line change plus a spec table row. Do not scatter type handling
  across the parser.
- Note in the spec which row types are still `docs`-only / unverified so a later
  real sample can confirm them.

### 5. Documentation

- Get the provider's export-format documentation (column meanings, type
  vocabulary, timezone, decimal/locale, fee columns). Look it up if the user
  cannot supply it. Record the URL in the spec and, on implementation, add it to
  the "Format references" list in
  [docs/reference/imports.md](../../../docs/reference/imports.md).
- Note timezone (assume UTC only if documented), number locale (comma vs dot
  decimals), and whether amounts are signed.

### 6. API connection?

- **CSV/file export only** → an export parser on the exchange contract (or,
  for a non-custodial broker whose rows only enrich existing ones, an evidence
  importer like `pocket.rs`).
- **Provider has an API** → an API transport on the same contract, built
  together with the export parser: both read into one event shape and share the
  mapping, so a transaction in both gets one record and id. Only a read-only
  key the user creates in their own normal account (no partner programs, OAuth
  apps or institutional tiers); record how to create it and which permissions
  to tick, and refuse a key the API shows can trade or withdraw where the API
  tells. Every request runs only after the user's explicit sync, under its
  consent; see [privacy and security](../../../docs/reference/privacy-and-security.md).

---

## Part 2 — Implementation checklist

Only start once `docs/exchanges/<slug>.md` is complete. Every exchange works the
same way; only its API and export formats differ. Read the
[exchange contract](../../../docs/reference/exchange-contract.md) and mirror the
closest exchange in `crates/wallets/src/file_import/exchange_api/`: Bitvavo (API +
CSV, one event shape), Bitstamp (API + CSV), Bitfinex and Bitcoin.de (BTC-only
readers with cross-checks), Binance's `history.rs` (an export without ids).

1. **The exchange module** — `exchange_api/<slug>.rs` for the API transport
   (signed reads through `Client`, paging and limits, the key check `probe`,
   the provider's own BTC balance as a `Balance` where it states one) and
   `exchange_api/<slug>/export.rs` for each file export. Both read rows into
   `Event`s and hand them to the `Mapper`: `admit` with the event's assets
   first, then `push` the shaped events (`Movement`, `Trade` with exact
   `Pricing` only when the provider states it, `Swap(reference)` legs,
   third-asset `Fee` rows). Keep the kind table in the module. Give every
   record a stable identity — `<slug>:<provider id>`, the on-chain txid kept
   in `raw_json` for transfer pairing, never as the identity — the same in the
   API and the export.

2. **The registry** — one `Descriptor` (`EXCHANGE`) in the module and its entry
   in `exchange_api/contract/registry.rs`: kind, provider name, which assets it
   reads (`AllAssets` or `BitcoinOnly`), default URL, key-only or key and
   secret, `fetch`/`probe`, its `FileFormat`s, `shared_ids`, same-second tags
   and any hooks. The sync, the desktop key check and the file coordinator read
   only the registry. Add the kind to `exchange_api::KINDS`, a `Format` variant
   in `crates/wallets/src/file_import.rs` (`from_source_format`, `name`, the
   registry arm in `load`), the wallet kind (`config.rs`,
   `connections_catalog.json`, `completeness.rs`), the source format
   (`connections.rs`, `sync/sources.rs`), `crates/setup/src/backend_runtime.rs`
   and the daemon's `onboarding.rs` arm.

3. **The CLI** — `wallets sync-<slug>` and `wallets import-<slug>` parsers
   appended to `crates/cli/src/spec_data.rs` (the parser count and the
   `wallets` subparser names), `RUST_ONLY_LEAVES` in `spec.rs`, the dispatch
   arms in `crates/cli/src/wallets.rs`, `IMPLEMENTED` in `describe.rs`, the
   capability in `crates/runtime/src/capabilities.rs`, and the Rust-only
   sibling entries in `crates/cli/tests/describe_drift.rs` and
   `crates/runtime/tests/capabilities.rs`.

4. **`assets/logos/integrations/<slug>.svg`** — the connection logo. Prefer a
   vector from the provider's brand kit (SVGO, `viewBox` only), square-ish for
   the 32 px tile; respect the trademark terms.

5. **The desktop (Rune)** — the catalog sources in
   `crates/kassiber-desktop/src/overview/catalog.rs` and their setups in
   `crates/kassiber-desktop/src/wallets/add/meta.rs`; for an API the exchange
   model in `wallets/add/exchange/model.rs` (kind, key guide with the
   permissions to grant and deny, secret label); logo bounds in `logos.rs`;
   `en` and `de` strings in `assets/i18n/locales/*/connections.json` (key
   parity is tested). `crates/daemon/tests/desktop_boundaries.rs` checks the
   exchange list.

6. **Tests** — the API over the loopback fake (`exchange_api/tests/<slug>_api.rs`:
   documented response shapes, signing, paging, refused keys, consent and
   offline make zero requests, echoed credentials withheld, the balance); the
   export in `crates/wallets/src/file_import/tests.rs` or its own test module
   (every documented row type, an unknown type refused, both scopes); an
   API-then-export import into one wallet without duplicates; a mutation entry
   in `crates/wallets/scripts/exchange_mutations.json` per rule, killed by a
   named test.

7. **Docs, in the same change** — `docs/reference/<slug>.md` (requests, key
   steps and permissions, mapping, refusals, balance scope, export format),
   `docs/reference/imports.md`, `docs/reference/rust-exchange-sync.md`,
   `docs/reference/exchange-desktop.md`, the egress list in
   `docs/reference/privacy-and-security.md`,
   `crates/ai/skill_references/wallets-backends.md`, and `TODO.md`. Keep
   importer inventories out of `README.md` and `AGENTS.md`.

An evidence broker (non-custodial, its rows only enrich existing ones) keeps
the older file-importer pattern: a parser in `crates/wallets/src/file_import/`
mirroring `pocket.rs` or `coinfinity.rs`, listed in `evidence()` and
`EVIDENCE_FORMATS`.
A non-custodial escrow keeps its own importer too: Bisq's
`crates/wallets/src/file_import/bisq.rs` books per-trade escrow legs (`full`
mode) that pair with the user's on-chain wallet, with a second export for the
txids and dates.

---

## Row-type mapping rules

- Valid `kind` vocabulary used downstream includes `buy`, `sell`, `deposit`,
  `withdrawal`, `receive`, `send`, and earn-like inbound kinds (`income`,
  `interest`, `staking`, `mining`, `airdrop`, `hardfork`, `lending_interest`,
  `routing_income`) which journal processing promotes into earn-like receipts.
  A valued `wages` row keeps its kind as provenance but processes as an
  ordinary acquisition with its reviewed value as basis. Unlabeled inbound
  rows stay conservative acquisitions.
- `buy` cost basis **includes** fiat fees; `sell` proceeds are **reduced** by
  fiat fees (mirror 21bitcoin / Coinfinity).
- Withdrawals from a custodial wallet are **not** disposals — emit a
  `withdrawal` with the BTC fee and let `transfers pair` carry basis to the
  receiving wallet. Do not invent a sell price for them.
- Lightning rows: derive `payment_hash` from a valid 64-hex hash/preimage when
  present, and use a provider-scoped `txid` (`<slug>:<ref>`) when there is no
  on-chain hash, so swap matching still works.
- Anything whose tax treatment the export does not pin down → leave it to
  **quarantine**, with an actionable hint. Never zero-basis-guess.
- The type→kind fallback must be **conservative, not pass-through**. Some
  existing importers (e.g. `kind` in `strike.rs`) fall back to passing an unknown type
  string straight through as the `kind`; for a new importer prefer mapping
  unknown types to a sign-based `deposit` / `withdrawal` plus a
  `<slug>-unmapped-type` review tag, so an unrecognized row can never silently
  acquire taxable buy/sell semantics. See "Incomplete samples" above.

---

## Verification

Run the gate and a real round-trip before calling it done:

```bash
./scripts/quality-gate.sh                                   # full Rust + frontend gate
cargo build --locked -p kassiber
K=target/debug/kassiber
$K wallets import-<slug> --help                             # parser wired
# round-trip on a disposable data root
SMOKE="$(mktemp -d)"
$K --data-root "$SMOKE/data" init
$K --data-root "$SMOKE/data" wallets import-<slug> --file docs/exchanges/samples/<slug>/example.csv
$K --data-root "$SMOKE/data" journals process
$K --data-root "$SMOKE/data" --machine reports summary
```

For focused catalog/i18n checks while developing, use the frontend commands in
[CONTRIBUTING.md](../../../CONTRIBUTING.md#verification-and-review). The full
gate already includes them.

Confirm: every spec row type is mapped/skipped/quarantined, exact pricing only
where the export is exact, withdrawals pair instead of disposing, and the
connection appears in `wallets kinds` and the desktop Add Connection modal.

---

## Part 3 — Contribute it back (open a PR)

A finished importer (or even just a finished spec) is worth sharing so the next
person with that exchange gets it for free. Offer to open a PR — there are two
shapes depending on who did the work:

- **Implemented importer → full PR.** Commit in small,
  reviewable slices (parser+test, then wiring, then catalog+logo, then docs —
  see the commit guidance in `AGENTS.md`), make sure
  `./scripts/quality-gate.sh` is green, and open the PR. Include the spec `docs/exchanges/<slug>.md`, a
  scrubbed sample (or note why none is committed), and the verification you ran.
- **Spec only (the user can't code) → spec PR or issue.** The intake is the hard
  part and it is done — do not let it evaporate. Open a PR that adds just
  `docs/exchanges/<slug>.md` (or file an issue with the spec inline) so a
  maintainer or a later agent can implement Part 2 from a complete spec. Title
  it so it's findable, e.g. "Exchange spec: <DisplayName>".

Only open a PR when the user asks for one (per repo policy, do not create PRs
unprompted) — but do *suggest* it, since a tracked spec is the unit other people
build on. Keep secrets and personal data out of the branch, the PR body, and any
committed sample.
