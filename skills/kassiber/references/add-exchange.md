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

## Part 1 — Intake interview

Ask these in order. Record answers straight into the spec template
([docs/exchanges/TEMPLATE.md](../../../docs/exchanges/TEMPLATE.md)); copy it to
`docs/exchanges/<slug>.md` first. Keep secrets (API keys, account numbers) out
of the spec and out of chat.

### 1. Name

- Display name (e.g. "Coinfinity") and a lowercase **slug** (e.g. `coinfinity`).
- The slug is load-bearing — it becomes the `<slug>_csv` source format, the
  `<slug>` wallet kind, the `import-<slug>` CLI command, and the
  `pricing_provider` string. Pick it once; it is hard to change later.

### 2. Custodial or non-custodial?

This is the single most important question — it decides the integration shape.

- **Custodial** (the platform holds your BTC: Strike, 21bitcoin, Pocket): the
  platform's own ledger is a transaction source. Import it as an **active
  custodial ledger** (every BTC-side row becomes a Kassiber transaction).
  Withdrawals should pair with the receiving on-chain wallet so RP2 carries
  basis out of the custodial balance. Pattern to mirror: 21bitcoin / Strike.
- **Non-custodial** (you withdraw to your own wallet: most brokers like Bull,
  Coinfinity): the on-chain side is already tracked by a descriptor/xpub
  wallet, so the provider export is **order/execution evidence**, not a new
  balance source. Import it as **match-existing-only enrichment** (`relevant`
  mode) so buys gain exact pricing without duplicating the on-chain rows.
  Pattern to mirror: Bull Bitcoin / Coinfinity.
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
- **The parser must fail-safe on any unrecognized row type** — never assign a
  tax-bearing kind (`buy` / `sell` / `income` / `interest` / ...) to a row whose
  type is not in the known map. Instead:
  - if the row carries a BTC amount, import it conservatively as `deposit` /
    `withdrawal` by amount sign, add a `<slug>-unmapped-type` tag, and preserve
    the raw type value in `raw_json` so it surfaces for review;
  - if the row cannot even be safely shaped (no amount, ambiguous direction),
    raise `AppError` with the offending type in the message so the import fails
    loudly instead of dropping data silently.
  This guarantees an unseen row type can never become a *wrong* taxable event —
  the worst case is a conservative, flagged row a human resolves later.
- **Keep one obvious place to extend the map.** The `_<slug>_kind` lookup is the
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

- **CSV/file export only** → build a file importer. This is the supported,
  common path; everything below assumes it.
- **Provider has an API** → live sync is *desirable* but note the current
  reality: Kassiber's live-sync backends are `esplora`, `electrum`,
  `bitcoinrpc`, and BTCPay Greenfield only. There is **no generic exchange-API
  sync backend pattern yet**. Capture the API (auth model, endpoints, rate
  limits) in the spec as a follow-up, and ship the CSV importer first. Do not
  build a bespoke network fetcher into `importers.py` — that file is
  file-parsers only.

---

## Part 2 — Implementation checklist

Only start once `docs/exchanges/<slug>.md` is complete. The normalized record
shape every parser returns is documented under "Generic transaction imports" in
[docs/reference/imports.md](../../../docs/reference/imports.md); read it before
writing the parser. Mirror the closest existing importer (custodial → 21bitcoin
/ Strike; evidence → Bull / Coinfinity).

Touch these files, in order. Each is required for the connection to work
end-to-end and to pass the drift test.

1. **`kassiber/importers.py`** — the parser. Following the module docstring,
   add `normalize_<slug>_record`, `load_<slug>_csv_records`, and
   `is_<slug>_format`, then wire `load_<slug>_csv_records` into
   `load_import_records`. Map each row per the spec's row-type table. Raise
   `AppError` on unparseable input. Skip fiat-only rows for BTC-side custodial
   ledgers. Set `pricing_source_kind="exchange_execution"`,
   `pricing_provider="<DisplayName>"`, and `pricing_quality="exact"` only when
   the export gives an exact price.

