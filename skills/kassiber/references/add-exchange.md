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

- **No-code path (works today, no PR).** If the user can reshape their export
  into Kassiber's generic columns (the field list under "Generic transaction
  imports" in [docs/reference/imports.md](../../../docs/reference/imports.md)),
  `kassiber wallets import-csv` / `import-json` imports it now — no parser, no
  catalog entry, no merge. This is the right answer for a one-off or personal
  book. Build a dedicated `import-<slug>` importer only when you want
  *repeatable* imports of the raw provider export, exact execution pricing
  applied automatically, and a shareable connection in the desktop modal.
- **Scope: Kassiber is the BTC-side subledger.** Only `BTC` / `LBTC` rows are
  imported. On a multi-asset exchange (ETH, USDT, altcoins), the non-BTC legs
  are **out of scope** — skip them, or keep them as excluded evidence; never
  model a full multi-asset ledger here. A BTC↔fiat trade is in scope; a
  BTC↔ETH trade is a disposal on the BTC side only and needs care (often
  quarantine). Capture this in the spec.
- **Export format.** CSV is the supported shape. **XLSX** → have the user
  save-as / export the relevant sheet to CSV; the bundled `XlsxWriter` is
  **write-only** and there is no XLSX *reader* in the project, so reading `.xlsx`
  would mean a new reader dependency (e.g. `openpyxl`) — get owner approval
  before reaching for that, and prefer Save-As-CSV. **JSON** → use the generic
  `import-json` path or a JSON parser. **PDF statements are not
  machine-importable** — ask for a CSV/XLSX/API export instead; do not scrape a
  PDF.

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
  Withdrawals should pair with the receiving on-chain wallet so RP2 carries
  basis out of the custodial balance. Pattern to mirror: 21bitcoin / Strike.
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
writing the parser. Mirror the closest existing importer — read its
`normalize_*_record` in `kassiber/importers.py` and its `## ` section in
`docs/reference/imports.md` as worked examples:

| Provider | Custodial model | Default mode | Mirror for... |
|---|---|---|---|
| `strike` | custodial wallet + exchange | `full`, skip fiat-only | Lightning + on-chain rows, provider-scoped ids |
| `21bitcoin` | custodial ledger | `full`/`relevant` | withdrawals that pair out to a receiving wallet |
| `pocketbitcoin` | non-custodial broker | `relevant` | matching when the export has no on-chain txid |
| `bullbitcoin` / `coinfinity` | non-custodial broker | `relevant` | order/execution evidence enriching existing rows |

Touch these files, in order. Each is required for the connection to work
end-to-end and to pass the drift test.

1. **`kassiber/importers.py`** — the parser. Following the module docstring,
   add `normalize_<slug>_record`, `load_<slug>_csv_records`, and
   `is_<slug>_format`, then wire `load_<slug>_csv_records` into
   `load_import_records`. Map each row per the spec's row-type table. Raise
   `AppError` on unparseable input. Skip fiat-only rows for BTC-side custodial
   ledgers. Set `pricing_source_kind="exchange_execution"`,
   `pricing_provider="<DisplayName>"`, and `pricing_quality="exact"` only when
   the export gives an exact price. Give every row a **stable** `txid` — the
   on-chain hash when present, else a provider-scoped id (`<slug>:<ref>`) from a
   stable column — so re-importing an updated export dedupes instead of
   duplicating rows.

2. **`kassiber/core/wallets.py`** — add `<slug>` to `WALLET_KINDS` and register
   its kind metadata (`config_fields: ["source_file", "source_format"]`,
   matching the other CSV-source kinds).

3. **`kassiber/daemon.py`** — add `<slug>_csv` to `_UI_WALLET_SOURCE_FORMATS`,
   and add the daemon import dispatch branch (mirror the `strike_csv` /
   `21bitcoin_csv` block, choosing `full` vs `relevant` default per the
   custodial decision).

