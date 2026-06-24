# 12 — Bitcoin-backed lending (goal state + implementation design)

Status: **design / not yet built.** A `docs/plan/` goal-state and guardrail doc, not
a task list — the backlog lives in [TODO.md](../../TODO.md). It supersedes the earlier
"escrow-as-encumbered-account" sketch: that model is **rejected** here (see
[§4](#4-architecture-encumbrance-is-a-lot-tag-not-an-account)) because it re-introduces
the per-`(exchange, holder)` "balance went negative" crash class — the same failure that
prompted this work.

Researched and red-teamed across nine lending products (Firefish, Hodl Hodl Lend,
Unchained, Debifi, Lygos, Lendasat, Ledn, Nexo/SALT/Strike/Xapo/Wirex, Coinbase/Morpho).
Mechanics below are verified against primary sources where stated; tax treatment is the
Austrian model established in this branch — advisory, **not** BMF-confirmed.

## 1. Why

A self-custody user funded a friend's multisig for one signing round; the coins returned
(a round-trip). Kassiber booked the outflow as a `SELL`, RP2's per-account `BalanceSet`
went negative, and the whole report aborted. That round-trip is the simplest member of a
family Kassiber doesn't model: **Bitcoin-backed loans**. A collateral lock has the same
shape — BTC leaves a wallet, sits somewhere the user still economically owns, and either
returns (repaid) or is seized (liquidated). Modelling loans subsumes the round-trip.

## 2. Recommended approach — Hybrid

Three approaches were weighed: generic + manual entry; per-provider adapters with
detection; and a **hybrid** (generic loans core + provider *presets* + progressive-disclosure
UX). The hybrid wins, for one decisive reason grounded in the research:

> **The chain carries no loan semantics for any of the nine providers.** Firefish escrows
> are Taproot-with-NUMS (indistinguishable from an ordinary key-path spend until spent);
> Hodl Hodl/Debifi are generic `sortedmulti`; custodial is opaque omnibus. So *correctness
> is provider-agnostic* and lives entirely in the generic core. Per-provider **code** buys
> little (the chain can't feed it) and rots when a provider flips custody.

So: a generic facility + legs core (provider-agnostic, works on day one for an unknown
platform), plus **presets as pure data** (a provider's known custody type, lifecycle
template, import hint) that *seed editable suggestions and never silently decide tax
treatment*. Grafts from the adapter approach: a thin **import-tier registry** (new, in
`kassiber/core/loans/` — modeled on the Lightning adapter registry, **not** reusing it) for
the one-click import paths that are genuinely feasible, each gated behind a field-presence
check so a drifted export degrades to manual rather than mis-mapping.

Honest trade-off: presets are maintenance debt (a custody flip makes a preset stale). But a
stale preset produces a *wrong default the user can override*, never a wrong silent booking;
every facility is dated (`as_of_custody_date`) so a loan keeps its original treatment across
a flip. A stale *adapter* (the rejected approach) produces silent mis-mapping — not
acceptable.

## 3. Tax treatment (Austria) — driven by two orthogonal linchpins

Established in this branch; advisory, **not** BMF-confirmed (a binding answer needs a
*verbindliche Auskunft*, §118 BAO). The research sharpened it in two ways:

- **Custody and rehypothecation are independent.** `custody_type` answers *who holds keys*;
  `rehypothecation` answers *can the lock be contested as a disposal*. Ledn "Custodied" is
  custodial but `rehypothecation=none` (soft flag); legacy Ledn "Standard" is custodial with
  `rehypothecation=allowed` (strong flag). Re-lending, not key-count, drives the contested
  branch — **and even that is legally unconfirmed** (GMLaw and others decline to hold that
  rehypothecation converts a pledge into a disposition). So contested-disposal is an
  **advisory flag, never a hardcoded default**, or Kassiber would systematically over-report.
- **Lender interest defaults to the progressive tariff (≤55 %), not 27.5 %.** Only publicly
  offered lending gets the 27.5 % special rate (§27a Abs 2, "nicht öffentlich angeboten"
  carve-out). Per-deal `public_offering` flag; the fork must carry its own primary-source
  citation before it ships, and never auto-applies 27.5 % from a preset guess.

| Lifecycle event | Treatment | Driver |
|---|---|---|
| Collateral lock — non-custodial (live key) / pre-signed / collaborative | **Not a disposal** — coins stay in the owned pool, flagged encumbered | `custody_type` |
| Collateral lock — custodial segregated | Not a disposal **+ soft review flag** | `custody_type` |
| Collateral lock — custodial rehypothecated | Not a disposal **+ strong contested flag**; never auto-realized | `rehypothecation=allowed` |
| Principal draw (fiat/USDC) | Not income | role |
| Interest paid — fiat/USDC | Non-event (private borrower: non-deductible) | `interest_asset` |
| Interest paid — **in BTC** | **Disposal** of those sats | `interest_asset=BTC` |
| Repayment + collateral release | Non-event, basis **and acquisition date** carry | role |
| Default / liquidation | **The disposal at FMV** (27.5 % if Neuvermögen); fees = disposal costs | role |
| Liquidation surplus return | Partial non-event; exact-cover split so only the debt-settling portion is the SELL | role |
| Lender interest received | **Progressive ≤55 % by default**; 27.5 % only if `public_offering` confirmed | `public_offering` |

## 4. Architecture — encumbrance is a lot *tag*, not an account

**Rejected:** modelling the escrow as a synthetic RP2 *exchange/account*. RP2's availability
gate is per-`(exchange, holder)` (the [rp2_per_account_balance_gate](../../kassiber/core/engines/rp2.py)
mechanism). On liquidation the entire collateral leaves the synthetic account in one SELL and
a surplus returns; a transient over-draw (fee output, CPFP anchor, multi-UTXO partial
liquidation) makes that synthetic account go negative and **aborts the whole report** — the
#213 crash class, re-introduced. A separate balance-bearing account is the wrong primitive.

**Adopted:** collateral stays in the **global per-asset pool** (`resolve_pool_id`,
[austrian.py:240](../../kassiber/core/austrian.py), is global by design); encumbrance is a
**partition tag / annotation on the lot**, not a balance. Concretely:

- **Lock** = an explicit loan-lock regime in the classification step that **suppresses the
  outbound→disposal branch** ([tax_events.py:896](../../kassiber/core/tax_events.py), the
  `elif direction == "outbound":` branch) and tags the lot `encumbered`. The coins are still
  owned (no disposal, no MOVE to a separate account), so no per-account balance can go
  negative. *(Note: "mint a MOVE by adding a `loan-leg:` prefix to `_SYNTHETIC_ID_PREFIXES`"
  does not work — that tuple is a `.startswith()` skip-guard at
  [ownership_transfers.py:73](../../kassiber/core/ownership_transfers.py), consumed at
  133/144/396, not a minting hook. And a Kassiber MOVE requires a paired inbound leg, which a
  one-legged lock has not got — the release may be months away or never come.)*
- **Release** = clear the `encumbered` tag. Non-event. The lot retains basis **and original
  acquisition date** (Alt/Neu by date, not a 1-year hold) — this must be covered by a test:
  lock-then-release of pre-2021 Altvermögen re-emerges as Altvermögen.
- **Liquidation** = book the `SELL` at FMV on the real liquidation transaction. The one
  disposal. Surplus return splits via the existing exact-cover allocation so only the
  debt-settling portion is the SELL.

This keeps the per-account gate honest, needs no synthetic counter-leg, and reuses the global
pool and exact-cover machinery already in place.

## 5. Data model

Two new tables (siblings of `transaction_pairs`/`direct_swap_payouts` in `db.py`), plus one
advanced-only. Established conventions: embedded `SCHEMA`, `workspace_id`/`profile_id` FKs
with CASCADE, `deleted_at` soft-delete, `created_at`.

**`loans`** (facility, one row per loan): `role` (borrower|lender), `platform` (free text,
not a code switch), `preset_label`+`preset_version` (denormalized snapshot — **not** an FK to
a JSON registry, so preset churn can't orphan facilities), `custody_type`, `rehypothecation`
(none|allowed|unknown), `control_mechanism` (live_key|presigned_only|none), `principal_asset`,
`principal_amount`, `collateral_asset` (default BTC), `status`
(open|repaid|defaulted|liquidated|cancelled|**disputed**), `public_offering` (bool),
`interest_asset` (**defaults to principal currency**; routes the interest leg),
`interest_terms`, `as_of_custody_date`, `notes`.

`custody_type` enum (finer than a binary):

| Value | Who holds keys | Lock default | Providers |
|---|---|---|---|
| `non_custodial_multisig` | Borrower holds a **live** key (2-of-3) | not-a-disposal | Hodl Hodl, Debifi |
| `non_custodial_presigned` | Borrower's key generated once then **discarded** | not-a-disposal (*different* argument) | Firefish |
| `collaborative_multisig` | Borrower 1-of-3 + sub-trust beneficial interest | not-a-disposal (+ title caveat) | Unchained |
| `custodial_segregated` | Provider holds all keys, ring-fenced/attested | not-a-disposal + soft review | Ledn Custodied, Strike, Xapo |
| `custodial_rehypothecated` | Provider holds all keys, may re-lend | not-a-disposal + strong contested | SALT, legacy Ledn |
| `onchain_smartcontract` | Code custodies (no human key) | per-product; BTC leg only | Coinbase/Morpho, some DLC |

`control_mechanism` exists because Firefish's borrower key is **discarded** at setup — at
steady state the borrower holds no live signing key, only pre-signed deterministic outcomes.
The non-disposal argument there is "pre-committed outcomes + no third party gains free
control," not "retains a key" — surfaced distinctly, not asserted as settled.

**`loan_legs`** (links a journal transaction to a loan with a role): `loan_id`, `role`,
`transaction_id` (**nullable by role — see below**), `escrow_address`/`escrow_txid`/
`escrow_vout`, `amount`, `fiat_value`, `occurred_at`, `on_chain_present` (0 for off-chain/
ARK/fiat legs), `notes`.

Role enum and which roles **require** a non-null `transaction_id` (they book tax and need a
priceable journal row):

| Role | transaction_id | Treatment |
|---|---|---|
| `collateral_lock` | required | suppress disposal, tag encumbered |
| `collateral_topup` | required | non-event (may add an escrow position) |
| `collateral_release` / `recovery_release` / `cancellation_release` | required / required / required | non-event, basis carries |
| `liquidation` | required | **SELL at FMV** |
| `collateral_repay_sale` | required | **SELL at FMV** (voluntary repay-in-collateral; same surplus-split as liquidation) |
| `liquidation_surplus_return` | required | partial non-event |
| `interest_payment` | required iff `interest_asset=BTC` | disposal if BTC; else non-event |
| `escrow_consolidation` | required | internal hop (Firefish prefund→escrow); non-event |
| `wrapped_conversion_out` | required | out-of-scope wrap (Coinbase BTC→cbBTC); **quarantined**, not booked |
| `principal_draw` / `principal_repay` | nullable | not income / non-event |

**`loan_escrow_positions`** (advanced): per-UTXO basis allocation, `output_type` defaults to
`unknown` (**never** assume P2TR/P2WSH/P2SH-P2WSH — read it from the artifact). Firefish
top-ups create a fresh escrow address per top-up, so one loan can have several positions;
partial liquidation allocates by exact-cover.

## 6. Per-provider handling

custody · base-layer observability · detection mode · reconstruction. **Honest where
manual-only.** No provider publishes a loan export; the chain never marks a loan.

| Provider | Custody | Observability (watch-only) | Detection | Reconstruction |
|---|---|---|---|---|
| **Firefish** | non-custodial, 3-of-3 P2TR-NUMS, borrower key discarded | own→prefund→escrow (escrow not in borrower descriptor → looks external); one of 4 closing spends; per-top-up escrows | heuristic/manual | paste escrow addr(s) + return addr + txids; each escrow = one watched scriptPubKey, `output_type=unknown` |
| **Hodl Hodl Lend** | non-custodial 2-of-3 P2SH | deposit(s)→escrow; one release; liquidation refunds surplus in same tx | API-anchor/heuristic | REST `escrow{address,deposit_txid,release_txid}` auto-proposes the pair, field-presence-gated; read script type from the returned witness_script |
| **Unchained** | collaborative 2-of-3, sub-trust | full wallet visible **if** the vault descriptor is imported watch-only | descriptor/manual labeling | import wallet-config JSON; read `addressType` (P2SH `m/45'` or P2WSH `m/48'`); chain can't tell release from liquidation → manual leg labels |
| **Debifi** | non-custodial 3-of-4 P2SH | per-loan escrow; deposit + spend | heuristic/manual | paste escrow + txids; 4th key holder is a role |
| **Lygos** | non-custodial 2-of-2 DLC | indistinguishable; may be off-chain | manual | manual; LTV 60–75 % |
| **Lendasat** | ships ≥1 shape (DLC / ARK / 2-of-3) | DLC indistinguishable; **ARK lock is off base layer** | manual | manual; `on_chain_present=0` allowed so ARK loans aren't dropped |
| **Ledn** | fully custodial (Custodied default ≥2025-07; legacy rehyp) | outbound to omnibus; release via different hot wallet (not correlatable) | import-only (CSV) | CSV deposit→`collateral_lock`, withdrawal→`release`/`surplus`; sample headers, quarantine unknowns; voluntary repay-in-collateral=`collateral_repay_sale` |
| **Nexo** | fully custodial | outbound→provider; opaque | import-only (CSV) | 10-col CSV; Type literals unverified → quarantine unknowns |
| **SALT** | fully custodial, explicit repledge | opaque | import-only (CSV) | CSV + manual; `rehypothecation=allowed` → strongest contested flag |
| **Strike** | custodial, delegated 3rd-party custody | opaque | import-only (CSV) | Strike CSV already parses (`normalize_strike_record`); loan rows → manual leg-tag downstream |
| **Xapo / Wirex** | fully custodial (MPC) | opaque | manual | manual/statements |
| **Coinbase/Morpho** | on-chain protocol (BTC→cbBTC on Base) | **zero BTC base-layer footprint for the lock** | manual | only the BTC withdrawal leg in scope → `wrapped_conversion_out`, flagged out-of-scope |

## 7. Detection + import

**Stance: heuristic/import-only — never chain-pattern auto-detection.** The single most
important guard: **never auto-`SELL` an unlabeled outbound.** Import tiers, registry-dispatched
(new `kassiber/core/loans/` registry, **not** the Lightning one):

1. **Manual** — universal baseline; the 3-question wizard. Every provider falls back here.
2. **Reconciliation assist ("Find in my wallets")** — runs `build_owned_index`/deep-derive
   ([ownership.py](../../kassiber/core/ownership.py)) over a pasted escrow address/txid to
   prefill the lock outbound and flag owned-vs-external. Heuristic help, not classification.
3. **Descriptor watch-only (Unchained)** — import the wallet-config JSON; read `addressType`
   from it; sync via `build_owned_index`. Gated on "descriptor present?".
4. **API-anchor (Hodl Hodl)** — propose the pair from the escrow object; field-presence-gated.
5. **CSV (Ledn/Nexo/Strike)** — existing conservative importers; rows land **quarantined**
   until the user assigns roles; build column maps from sampled real exports, never hardcoded.

**Escrow addresses are a THIRD ownership category** ("encumbered / co-controlled") — never
folded into "owned" by `wallets identify`/`build_owned_index`. A 2-of-3 or 3-of-4 escrow is
*partially* owned; if registered as owned, the engine would treat escrow UTXOs as spendable
and could auto-pair a liquidation outflow as a self-transfer (mis-taxing the disposal as a
non-event). The Unchained descriptor import in particular must register the vault as a
**read-only encumbered** descriptor.

**Do not build** (each refuted by research): a per-provider CSV parser to a guessed schema; an
address-shape auto-detector (Firefish P2TR-NUMS and DLC funding are indistinguishable); auto
lock↔release pairing (custodial cold-in/hot-out is uncorrelatable; multisig can't distinguish
release from liquidation); auto-upgrading an outbound-to-multisig into a lock. CSV loan-row
quarantine happens **downstream** of normalization (a post-import "unassigned outbound to a
known-lender address" sweep), not by changing `normalize_strike_record`, which already books
typed rows.

## 8. UX — simple by default, advanced on demand

**New `/loans` side-nav route**, sibling to `/exit-tax` and `/source-of-funds`. Precise terms
(workspace/profile/account/wallet), not invented branding.

**Simple default — one dialog, three plain questions:**
1. *Role*: "Borrowing against your Bitcoin, or lending Bitcoin?" → `role` (default borrower).
2. *Provider*: searchable picker (12 known + "Other / private"); a known provider pre-fills
   `custody_type`, `control_mechanism`, footprint expectation — all editable, never silent.
3. *Custody*: "Who can move your collateral?" — "Only me / I co-hold a key"
   (`non_custodial_multisig`) · "A pre-signed escrow returns it automatically"
   (`non_custodial_presigned`) · "The platform holds it" (`custodial_segregated`). **"Not
   sure" → `custodial_segregated` + `rehypothecation=unknown` → soft review chip only** —
   never the strong contested/rehypothecated branch (that would over-tax the unsure user,
   the opposite of conservative).

**Lock pairing:** pre-select the most recent owned→external outbound ("Is this the Bitcoin
you sent as collateral?"). **Close-out:** one prompt — Repaid / Liquidated-defaulted / Still
active. The interest flow explicitly asks "paid in BTC?" before booking (BTC interest is a
disposal).

**Advanced expander** (never in the default path): full role editor (attach to txid+vout),
full `custody_type` + orthogonal `rehypothecation` + `control_mechanism` + `as_of_custody_date`
override, liquidation modeling (debt, FMV/price source, fees as disposal costs, auto
surplus-split), per-escrow-UTXO positions, per-leg interest currency, `public_offering`
toggle, advisory banners (shown only when that loan's custody triggers them), Steuerberater
handoff export.

**Signal-not-reassurance:** surface a chip **only when actionable** — needs lock pairing ·
needs close-out · custodial/rehyp lock review · liquidation missing FMV · BTC interest to
confirm · quarantined leg awaiting a role. **No LTV/margin chip** (that's price-monitoring,
not accounting, and would stand permanently on healthy loans — Kassiber is not a margin
monitor). **No standing "Active" badge** — a clean active loan shows just "N BTC locked," no
status. Liquidation is observed *after* it happens, as a real spend to label.

**CLI (co-equal peer):** `kb loans add|leg|pair|import|identify|status|export|set`, with
`status` actionable-only and `import` quarantine-first. CLI/daemon stay
English/machine-deterministic; GUI strings land in `en` + Austrian-`de` (informal `du`) in
lockstep, legal/tax terms deferred-German per the SoF precedent.

## 9. Tax wiring

Classification hook in [tax_events.py:896](../../kassiber/core/tax_events.py), **before** the
`elif direction == "outbound":` disposal branch: if a `loan_legs` row references the
transaction, dispatch on `role` first (lock → suppress + tag encumbered; liquidation/repay-sale
→ SELL; release/topup/recovery/cancellation → non-event; interest → disposal iff BTC). Roles,
not address shape, decide taxability.

**Guards (never let the default fire wrong):** liquidation is always its own role that books a
SELL; the `collateral_release` non-event is gated on the destination being an owned/return
address (never treat a liquidation outflow as a release); unpaired outbounds to known-external
multisig/omnibus addresses are quarantined as "possible loan leg — confirm" rather than booked.
The lender 27.5 %-vs-progressive fork is gated behind an explicit advisory + advisor confirmation
and a primary-source citation; it defaults to progressive and never auto-applies 27.5 % from a
preset guess.

## 10. Phasing

- **Phase 1 — core + manual + tax (ships first).** `loans` + `loan_legs` tables, the
  encumbrance lot-tag + lock-suppression regime + guards at `tax_events.py:896`, the
  3-question wizard + `/loans` route, CLI `loans add/leg/pair/status`, the "Find in my
  wallets" reconciliation assist, signal-not-reassurance status, and the Altvermögen
  round-trip test. Covers all providers correctly at the data/tax layer; satisfies the
  simple-default + advanced-expander goal.
- **Phase 2 — import on-ramps for the verified-easy cases.** Unchained descriptor import
  (read `addressType`, register encumbered/read-only), Hodl Hodl escrow-object pairing
  (field-presence-gated), Ledn/Nexo CSV via sampled-not-hardcoded mapping with quarantine.
- **Phase 3 — advanced granularity.** `loan_escrow_positions` multi-UTXO basis allocation,
  surplus-split UI, liquidation fee/FMV modeling, per-leg interest currency, Steuerberater
  handoff export, advisory banners, BIP329 label→role mapping, preset-library review process.
- **Deferred / out of scope:** any chain-pattern auto-detector; Coinbase cbBTC-on-Base
  decoding (EVM, out of Bitcoin-only scope — BTC withdrawal leg + wrap-flag only).

## 11. Open risks — need a Steuerberater / legal ruling

Surface as advisory, never hardcode a conclusion. See also
[07-austrian-tax-open-questions.md](07-austrian-tax-open-questions.md).

1. **Non-custodial lock = not a disposal** — advisory, not BMF-confirmed; the whole model
   rests on it.
2. **Custodial/rehypothecating lock = possible disposal at FMV** — contested; no Austrian
   ruling; GMLaw declines to hold rehyp = disposal. Flag, never auto-realize.
3. **Firefish discarded-key** — borrower holds no live key post-lock; the non-disposal
   argument is "pre-committed outcomes," not "retains a key." Distinct, advisory.
4. **Unchained sub-trust title split** — legal title in trust, borrower holds beneficial
   interest; could push toward the contested branch. Advisory caveat.
5. **Lender 27.5 % vs progressive ≤55 %** — turns on whether a marketplace listing meets the
   "öffentlich angeboten" test (§27a Abs 2). Per-deal; needs its own citation before shipping.
6. **Coinbase BTC→cbBTC wrap** — potential wrapped-asset disposal under AT rules; invisible to
   base-layer sync. Import-only flag.

**Engineering risks + mitigations:** custody as a platform constant → mis-tax across a flip
(per-facility, dated `as_of_custody_date`; presets only seed); hardcoded script types → wrong
address derivation (read from the artifact; `output_type=unknown`); one-escrow-UTXO assumption
→ broken partial-liquidation basis (`loan_escrow_positions` + exact-cover); pairing a
liquidation as a self-transfer → mis-tax the disposal (role-not-shape decides; gate the release
non-event on an owned destination); guessed CSV/API schema → silent mis-mapping (sample real
exports; quarantine unknowns; field-presence-gate auto-import).
