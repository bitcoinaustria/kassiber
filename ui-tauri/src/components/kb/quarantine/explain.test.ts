import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import en from "@/i18n/locales/en/journals.json";
import de from "@/i18n/locales/de/journals.json";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";

import {
  CAUSE_KEYS,
  KNOWN_QUANTITY_BLOCKERS,
  KNOWN_QUARANTINE_REASONS,
  actionLabel,
  categoryForReason,
  causeCopy,
  causeFacts,
  causeKeyFor,
  decidedFixes,
  detailContextFor,
  exclusionFitsReason,
  fixOperations,
  isWaiting,
  MAX_FIX_OPERATIONS,
  quarantineRowTarget,
  sheetTabForCause,
} from "./explain";
import type { QuarantineCategory, QuarantineItem } from "./types";

const CATEGORIES: QuarantineCategory[] = [
  "missing_wallet_history",
  "missing_chain_evidence",
  "missing_price",
  "missing_acquisition_history",
  "needs_decision",
  "unsupported",
  "downstream",
];

type CauseBundle = Record<string, Record<string, string>>;

describe("quarantine cause copy", () => {
  it.each([
    ["en", en],
    ["de", de],
  ])("explains every known reason, blocker and category in %s", (_lang, bundle) => {
    const cause = bundle.quarantine.cause as unknown as CauseBundle & {
      category: CauseBundle;
    };
    for (const key of CAUSE_KEYS) {
      for (const field of ["title", "why", "provide"]) {
        expect(cause[key]?.[field], `${key}.${field}`).toBeTruthy();
      }
    }
    for (const category of CATEGORIES) {
      expect(cause.category[category]?.title, category).toBeTruthy();
      expect(
        (bundle.quarantine.category as Record<string, string>)[category],
        category,
      ).toBeTruthy();
    }
    expect(KNOWN_QUARANTINE_REASONS.length).toBeGreaterThan(40);
    expect(KNOWN_QUANTITY_BLOCKERS).toContain("custody_gap_review_required");
  });

  it("never shows a raw reason code, even for a reason it does not know", () => {
    const t = i18n.getFixedT("de", "journals");
    const copy = causeCopy(
      { reason: "future_reason_code", category: "missing_chain_evidence" },
      t,
    );
    expect(copy.title).toBe("Blockchain-Nachweis ist unvollständig");
    expect(`${copy.title} ${copy.why} ${copy.provide}`).not.toMatch(/_/);
  });

  it("names the wallets and amounts the user has to supply", () => {
    const t = i18n.getFixedT("en", "journals");
    expect(
      causeCopy(
        {
          reason: "ownership_transfer_source_missing",
          evidence: { missing_source_wallets: [{ id: "w1", label: "Cold storage" }] },
        },
        t,
      ).provide,
    ).toContain("Sync Cold storage");
    const lots = causeCopy(
      {
        reason: "insufficient_lots",
        evidence: { wallet_label: "Hot", required_msat: 12_000_000_000, available_msat: 1_000_000_000 },
      },
      t,
    );
    expect(lots.why).toBe("Hot sends 12,000,000 sats, but Kassiber only knows 1,000,000 sats of earlier receipts.");
  });

  it("reads custody holds by their blocker, not as a pricing problem", () => {
    expect(causeKeyFor("custody_quantity_unresolved", { blocker_code: "custody_gap_review_required" })).toBe("gapHold");
    expect(categoryForReason("custody_quantity_unresolved", { blocker_code: "custody_gap_review_required" })).toBe(
      "missing_wallet_history",
    );
    expect(causeKeyFor("custody_quantity_unresolved", null, { blocker_code: "custody_component_evidence_drift" })).toBe(
      "componentProblem",
    );
    expect(sheetTabForCause("custody_quantity_unresolved", "missing_wallet_history")).toBe("details");
    expect(sheetTabForCause("custody_basis_barrier", "downstream")).toBe("details");
    expect(sheetTabForCause("missing_spot_price", "missing_price")).toBe("pricing");
    expect(sheetTabForCause("insufficient_lots", "missing_acquisition_history")).toBe("tax");
    expect(sheetTabForCause("ownership_transfer_destination_ambiguous", "needs_decision")).toBe("linked");
  });

  it("offers exclusion only where it can answer the question", () => {
    expect(exclusionFitsReason("missing_spot_price", "missing_price")).toBe(true);
    expect(exclusionFitsReason("non_sale_disposal_kind", "needs_decision")).toBe(true);
    expect(exclusionFitsReason("ownership_transfer_duplicate_outbound", "needs_decision")).toBe(true);
    // Custody decisions and umbrella holds without their blocker never qualify.
    for (const reason of [
      "custody_quantity_unresolved",
      "privacy_hop_unresolved",
      "ownership_transfer_destination_ambiguous",
      "unscoped_transfer_review",
    ]) {
      expect(exclusionFitsReason(reason, categoryForReason(reason)), reason).toBe(false);
    }
    for (const category of [
      "missing_wallet_history",
      "missing_chain_evidence",
      "missing_acquisition_history",
      "downstream",
      "unsupported",
    ] as const) {
      expect(exclusionFitsReason("insufficient_lots", category)).toBe(false);
    }
  });
});