4. **`kassiber/cli/main.py`** — two edits in this one file. Add the
   `wallets import-<slug>` **subparser** (`--workspace`, `--profile`,
   `--wallet`; `--file` required; add `--mode` if the provider uses
   relevant/full), and add the matching **dispatch branch** in the wallets
   command handler that calls `import_into_wallet(conn, args.workspace,
   args.profile, args.wallet, args.file, "<slug>_csv", <mode>)`. Mirror the
   existing `import-strike` / `import-21bitcoin` blocks — both the parser and
   the dispatch live in `main.py`; there is no separate `handlers.py` branch for
   imports.

5. **`ui-tauri/src/assets/integrations/<slug>.svg`** — the connection logo.
   Prefer a **vector** (`.svg`): it stays crisp at every size and the existing
   assets are almost all tiny SVGs (~0.3–3 KB). Source the official brand mark
   from the provider's press/brand kit or site, optimize it (e.g. SVGO; strip
   width/height so `viewBox` drives scaling), and commit it under this name.
   Only fall back to a trimmed, transparent `.png`/`.jpg` (like `strike.jpg` /
   `21bitcoin.png`) when no usable vector exists. If you cannot get a clean logo
   at all, use the generated `sourceIcon("XX", "#bg", "#fg")` monogram in the
   catalog entry instead of shipping a blurry raster. Keep it square-ish so it
   fits the 32 px tile; respect the provider's trademark usage terms.

6. **`ui-tauri/src/lib/connectionCatalog.tsx`** — add the catalog entry so it
   shows in the desktop Add Connection modal. Steps:
   - `import <slug>Icon from "@/assets/integrations/<slug>.svg";` at the top.
   - Add the `<slug>_csv` member to the `ConnectionSourceFormat` union.
   - Add a `CONNECTION_SOURCES` entry mirroring `strike`: `id`, `title`,
     `description`, `category: "exchanges"`, `image: <slug>Icon`,
     `imageClassName: "size-8 rounded-lg"`, `status: "ready"`, `pathLabel`,
     `formatLabel: "<slug>_csv"`, `docsHref`, `setupKind: "file-wallet"`,
     `walletKind: "<slug>"`, `sourceFormat: "<slug>_csv"`, and a `details: []`
     bullet list.
   - **Sizing/fit:** `imageClassName` controls the tile render. If the logo
     needs a light backing in dark mode (dark marks on transparency), add
     `imageFrameClassName: lightLogoFrame` like the other dark-on-light logos so
     it doesn't disappear. Check it renders cleanly in both themes.
   - A `status: "ready"` entry **must** reference the real `walletKind` and
     `sourceFormat` or `tests/test_connection_catalog_drift.py` fails.
   - Catalog `title` / `description` / `details` are inline English literals
     here (not `t()` keys). Only if you add keys to `connections.json` (e.g. a
     format label or import-mode helper) must you add them to **both** `en` and
     `de`, or the i18n key-parity test fails — see
     [docs/reference/i18n.md](../../../docs/reference/i18n.md).

7. **`tests/test_cli_smoke.py`** — extend the behavior pin: a small fixture CSV
   covering the main row types (and at least one unrecognized type to prove the
   fail-safe fallback), an import, and assertions on inserted counts, `kind`,
   msat amounts, and pricing. Prefer extending this suite over new test files
   (see AGENTS.md). Also add a `wallets import-<slug> --help` line to the smoke
   block in `scripts/quality-gate.sh` and the verification list in `AGENTS.md`.

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

---

## Part 3 — Contribute it back (open a PR)

A finished importer (or even just a finished spec) is worth sharing so the next
person with that exchange gets it for free. Offer to open a PR — there are two
shapes depending on who did the work:

- **Implemented importer → full PR.** Commit on a feature branch in small,
  reviewable slices (parser+test, then wiring, then catalog+logo, then docs —
  see the commit guidance in `AGENTS.md`), make sure
  `./scripts/quality-gate.sh` is green (plus the `ui-tauri/` checks for catalog
  changes), and open the PR. Include the spec `docs/exchanges/<slug>.md`, a
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
