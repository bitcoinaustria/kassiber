# 16 — Custody UX north star: the Custody Inbox

Status: implemented — the `/swaps` route now ships the Inbox | History |
Advanced surface described below (`ui-tauri/src/routes/transfers-custody/
CustodyInbox.tsx` + decision cards + `inboxModel.ts`); supersedes the
information architecture shipped in PR #455 while keeping its daemon
contracts.
An interactive mockup of this design lives at
[docs/plan/assets/custody-inbox.html](assets/custody-inbox.html) — open it
directly in a browser (a working copy may also sit in the untracked
`ui-tauri/claude-design/` staging directory).

## Why the current surface fails

PR #455 correctly merged Swaps and Custody Gaps into one route and killed the
raw JSON editor, but the merged screen still transplants the *system's* mental
model onto the user:

- **Two queues, two vocabularies.** The Review tab speaks
  pair/kind/policy/conflict-cluster; the Custody-gaps tab speaks
  bridge/residual/suspense/retained. Both queues ask the user the *same
  underlying question* ("do these movements belong together?") in unrelated
  words.
- **A hidden 2×2 matrix.** Review nests a segmented control (Bitcoin moves /
  Swaps) times a queue/history toggle inside a tab, inside the page tabs.
- **Data without actions.** Three stacked timelines (summary tiles, the
  4-level coverage timeline that is explicitly "technical only — never clears
  a gap", the lineage timeline) sit above the actual work. Metric strips
  aren't filters. Fee is shown three ways per row.
- **Jargon surfaces verbatim.** ~70 backend issue codes render as sentences;
  suspense, conservation, anchors, epochs, authored-active vs effective-active
  appear on primary surfaces.
- **The expert path is the centerpiece.** An ~18-input component form is the
  first thing on the Components tab, although the guided gap flow needs *zero*
  amount/leg/allocation input from the user.

## The domain fact the design is built on

The core already does all the work (see `custody_gap_reviews.py`,
`custody_gaps.py`):

- A guided bridge (`action: create`) needs **only `gap_id` + confirmation**.
  Legs, allocations, fees, and the suspense residual are authored by
  `_guided_component_spec`. Dismissal needs the same. Residual classification
  is one 6-way choice. Conflicts are pre-clustered with a score margin.
- The engine ranks candidates (score 0–1000, confidence bands), flags
  `promotion_eligible` ("safe to propose"), and computes the tax blast radius
  (`downstream.affected_disposals` / `affected_years`) per gap.
- Swap/transfer pairing is the same shape: `ui.transfers.pair` with sensible
  kind/policy defaults; exact-evidence candidates are one-click safe.

So the **minimum decision set per item is**:

1. *(only if conflicting)* Which competing explanation is right?
2. **Is this the same money?** → Yes (bridge/pair) / No (dismiss).
3. *(only if a residual remains)* Where did the remainder go? (6-way)

Everything else the UI currently asks for is noise. The north star: **the UI
asks exactly these questions, one at a time, in plain language — and nothing
else.**

## North star: one inbox, one card, one question

Model the surface on the app's own two best patterns — the Quarantine
save-and-next triage loop and the `ReviewDataTable` worklist with clickable
metric filters — and on an email inbox, which every user already knows.

### Information architecture

```
Custody  (one sidebar item, one badge = open questions, blocker tone when report-blocking)
├── Inbox      (default) — the unified decision queue
├── History    — settled decisions as one audit timeline (pairs + bridges +
│                components + dismissals), with Reopen/Revise/Undo per entry
└── Advanced   — expert escape hatches, out of the main path:
                 manual component builder (pre-seeded from a gap when opened
                 from one), auto-pair rules, saved views, bulk selection,
                 wallet coverage timeline, custody lineage timeline
```

The Inbox unifies gap candidates *and* transfer/swap candidates into **one
ranked queue** — they are the same user question with different evidence. A
type chip (Transfer · Swap · Missing wallet) distinguishes them; ranking is
report-blocking first, then score/coverage/amount (the engine's
`_candidate_sort_key` order).

### The screen (master–detail)

**Header = goal, not metrics.** One line of purpose ("4 open questions · 2
block your 2024 report"), a progress bar (resolved vs. total this cycle), and
filter chips that *are* the metrics (All / Blocks report / Suggested / Low
confidence). No passive counter tiles.

**Queue (left).** Compact rows, each a plain-language sentence:
"10 BTC left *Ledger cold* → 9.9 BTC arrived *Sparrow hot* 2 days later",
plus a confidence chip, a "Suggested" spark when `promotion_eligible`, and an
impact chip ("blocks 2024"). Low-confidence search hints collapse into a
single expandable group at the bottom. Keyboard: ↑/↓ move, y/n decide.

**Decision card (right).** One question per card:

1. **Headline question:** "Is this the same Bitcoin?"
2. **Flow diagram** — the centerpiece. Source wallet → dashed "unobserved
   hop" node (mixer/unknown wallet) → destination wallet, with amounts on the
   edges, the network fee small, and the *unexplained remainder* highlighted
   amber. This single picture replaces four metric tiles and the evidence
   dump.
3. **"Why Kassiber thinks so"** — the top 3 reason codes translated into
   sentences ("Both sides sit on a known Whirlpool boundary", "9.9 of 10 BTC
   returned within 2 days", "No competing explanation comes close").
   Full evidence, review history, and machine codes behind one disclosure.
4. **Impact line** — "Resolving affects 3 later sales · tax years 2024–2025."
5. **Two primary actions:** **"Yes — it's mine, connect it"** and
   **"No — not my money."** The confirm step expands inline: consequences in
   words (what basis carries, what re-opens, filed-report impacts from
   `filed_report_impacts`), the existing explicit-review checkbox, then
   Confirm. Never a second page, never a modal-on-modal.
6. **Conflicts first:** when `conflict_size > 1`, the card's first question is
   "Which explanation is right?" with the competitors as selectable options,
   the engine's leader pre-selected only when `competitor_score_margin` is
   comfortable.
7. **Residual follow-up:** after a bridge with `residual_msat > 0`, the *next
   card in the queue* is the follow-up question "Where did the remaining
   0.1 BTC go?" with the six classifications as human radio options grouped as
   *It left my custody* (spent / sold / gifted / lost) · *Still mine*
   (retained) · *Leave open for now* (suspense). Deferring is allowed —
   suspense is a first-class, honest answer.
8. **Save-and-next:** confirming advances to the next card automatically,
   with a 20s undo toast (keep the existing undo machinery).

**Done state is a reward, not an empty table:** "All custody questions
resolved — your 2024 report is unblocked", tied into the Overview readiness
pill.

### Vocabulary (primary surfaces only)

| System term | Inbox term |
| --- | --- |
| custody gap candidate | open question / unexplained move |
| bridge / create reviewed bridge | connect ("Yes — it's mine") |
| dismiss | not my money |
| suspense / residual | unexplained remainder / "leave open for now" |
| promotion_eligible | Suggested |
| conflict cluster | competing explanations |
| pair (exact evidence) | confirmed match |

Expert views (Advanced, History detail) keep the precise system vocabulary —
auditors need it; the inbox does not. The ~70 backend issue codes group into
a handful of human categories with the raw code behind a disclosure.

### What this removes from the primary path

- The nested segmented control and the queue/history toggle (History is a tab).
- All three custody-gaps timelines (coverage + lineage move to Advanced;
  per-gap evidence lives inside the decision card).
- The metric strips (replaced by filter chips) and triple fee rendering
  (one fee figure; sats/% behind hover).
- Kind/policy selects on every pairing (defaults from method/route; editable
  under "Details" in the card, and in Advanced bulk flows).
- The component form as an entry point (reached only from Advanced or from a
  gap the guided path refuses, pre-seeded either way).

### Contract fit (no daemon changes required)

Everything above is renderable from existing kinds:
`ui.custody.gaps.list` (score, confidence, reason_codes, promotion_eligible,
conflict fields, downstream, residual/excess msat, summary + cursors),
`ui.custody.gaps.review_context` / `history`, `ui.custody.review.plan|apply`
(warnings, filed_report_impacts, input_version),
`ui.transfers.suggest` (counts, confidence, method, conflict_size),
`ui.transfers.pair|dismiss|bulk_pair|unpair|update|list`,
`ui.transfers.components.*`, `ui.saved_views.*`, `ui.transfers.rules.*`.
The inbox is a *reshuffling of presentation*, not a new backend surface.

### Build order (suggested)

1. Inbox shell: unified queue + ranking + filter-chip header (reuse
   `ReviewDataTable` internals where they fit, else a thin new list).
2. Decision card for gap candidates (flow diagram, plain-language evidence,
   inline confirm) on top of the existing plan/apply hooks.
3. Decision card for transfer/swap candidates (same skeleton, pair/dismiss).
4. Residual follow-up card + conflict chooser.
5. History unification; move rules/bulk/saved-views/coverage/lineage/manual
   form under Advanced.
6. Delete the now-dead layout paths from `SwapMatching.tsx` /
   `CustodyGaps.tsx`; en/de i18n in lockstep per surface.
