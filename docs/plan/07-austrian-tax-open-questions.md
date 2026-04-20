# Austrian Tax — Open Questions (Live Backlog)

This document catalogues tax questions where **BMF has not published a clear position** or where authoritative sources disagree. It is planning input for the future Austrian RP2-backed path; Kassiber does not apply these defaults today. For each:

- A default behavior the engine applies
- The evidence informing that default
- What would change the default

These are surfaced in the E 1kv PDF footer so a reviewing Steuerberater sees exactly which assumptions were used.

Important implementation note: these defaults only apply when Kassiber's normalization/provenance layer has enough information to classify the event confidently. If the required facts are missing, the event is quarantined instead of silently receiving one of these defaults.

**Last updated:** 2026-04-18. Maintainer: project owner + Steuerberater (post-MVP).

---

## AT-001 — Lightning routing fee classification

**Question.** How are bitcoin amounts earned as Lightning Network routing fees classified for Austrian tax?

**Default applied.** Laufende Einkünfte aus Kryptowährungen at Fair Market Value on the date of receipt, taxed at 27.5% KESt (§ 27b Abs 2 EStG, by analogy to lending interest paid in kind).

**Why this default.**
- No BMF FAQ entry, no published WKO guidance, no practitioner article located as of 2026-04.
- The closest analogous treatment is § 27b Abs 2 Z 1 (lending interest paid in kind) and Z 2 (mining income). Both are treated as laufende Einkünfte at FMV.
- Lightning routing is neither lending (no principal transferred) nor mining (no consensus contribution), but the income pattern (reward for providing network capacity in exchange for running infrastructure) is closer to staking/masternode income, which is covered by Z 2.

**What would change the default.** A BMF FAQ, an Einkommensteuer-Richtlinie update, or a published Steuerberater opinion specifically addressing Lightning. If income were reclassified as:
- Not taxable until conversion: engine would emit zero-basis acquisition entries and only tax on later disposal
- Sonstige Einkünfte (§ 29) at progressive rate: engine would surface as informational only; user applies own rate on E 1

