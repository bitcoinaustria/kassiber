# 12 — Bitcoin-backed loans (shipped: per-transaction loan marks)

Status: **shipped, minimal.** This supersedes an earlier facility-centric design
(provider presets, a custody/rehypothecation matrix, import on-ramps, liquidation
modelling, a Steuerberater export, a `/loans` screen). That design was explored,
built, and then **deliberately collapsed** to the one tax fact that actually
matters. The full facility sketch lives in git history if it is ever needed again.

## What ships

Kassiber supports two distinct Bitcoin loan shapes as per-transaction *marks*,
not a facility:

- A fiat loan with BTC posted as collateral. The collateral lock is **a non-event
  for tax**: the borrower still owns the coins, just encumbered. The fiat
  principal itself is outside Kassiber.
- A BTC-denominated loan. Borrowed BTC principal is also a non-event for tax:
  receiving principal is not income/acquisition of owned coins, and repaying
  principal is not a disposal of owned coins.

- **Mark an outbound `collateral_lock`** → BTC collateral posted for a fiat loan;
  the disposal is suppressed (the coins stay in the owned global pool).
- **Mark the returning inbound `collateral_release`** → BTC collateral returned;
  the acquisition is suppressed (the coins re-enter the pool they never really left).
- **Mark an inbound `loan_principal_received`** → borrowed BTC principal received;
  the acquisition/income event is suppressed.
- **Mark an outbound `loan_principal_repaid`** → borrowed BTC principal repaid;
  the disposal event is suppressed.

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

- Storage: one row per mark in `loan_legs` (`transaction_id`, `role`, optional
  `loan_id`); the engine reads only `(transaction_id → role)` via
  `kassiber.core.loans.load_collateral_role_map`. `loan_id` is UI/audit grouping
  metadata and has no tax effect.
- Engine: suppression in `kassiber/core/tax_events.py` + `kassiber/core/engines/rp2.py`.
- CLI: `kassiber loans mark|link|unmark|list`.
- Daemon: `ui.loans.{list,link,mark,unmark}`.
- GUI: a row action + understated badge on the Transactions screen, with linked
  loan legs shown in the transaction detail sheet (no dedicated route).

## Resilience precursor (also shipped)

Independent of loans: a carrying-value swap whose leg was blocked in phase 1
(e.g. `insufficient_lots` on a self-custody round-trip mis-paired as a BTC↔L-BTC
swap) is quarantined as a *pair* in `_select_at_cross_asset_swap_links` rather than
promoted to an `at_swap_link` that would bypass the quantity gate and abort the
whole multi-asset report. Regression: `ATSwapOverSellQuarantineTest`; contract in
[docs/austrian-handoff.md](../austrian-handoff.md).