2. **`kassiber/core/wallets.py`** — add `<slug>` to `WALLET_KINDS` and register
   its kind metadata (`config_fields: ["source_file", "source_format"]`,
   matching the other CSV-source kinds).

3. **`kassiber/daemon.py`** — add `<slug>_csv` to `_UI_WALLET_SOURCE_FORMATS`,
   and add the import dispatch branch (mirror the `strike_csv` / `21bitcoin_csv`
   block, choosing `full` vs `relevant` default per the custodial decision).

4. **`kassiber/cli/main.py`** — add the `wallets import-<slug>` subparser
   (`--workspace`, `--profile`, `--wallet`, `--file` required; add `--mode` if
   the provider uses relevant/full).

5. **`kassiber/cli/handlers.py`** — add the `import-<slug>` handler branch that
   dispatches to the import coordinator.

6. **`ui-tauri/src/lib/connectionCatalog.tsx`** — add a catalog entry. A
   `status: "ready"` entry must reference the real `walletKind` and
   `sourceFormat`, or `tests/test_connection_catalog_drift.py` fails. Add the
   `ConnectionSourceFormat` union member and an icon. Also add the English +
   German connection strings (the catalog is user-facing UI — follow
   [docs/reference/i18n.md](../../../docs/reference/i18n.md); en/de in lockstep).

7. **`tests/test_cli_smoke.py`** — extend the behavior pin: a small fixture CSV
   covering the main row types, an import, and assertions on inserted counts,
   `kind`, msat amounts, and pricing. Prefer extending this suite over new test
   files (see AGENTS.md).

8. **Docs, in the same change:**
   - `docs/reference/imports.md` — a "## <DisplayName>" section (supported-paths
     bullet, format-reference link, behavior list, CLI example).
   - `README.md` — add to the supported-imports story if it lists providers.
   - `AGENTS.md` "Known gaps" — update the importer inventory line.
   - `skills/kassiber/references/wallets-backends.md` — add the import example.

---

## Row-type mapping rules

- Valid `kind` vocabulary used downstream includes `buy`, `sell`, `deposit`,
  `withdrawal`, `receive`, `send`, and earn-like inbound kinds (`income`,
  `interest`, `staking`, `mining`, `airdrop`, `hardfork`, `wages`,
  `lending_interest`, `routing_income`) which journal processing promotes into
  RP2 earn-like receipts. Unlabeled inbound rows stay conservative acquisitions.
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
  existing importers (e.g. `_strike_kind`) fall back to passing an unknown type
  string straight through as the `kind`; for a new importer prefer mapping
  unknown types to a sign-based `deposit` / `withdrawal` plus a
  `<slug>-unmapped-type` review tag, so an unrecognized row can never silently
  acquire taxable buy/sell semantics. See "Incomplete samples" above.

---

## Verification

Run the gate and a real round-trip before calling it done:

```bash
./scripts/quality-gate.sh                                   # compile + smoke + drift + help
uv run python -m kassiber wallets import-<slug> --help       # parser wired
# round-trip on a temp data root
uv run python -m kassiber --data-root /tmp/smoke/data init
uv run python -m kassiber --data-root /tmp/smoke/data wallets import-<slug> --file docs/exchanges/samples/<slug>/example.csv
uv run python -m kassiber --data-root /tmp/smoke/data journals process
uv run python -m kassiber --data-root /tmp/smoke/data --machine reports summary
```

For `ui-tauri/` catalog/i18n changes also run, from `ui-tauri/`:

```bash
pnpm typecheck && pnpm test --run && pnpm lint
```

Confirm: every spec row type is mapped/skipped/quarantined, exact pricing only
where the export is exact, withdrawals pair instead of disposing, and the
connection appears in `wallets kinds` and the desktop Add Connection modal.
