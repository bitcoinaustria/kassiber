import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import en from "@/i18n/locales/en/journals.json";
import de from "@/i18n/locales/de/journals.json";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";

import {
  CAUSE_KEYS,
  KNOWN_QUANTITY_BLOCKERS,
  KNOWN_QUARANTINE_REASONS,
  categoryForReason,
  causeCopy,
  causeKeyFor,
  detailContextFor,
  exclusionFitsReason,
  sheetTabForCause,
} from "./explain";
import { quarantineItemToRow } from "./model";
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

  it("takes priority, status and the root link from the daemon", () => {
    const row = quarantineItemToRow(item, "Book", t);
    expect(row.status).toBe("Needs review");
    expect(row.priority).toBe("Low");
    expect(row.impact).toBe("Clears with its cause");
    expect(row.event).toBe("Waiting for an earlier problem");
    expect(row.evidenceHint).toContain("2024-01-01, A");
    expect(row.transactionAction).toMatchObject({
      transactionId: "out",
      tab: "details",
      reviewReason: "custody_quantity_unresolved",
      label: "Open cause",
    });
    const root = quarantineItemToRow(
      {
        ...item,
        transaction_id: "out",
        reason: "custody_quantity_unresolved",
        category: "missing_wallet_history",
        blocks_reports: true,
        is_downstream: false,
        root: null,
        evidence: { blocker_code: "custody_gap_review_required", wallet_label: "A" },
        actions: [{ kind: "connect_wallet" }],
      },
      "Book",
      t,
    );
    expect(root.status).toBe("Blocked");
    expect(root.priority).toBe("High");
    expect(root.event).toBe("An intermediate wallet is missing");
    expect(root.transactionAction?.tab).toBe("details");
    // The table button only opens the row; "Connect wallet" lives in the panel.
    expect(root.transactionAction?.label).toBe("Open transaction");
  });

  it("flags explanations that quote amounts so tables can mask them", () => {
    const oversell = quarantineItemToRow(
      {
        ...item,
        reason: "insufficient_lots",
        category: "missing_acquisition_history",
        is_downstream: false,
        root: null,
        evidence: { required_msat: 123_456_789_000, available_msat: 87_654_321_000 },
      },
      "Book",
      t,
    );
    expect(oversell.evidenceHint).toContain("123,456,789");
    expect(oversell.evidenceHintSensitive).toBe(true);
    expect(quarantineItemToRow(item, "Book", t).evidenceHintSensitive).toBe(false);
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