describe("classified quarantine rows", () => {
  const t = i18n.getFixedT("en", "journals");
  const item: QuarantineItem = {
    transaction_id: "later-sale",
    external_id: "",
    occurred_at: "2026-01-01T00:00:00Z",
    confirmed_at: null,
    wallet: "C",
    direction: "outbound",
    asset: "BTC",
    amount: 1,
    amount_msat: 100_000_000_000,
    fee: 0,
    fee_msat: 0,
    reason: "custody_basis_barrier",
    detail: {},
    created_at: "2026-01-02T00:00:00Z",
    category: "downstream",
    blocks_reports: false,
    is_downstream: true,
    root: {
      transaction_id: "out",
      reason: "custody_quantity_unresolved",
      occurred_at: "2024-01-01T00:00:00Z",
      wallet: "A",
      external_id: "out",
    },
    reasons: ["custody_basis_barrier"],
    evidence: {},
    actions: [{ kind: "resolve_root", transaction_id: "out" }],
  };

  it("lists a row that follows a named cause as waiting, and opens it with its reading", () => {
    expect(isWaiting(item)).toBe(true);
    expect(isWaiting({ ...item, root: null })).toBe(false);
    expect(isWaiting({ ...item, is_downstream: false, root: null })).toBe(false);
    const target = quarantineRowTarget(item);
    expect(target.tab).toBe("details");
    expect(target.context?.rootLabel).toBe("2024-01-01, A");
    expect(causeCopy({ reason: item.reason, category: "downstream", rootLabel: "2024-01-01, A" }, t).title).toBe(
      "Waiting on an earlier transaction",
    );
  });

  it("reads a suspense a pair left as the pair, with what was seen, on the Linked tab", () => {
    const evidence = {
      blocker_code: "reviewed_residual_suspense",
      pair_id: "pair-1",
      pair_counterpart_transaction_id: "in",
      pair_txids_differ: true,
      pair_receipt_before_spend: true,
    };
    expect(causeKeyFor("custody_quantity_unresolved", evidence)).toBe("reviewedSuspensePair");
    // Without a pair, the suspense keeps its general reading.
    expect(
      causeKeyFor("custody_quantity_unresolved", { blocker_code: "reviewed_residual_suspense" }),
    ).toBe("reviewedSuspense");
    expect(sheetTabForCause("custody_quantity_unresolved", "needs_decision", evidence)).toBe("linked");
    expect(causeCopy({ reason: "custody_quantity_unresolved", evidence }, t).title).toBe(
      "A transfer pair doesn't add up",
    );
    expect(causeFacts(evidence, t)).toEqual([
      "The two sides are different on-chain transactions; one movement between your wallets has a single transaction id.",
      "The receipt is dated before the payment it is paired with.",
    ]);
    expect(causeFacts({}, t)).toEqual([]);
    expect(actionLabel({ kind: "review_pair", transaction_id: "out", pair_id: "pair-1" }, t)).toBe(
      "Review the pair",
    );
  });

  it("flags explanations that quote amounts so lists can mask them", () => {
    const oversell = causeCopy(
      {
        reason: "insufficient_lots",
        category: "missing_acquisition_history",
        evidence: { required_msat: 123_456_789_000, available_msat: 87_654_321_000 },
      },
      t,
    );
    expect(oversell.why).toContain("123,456,789");
    expect(oversell.whyQuotesAmounts).toBe(true);
    expect(causeCopy({ reason: item.reason, category: "downstream" }, t).whyQuotesAmounts).toBe(false);
  });

  it("keeps the reading that came with a click for a root on another page", () => {
    const opened = {
      transactionId: "out",
      context: {
        reason: "custody_quantity_unresolved",
        category: "needs_decision" as const,
        evidence: { blocker_code: "reviewed_residual_suspense" },
        rootLabel: null,
      },
    };
    // The page holds only the downstream row; its root was opened from the panel.
    expect(detailContextFor("out", [item], opened)).toBe(opened.context);
    expect(detailContextFor("later-sale", [item], opened)?.category).toBe("downstream");
    expect(detailContextFor("other", [item], opened)).toBeNull();
    expect(detailContextFor(null, [item], opened)).toBeNull();
  });
});

