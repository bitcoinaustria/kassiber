import { describe, expect, it } from "vitest";

import {
  AIE_HEURISTIC_COVERAGE,
  heuristicAvailableCount,
  gradeForScore,
  privacyScoreModel,
} from "./privacyScore";
import type { PrivacyMirrorPayload } from "./privacyMirror";

describe("privacyScore", () => {
  it("describes available surfaces without claiming checks were executed", () => {
    expect(heuristicAvailableCount()).toBe(20);
    expect(AIE_HEURISTIC_COVERAGE.find((item) => item.id === "h4")?.status).toBe("mirror");
    expect(AIE_HEURISTIC_COVERAGE.find((item) => item.id === "h6")?.status).toBe("mirror");
    for (const id of ["h5", "peel", "postmix", "entity", "consolidation", "bip69"])
      expect(AIE_HEURISTIC_COVERAGE.find((item) => item.id === id)?.status).toBe("chain_analysis_workspace");
    expect(AIE_HEURISTIC_COVERAGE.find((item) => item.id === "anon")?.status).toBe("not_implemented");
    expect(AIE_HEURISTIC_COVERAGE.every((item) => !["computed", "partial"].includes(item.status))).toBe(true);
  });

  it("leaves an empty payload ungraded instead of inventing an evaluable population", () => {
    const model = privacyScoreModel({});
    expect(model.score).toBeNull();
    expect(model.grade).toBeNull();
    expect(model.findings).toHaveLength(0);
    expect(model.census).toEqual({ alert: 0, warning: 0, info: 0 });
  });

  it("derives severity from counts and never double-counts the worst risk", () => {
    const payload: PrivacyMirrorPayload = {
      summary: { utxo_count: 2, worst_risk: { severity: "warning", kind: "common_input" } },
      transaction_view: [
        { txid: "a", tell_count: 2, wallet_penalty_count: 2, evidence_level: "exact" },
        { txid: "b", tell_count: 1, wallet_penalty_count: 0, evidence_level: "derived" },
      ],
      unknowns: [{ code: "x", evidence_level: "unknown" }],
      coverage: { degraded: true },
    };
    const model = privacyScoreModel(payload);
    // Only the sender tell is a warning; counterparty context stays informational.
    expect(model.census).toEqual({ alert: 0, warning: 1, info: 3 });
    // Only the observed wallet tell incurs a penalty. Unknown coverage and
    // counterparty context remain visible without being counted as wallet risk.
    expect(model.score).toBe(61);
    expect(model.grade).toBe("C");
    expect(model.findings).toHaveLength(4);
  });

  it("clamps at zero and grades F under heavy penalties", () => {
    const payload: PrivacyMirrorPayload = {
      summary: { utxo_count: 10 },
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
        utxo_count: 2,
        privacy_score: {
          value: 80,
          base: 100,
          coverage_ratio: 0.9,
          factors: [
            { key: "wallet_linkage", linked: 0, total: 2, points: 0 },
            { key: "transaction_leaks", leaking: 1, total: 3, points: -20 },
          ],
        },
      },
      // Findings that WOULD drag a client-side score down are ignored for the value.
      transaction_view: [{ txid: "a", tell_count: 3 }],
      coverage: { source_proximity_known_coin_count: 9, source_proximity_unknown_coin_count: 1 },
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

  it.each([100, null])("leaves a zero-population daemon value %s unavailable while retaining observed facts", (value) => {
    const payload: PrivacyMirrorPayload = {
      summary: {
        utxo_count: 0,
        privacy_score: {
          value,
          coverage_ratio: value === null ? null : 1,
          ...(value === null ? { evaluation_status: "unavailable" as const } : {}),
          factors: [
            { key: "wallet_linkage", linked: 2, total: 0, points: null },
            { key: "transaction_leaks", leaking: 0, total: 0, points: null },
          ],
        },
      },
      transaction_view: [{ txid: "observed", tell_count: 1, wallet_penalty_count: 1 }],
      coverage: { source_proximity_known_coin_count: 0, source_proximity_unknown_coin_count: 0, degraded: true },
    };
    const model = privacyScoreModel(payload);
    expect(model.score).toBeNull();
    expect(model.grade).toBeNull();
    expect(model.coverageRatio).toBeUndefined();
    expect(model.factors[0]).toMatchObject({ linked: 2, total: 0, points: null });
    expect(model.findings.some(finding => finding.txid === "observed")).toBe(true);
  });

  it("does not penalize unknown origins when owned and transaction populations are evaluable", () => {
    const payload: PrivacyMirrorPayload = {
      summary: { utxo_count: 2, privacy_score: {
        value: 100, evaluation_status: "available", coverage_ratio: 0,
        factors: [
          { key: "wallet_linkage", linked: 0, total: 1, points: 0 },
          { key: "transaction_leaks", leaking: 0, total: 2, points: 0 },
        ],
      } },
      coverage: { source_proximity_known_coin_count: 0, source_proximity_unknown_coin_count: 2, degraded: true },
      unknowns: [{ code: "source_proximity_coverage_gaps" }],
    };
    expect(privacyScoreModel(payload)).toMatchObject({ score: 100, grade: "A+", coverageRatio: 0 });
    const withoutDaemon = { ...payload, summary: { utxo_count: 2 } };
    const legacy = privacyScoreModel(withoutDaemon);
    expect(legacy.score).toBe(70);
    expect(legacy.factors).toEqual([]);
  });

  it("withholds the aggregate when one denominator is missing and honors explicit unavailable over a fallback", () => {
    const payload: PrivacyMirrorPayload = {
      summary: { utxo_count: 2, privacy_score: {
        value: 100, coverage_ratio: 1,
        factors: [
          { key: "wallet_linkage", linked: 0, total: 1, points: 0 },
          { key: "transaction_leaks", leaking: 0, total: 0, points: null },
        ],
      } },
      coverage: { source_proximity_known_coin_count: 2, source_proximity_unknown_coin_count: 0 },
    };
    expect(privacyScoreModel(payload).score).toBeNull();
    expect(privacyScoreModel(payload).coverageRatio).toBe(1);
    payload.summary!.privacy_score!.factors![1].total = 1;
    payload.summary!.privacy_score!.evaluation_status = "unavailable";
    expect(privacyScoreModel(payload).score).toBeNull();
    payload.unknowns = [{ code: "no_owned_bitcoin_outputs" }];
    expect(privacyScoreModel(payload).coverageRatio).toBeUndefined();
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