**Sources consulted, none authoritative on this specific question:**
- [BMF — Kryptowährungen](https://www.bmf.gv.at/themen/steuern/sparen-veranlagen/steuerliche-behandlung-von-kryptowaehrungen.html)
- [§ 27b EStG](https://www.jusline.at/gesetz/estg/paragraf/27b)
- [TPA — Krypto-Besteuerung NEU](https://www.tpa-group.at/news/krypto-besteuerung-neu-was-jetzt-wirklich-gilt/)

**Status.** Open. Priority: medium — relevant to users running Lightning node, niche but real.

---

## AT-002 — L-BTC (Liquid Bitcoin) status under § 27b

**Question.** Is Liquid Bitcoin (L-BTC), a federated sidechain asset that represents BTC 1:1 via a peg mechanism, a "Kryptowährung" within the meaning of § 27b Abs 4 EStG?

**Default applied.** Yes — L-BTC is treated as a Kryptowährung for all kassiber AT engine purposes. Same tax treatment as native BTC.

**Why this default.**
- § 27b Abs 4 definition: digital representation of value, not issued by a central bank, not tied to legal currency, electronically transferable/storable/tradable. L-BTC satisfies every clause.
- BMF has not published an explicit position on sidechain assets.
- Stablecoins are expressly covered (BMF FAQ); L-BTC is not a stablecoin but the inclusion logic is the same.
- Treating L-BTC differently from BTC would be inconsistent with the economic equivalence of the peg.

**Potential concern.** L-BTC is technically issued by a federation of functionaries (the Liquid federation). A restrictive reading of "not issued by a central authority" could argue L-BTC is issued by an authority, namely the federation. No Austrian court or BMF has addressed this.

**What would change the default.** A BMF ruling distinguishing sidechain assets from their anchors. A court decision on federation-issued tokens.

**Engine behavior.** L-BTC transactions flow through the engine identically to BTC transactions. `wallet.kind='descriptor'` with Liquid descriptors produces Neuvermögen lots as expected. No separate asset code.

**Status.** Low priority. Relevant only when Liquid wallets are used. Default is defensible.

---

## AT-003 — Mixed-wallet consolidation sweep under moving-average

**Question.** When multiple wallet addresses with different running averages are consolidated into a single destination via a sweep transaction, what is the correct cost-basis treatment under § 2 KryptowährungsVO (gleitender Durchschnittspreis per Wallet-Adresse)?

**Default applied.** Weight the incoming averages by quantity. The destination wallet's new running state becomes:

```
new_qty = sum(source_qty for each source)
new_avg = sum(source_qty * source_avg for each source) / new_qty
```

This is a pure pooling rule.

**Why this default.**
- § 2 KryptowährungsVO mandates moving average per wallet address but is silent on consolidation.
- The pooling rule preserves the total cost basis across the consolidation and is economically neutral.
- Any alternative (e.g., "consolidation is a disposal") would conflict with § 27b Abs 3 Z 2's non-taxable swap principle.

**Potential concerns.**
- A strict reading of "per Wallet-Adresse" could require the destination to track each source's contribution separately forever — infeasible and not practiced by any tool reviewed (Blockpit, Accointing, Koinly with AT preset).
- A BMF clarification could mandate that the earliest-acquired source contribution determines the holding period for purposes of the Regelbesteuerungsoption calculation (but Neuvermögen has no holding-period rate benefit, so this is moot for 27.5% KESt).

**What would change the default.** A BMF ruling on multi-input consolidation. A Steuerberater instruction per specific client.

**Engine behavior.** Consolidation events from self-transfers apply the weighted average across the contributing normalized tax containers. The journal entry carries a note recording the source wallets/containers and their contributed quantities. Entry has `note="AT-003 default pooling rule applied"`.

**Status.** Open. Priority: medium — important for any user who does wallet hygiene (many addresses → one cold storage wallet).

---

## AT-004 — Inheritance basis

**Question.** When a taxpayer inherits BTC, what cost basis and acquisition date apply for the heir's later disposal?

**Default applied.** Buchwertfortführung: the decedent's original cost basis and acquisition date carry over to the heir. If the decedent's holdings were Altvermögen (acquired on/before 2021-02-28), the heir receives Altvermögen lots. Holding-period clock is continuous.

**Why this default.**
- Strong practitioner consensus (ICON, TPA, crypto-tax.at, lrz.legal).
- Consistent with the gift-carryover rule under § 6 Z 9 EStG analogized to crypto.
- Aligns with the treatment of other capital assets under § 27 (securities) where Buchwertfortführung is standard.

**Potential concern.**
- BMF FAQ does not address inheritance expressly.
- Some German-language sources (German, not Austrian) apply FMV-at-death as basis. If the Austrian BMF reversed course to follow this, the heir would gain a stepped-up basis.

**What would change the default.** A BMF ruling endorsing FMV-at-death basis. Unlikely based on current Austrian tax policy direction.

**Planned engine behavior.** MVP does not yet have a dedicated inheritance workflow. Until explicit provenance capture exists, inherited holdings should be either annotated through the tax-annotation layer or quarantined as unsupported for Austrian processing. Once Austrian support lands, it should capture original acquisition date and basis explicitly rather than inferring from first observation.

**Status.** Open. Priority: low — relevant only to users with inherited BTC. Default is the safe choice.

---

## AT-005 — CoinJoin and mixed-output transactions

**Question.** How should transactions that pass through a CoinJoin (JoinMarket, Whirlpool, Samourai, Wabisabi, etc.) be treated for cost-basis tracking?

**Default applied.** A CoinJoin participation is treated as a self-transfer for the user's own contributed inputs and outputs — the running average is preserved, basis carries. Fees paid to coordinators/miners in the CoinJoin reduce basis proportionally on the user's contributed amounts.

**Why this default.**
- Economically, a CoinJoin is a self-transfer: the same user's coins go in and come out, transformed by a privacy-enhancing batching.
- No BMF guidance specific to mixing exists.
- Treating it as a disposal would conflict with § 27b Abs 3 Z 2 (swap non-taxable for Neuvermögen) and would produce absurd results (gain/loss events every time the user mixes).

**Potential concern.**
- BMF could argue that the mixing breaks the chain-of-title and constitutes a disposal event. No authority has made this argument as of 2026-04.
- If treated as a disposal, every CoinJoin would be a taxable event at FMV with proceeds ≈ cost (modulo fees) — realizing near-zero gains but generating reporting burden.

**What would change the default.** An Austrian court or BMF ruling treating CoinJoin as a taxable event.

**Engine behavior.** CoinJoin handling is not assumed to exist for MVP. Until Kassiber has explicit CoinJoin provenance or robust detection, CoinJoin-like fanout transactions should remain quarantined or manually annotated instead of being auto-classified.

**Status.** Open. Priority: medium for privacy-conscious users, low for typical cases.

---

## AT-006 — Replace-by-Fee (RBF) and Child-Pays-for-Parent (CPFP) fee treatments

**Question.** When an RBF or CPFP transaction replaces a pending transaction, how is the differential fee treated?

**Default applied.** The final fee (after all replacements settle) is what counts. Intermediate fees from replaced transactions are ignored (they were never effective). This matches the on-chain reality: replaced transactions have no effect once evicted from the mempool.

**Why this default.**
- Straightforward on-chain accounting principle.
- No BMF guidance; universal practitioner agreement.

**What would change the default.** Unlikely to change.

**Engine behavior.** Only confirmed transactions are ingested. Replaced transactions never appear in the wallet's transaction history per the sync backend's view.

**Status.** Closed-ish — default is clearly correct; listed here for completeness.

---

## AT-007 — Spending sats on goods/services at point of sale — daily vs per-transaction FMV

**Question.** For a user making many small BTC payments in a day (e.g., at a Bitcoin-accepting shop), must each transaction be valued at its exact timestamp FMV, or is daily FMV acceptable?

**Default applied.** Per-transaction FMV using the hourly CoinGecko rate closest to the transaction timestamp. Daily close used only when hourly is unavailable.

**Why this default.**
- BMF requires "angemessene Bewertung" without specifying granularity.
- Hourly data is readily available; no reason to coarsen.
- Consistency with moving-average treatment (which needs a per-event price).

**Alternative worth considering.** Daily weighted-average price for the transaction day, which some practitioner guides accept. Would simplify for small-transaction users but introduces basis drift vs the hour-level reality.

**What would change the default.** User preference — make it configurable in a later phase.

**Engine behavior.** Current default is per-transaction using nearest available rate from `rates_cache`. A future CLI flag `--rate-granularity daily|hourly|per-tx` could expose this choice.

**Status.** Open. Priority: low.

---

## AT-008 — Wallet seen for the first time after 2021-02-28 — does late first-sync change classification?

**Question.** A user adds an xpub wallet to kassiber in 2024 whose earliest on-chain transaction is 2020-06-01. Should those pre-2021-02-28 receipts be classified as Altvermögen?

**Default applied.** Yes. Classification is based on the transaction's on-chain timestamp, not when it was first observed by kassiber. The xpub-add event is irrelevant to tax regime classification.

**Why this default.**
- The on-chain timestamp is the economic reality of when the user acquired the coins.
- BMF classification is by acquisition date, not by discovery date.

**Potential concern.** If the user cannot prove ownership of the wallet back to 2020, BMF could demand evidence. Not an engine problem — a user-and-Steuerberater problem.

**What would change the default.** Nothing technical. The user must be able to demonstrate ownership if audited; the engine cannot verify this.

**Engine behavior.** As described. The engine does not depend on a separate persisted "first observed by kassiber" field for the tax classification itself.

**Status.** Closed. Default is clearly correct.

---

## AT-009 — Altvermögen declaration vs computed

**Question.** Who decides whether a wallet's contents are Altvermögen — the user's explicit declaration, or the engine's computation from transaction timestamps?

**Default applied.** Both, with the user's explicit declaration acting as a safety override. The future RP2-backed engine would classify each lot by its acquisition date. If Austrian support needs explicit wallet-level provenance again, Kassiber should add it deliberately rather than assuming a live `Altbestand` workflow already exists.

1. **Trust signal:** If the user has declared the wallet as originating from Altvermögen holdings, this is logged alongside computed classifications. Discrepancies (e.g., user declares Altvermögen but a lot has timestamp 2022-05-01) surface as warnings, not errors.
2. **Fallback:** For wallets imported from exchanges or partial records where some transaction dates are unknown, the declaration applies to pre-reform-dated lots.

**Why this default.**
- BMF expects the taxpayer to know their own history. Explicit declaration reflects reality the engine cannot verify.
- Automatic classification is the safe default; user override provides a clear audit trail.

**What would change the default.** A BMF requirement that tools refuse to compute Altvermögen without explicit user declaration. Unlikely.

**Planned engine behavior.** As described. Warnings should surface in the PDF's disclaimer section for review once Austrian output exists again. Add any wallet-level Austrian provenance contract only if the RP2-backed path proves it is necessary.

**Status.** Open. Could evolve into a small UI flow in Phase 5+ where the user walks through each wallet confirming Altvermögen status.

---

## AT-010 — Multiple domestic providers with overlapping KESt withholding

**Question.** From 2024-01-01, domestic Austrian crypto providers withhold KESt automatically. If a user has multiple domestic providers and each withholds on its own slice, does that sum correctly to the user's total owed?

**Default applied.** Yes, each provider's KESt withholding is independent and correct for the disposals on that provider. Kassiber's engine treats off-provider activity (self-custody, foreign exchanges) as the user's taxpayer-declaration surface. No double-counting.

**Why this default.**
- Austrian KESt withholding is per-account under current rules — each provider knows only their own transactions.
- Cross-provider loss offsetting requires user to declare on E 1kv with Regelbesteuerungsoption or regular filing.

**Potential concern.** If a provider withholds KESt on a gross gain but the user had offsetting losses elsewhere, the user may over-pay in the withholding year. This is a feature of the regime, not a bug.

**What would change the default.** A BMF clarification on cross-provider offset handling.

**Planned engine behavior.** This is not fully supported in MVP because Kassiber does not yet persist provider domicile / withheld-KESt metadata in a structured way. Until that metadata exists, the future Austrian output should treat provider-withheld tax as outside the supported automation boundary and surface a warning or quarantine as appropriate.

**Status.** Open. Priority: medium for users with multiple domestic providers.

---

## Summary of defaults and review gates

| ID | Default | Priority | Steuerberater review |
|---|---|---|---|
| AT-001 | Lightning routing fees as laufende Einkünfte at FMV (27.5%) | medium | required |
| AT-002 | L-BTC treated as Kryptowährung | low | required if Liquid used |
| AT-003 | Consolidation sweep weighted-average pooling | medium | recommended |
| AT-004 | Inheritance basis via Buchwertfortführung | low | required if inheritance used |
| AT-005 | CoinJoin as self-transfer, basis preserved | medium | recommended if privacy tools used |
| AT-006 | RBF/CPFP — final fee only | — | n/a (clearly correct) |
| AT-007 | Per-transaction FMV (hourly) | low | n/a |
| AT-008 | On-chain timestamp governs classification | — | n/a (clearly correct) |
| AT-009 | Altvermögen: computed + user-declared hybrid | low | recommended |
| AT-010 | Per-provider KESt; user declares rest | medium | required |

The PDF report footer lists every open question whose default was invoked for transactions in the report period, so the reviewing Steuerberater knows exactly which assumptions to verify.

---

## Process for resolving entries

When a question is resolved (new BMF ruling, Steuerberater opinion obtained, consensus shifts):

1. Update this document — keep the history of the prior default in an "Old default" section
2. Add a migration only if the resolution changes persisted annotation/provenance fields or other stored metadata
3. Update engine code with the new behavior
4. Bump a `at_engine_version` constant
5. Offer the user a "Recompute journal" action on profile settings; the prior journal is archived
6. Report footer updated to reference resolution

The process above is the same whether the resolution tightens or loosens the default.
