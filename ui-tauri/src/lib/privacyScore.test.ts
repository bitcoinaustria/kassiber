import { describe, expect, it } from "vitest";

import {
  SCORE_BASE,
  gradeForScore,
  privacyScoreModel,
} from "./privacyScore";
import type { PrivacyMirrorPayload } from "./privacyMirror";

describe("privacyScore", () => {
  it("scores a clean payload at the base and grades it C", () => {
    const model = privacyScoreModel({});
    expect(model.score).toBe(SCORE_BASE);
    expect(model.grade).toBe("C");
    expect(model.findings).toHaveLength(0);
    expect(model.census).toEqual({ alert: 0, warning: 0, info: 0 });
  });

  it("derives severity from counts and never double-counts the worst risk", () => {
    const payload: PrivacyMirrorPayload = {
      summary: { worst_risk: { severity: "warning", kind: "common_input" } },
      transaction_view: [
        { txid: "a", tell_count: 2, wallet_penalty_count: 2, evidence_level: "exact" },
        { txid: "b", tell_count: 1, wallet_penalty_count: 0, evidence_level: "derived" },
      ],
      unknowns: [{ code: "x", evidence_level: "unknown" }],
      coverage: { degraded: true },
    };
    const model = privacyScoreModel(payload);
    // 2 tx tells -> warning, 1 unknown + degraded coverage -> info.
    expect(model.census).toEqual({ alert: 0, warning: 2, info: 2 });
    // 70 - (2 * 9) - (2 * 3) = 46 -> D. The worst risk is NOT re-added.
    expect(model.score).toBe(46);
    expect(model.grade).toBe("D");
    expect(model.findings).toHaveLength(4);
  });

  it("clamps at zero and grades F under heavy penalties", () => {
    const payload: PrivacyMirrorPayload = {
      transaction_view: Array.from({ length: 10 }, (_, index) => ({
        txid: String(index),
        tell_count: 1,
      })),
    };
    const model = privacyScoreModel(payload);
    expect(model.score).toBe(0);
    expect(model.grade).toBe("F");
  });

  it("prefers a grounded daemon score over the client-side fallback", () => {
    const payload: PrivacyMirrorPayload = {
      summary: {
        privacy_score: {
          value: 80,
          base: 100,
          coverage_ratio: 0.9,
          factors: [{ key: "wallet_linkage", linked: 0, total: 2, points: 0 }],
        },
      },
      // Findings that WOULD drag a client-side score down are ignored for the value.
      transaction_view: [{ txid: "a", tell_count: 3 }],
    };
    const model = privacyScoreModel(payload);
    expect(model.grounded).toBe(true);
    expect(model.score).toBe(80);
    expect(model.grade).toBe("B");
    expect(model.coverageRatio).toBe(0.9);
    expect(model.factors[0]?.key).toBe("wallet_linkage");
    // Findings + census still derive from the payload for the cards/ring.
    expect(model.findings).toHaveLength(1);
  });

  it("maps grade boundaries", () => {
    expect(gradeForScore(100)).toBe("A+");
    expect(gradeForScore(90)).toBe("A+");
    expect(gradeForScore(89)).toBe("B");
    expect(gradeForScore(75)).toBe("B");
    expect(gradeForScore(50)).toBe("C");
    expect(gradeForScore(25)).toBe("D");
    expect(gradeForScore(24)).toBe("F");
    expect(gradeForScore(0)).toBe("F");
  });
});
