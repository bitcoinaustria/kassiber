# 12 — Bitcoin-backed loans (shipped: per-transaction collateral mark)

Status: **shipped, minimal.** This supersedes an earlier facility-centric design
(provider presets, a custody/rehypothecation matrix, import on-ramps, liquidation
modelling, a Steuerberater export, a `/loans` screen). That design was explored,
built, and then **deliberately collapsed** to the one tax fact that actually
matters. The full facility sketch lives in git history if it is ever needed again.

## What ships

A collateral lock is **a non-event for tax**: the borrower still owns the coins,
just encumbered. So Kassiber models it as a per-transaction *mark*, not a facility:

- **Mark an outbound `collateral_lock`** → the disposal is suppressed (the coins
  stay in the owned global pool).
- **Mark the returning inbound `collateral_release`** → the acquisition is
  suppressed (the coins re-enter the pool they never really left).

A lock/release round-trip therefore nets to zero and preserves the original basis
**and acquisition date** (Alt/Neu by date, not a hold period — covered by the
Altvermögen round-trip test).

## Why a mark, not a facility

The chain carries no loan semantics for any provider, and custody type /
rehypothecation / interest schedule / liquidation mechanics do not change the
default booking (a lock is not a disposal regardless). All of that was advisory
scaffolding around a single per-transaction classification — so it was removed.
Keeping it would have been a large, provider-coupled surface that rots when a
provider flips custody, for no change to the numbers.

## Liquidation (no modelling)

If collateral is seized and never returns, a real disposal happened — but watching
for it is the loan platform's job, not Kassiber's. The user **removes the mark**
and the outbound reverts to the normal disposal it always was (booked at that
transaction's date/value; override the tx pricing for a precise seizure-date FMV).
`open_collateral_locks` surfaces locks with no offsetting release as a reconcile
hint ("repaid, or liquidated?"). No `liquidation` role, no status machine.

## Architecture note (the one that mattered)

Encumbrance is **not** a synthetic RP2 exchange/account. RP2's availability gate is
per-`(exchange, holder)`; routing locked collateral through a separate
balance-bearing account re-introduces the "balance went negative" abort. Instead a
mark only *suppresses* the lock/release events — the lot never leaves the global
per-asset pool, so no per-account balance can go negative.

## Surface

- Storage: one row per mark in `loan_legs` (`transaction_id`, `role`); the engine
  reads `(transaction_id → role)` via `kassiber.core.loans.load_collateral_role_map`.
- Engine: suppression in `kassiber/core/tax_events.py` + `kassiber/core/engines/rp2.py`.
- CLI: `kassiber loans mark|unmark|list`.
- Daemon: `ui.loans.{list,mark,unmark}`.
- GUI: a row action + understated badge on the Transactions screen (no dedicated route).

## Resilience precursor (also shipped)

Independent of loans: a carrying-value swap whose leg was blocked in phase 1
(e.g. `insufficient_lots` on a self-custody round-trip mis-paired as a BTC↔L-BTC
swap) is quarantined as a *pair* in `_select_at_cross_asset_swap_links` rather than
promoted to an `at_swap_link` that would bypass the quantity gate and abort the
whole multi-asset report. Regression: `ATSwapOverSellQuarantineTest`; contract in
[docs/austrian-handoff.md](../austrian-handoff.md).