describe("quarantine snapshot normalizer", () => {
  it("keeps freshness unknown for an older daemon instead of claiming current", () => {
    const snapshot = normalizeQuarantineSnapshot({ summary: { count: 2 }, items: [] });
    expect(snapshot.summary.freshness).toBeNull();
    expect(snapshot.summary.groups).toEqual([]);
    expect(snapshot.summary.assumptions).toBeNull();
  });

  it("passes groups, freshness and assumptions through", () => {
    const snapshot = normalizeQuarantineSnapshot({
      summary: {
        count: 3,
        offset: 100,
        blocking_count: 1,
        reports_blocked: true,
        freshness: {
          needs_processing: true,
          last_processed_at: "2026-09-01T00:00:00Z",
          last_error: { code: "tax_failed", message: "boom", at: "2026-09-02T00:00:00Z" },
        },
        groups: [
          {
            key: "custody_quantity_unresolved:custody_gap_review_required:gap-1",
            category: "missing_wallet_history",
            reason: "custody_quantity_unresolved",
            root_transaction_id: "out",
            count: 3,
            downstream_count: 2,
            blocks_reports: true,
            wallets: ["A", 5],
            evidence: { gap_id: "gap-1" },
            actions: [{ kind: "review_custody_gap", gap_id: "gap-1" }],
          },
        ],
        assumptions: {
          presumed_external_outbound: {
            count: 1,
            amount_msat: 5,
            items: [{ transaction_id: "pay", wallet: "Hot", amount_msat: 5 }, { wallet: "x" }],
          },
        },
      },
      items: [],
    });
    expect(snapshot.summary.freshness?.last_error?.message).toBe("boom");
    expect(snapshot.summary.offset).toBe(100);
    expect(snapshot.summary.groups?.[0].wallets).toEqual(["A"]);
    expect(snapshot.summary.groups?.[0].actions[0].gap_id).toBe("gap-1");
    expect(snapshot.summary.assumptions?.presumed_external_outbound.items).toHaveLength(1);
    expect(snapshot.summary.assumptions?.unclassified_inbound.count).toBe(0);
  });
});

describe("fixes Kassiber decides itself", () => {
  const row = (id: string, evidence: QuarantineItem["evidence"], downstream = false) =>
    ({ transaction_id: id, is_downstream: downstream, evidence }) as QuarantineItem;

  it("takes only pairs of two different txids, once per pair, never a waiting row", () => {
    const fixes = decidedFixes([
      row("a", { pair_id: "p1", pair_txids_differ: true }),
      // The other held leg of the same pair: one unpair clears both.
      row("b", { pair_id: "p1", pair_txids_differ: true }),
      row("c", { pair_id: "p2", pair_txids_differ: false }),
      row("d", { pair_id: "p3", pair_txids_differ: true }, true),
      row("e", { pair_txids_differ: true }),
      row("f", { pair_id: "p4", pair_txids_differ: true }),
    ]);
    expect(fixes.map((item) => item.transaction_id)).toEqual(["a", "f"]);
    expect(fixOperations(fixes).map((operation) => operation.pair_id)).toEqual(["p1", "p4"]);
    expect(fixOperations(fixes, "chosen")[0].reason).not.toEqual(fixOperations(fixes)[0].reason);
  });

  it("stops at the daemon's bound for one proposal", () => {
    const rows = Array.from({ length: MAX_FIX_OPERATIONS + 5 }, (_, index) =>
      row(`t${index}`, { pair_id: `p${index}`, pair_txids_differ: true }),
    );
    expect(decidedFixes(rows)).toHaveLength(MAX_FIX_OPERATIONS);
  });
});

