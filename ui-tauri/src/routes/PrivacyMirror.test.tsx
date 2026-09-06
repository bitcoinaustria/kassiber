import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import i18n from "@/i18n";
import { router } from "@/routeTree";
import { DEFAULT_ANALYSIS_QUERY } from "@/lib/chainAnalysis";
import { groupPrivacyFindings, type PrivacyFinding, type PrivacyMirrorPayload } from "@/lib/privacyMirror";
import { PrivacyMirrorPayloadView } from "./PrivacyMirror";

const investigation = { query: { ...DEFAULT_ANALYSIS_QUERY, observer: "public" as const, include_relations: false }, snapshot_id: "local-snapshot" };
const finding = (overrides: Partial<PrivacyFinding> = {}): PrivacyFinding => ({ id: "one", code: "common_input", category: "linkage", severity: "warning", authority: "heuristic", relevance: "own_spend", title: "Common-input linkage", detail: "Inputs may belong to one participant only if the collaboration assumptions hold.", assumptions: ["Known collaborative transactions are excluded."], limitations: ["Unrecognized Payjoin remains possible."], affected_output_count: 2, investigation, ...overrides });
const payload = (overrides: Partial<PrivacyMirrorPayload> = {}): PrivacyMirrorPayload => ({
  payload_schema_version: 2, local_only: true, read_only: true, advisory_only: true, observer: "public", investigation,
  summary: { status: "findings", finding_count: 2, attention_count: 1, owned_output_count: 5, analyzed_transaction_count: 2, local_transaction_count: 4, domain_count: 1 },
  findings: [finding(), finding({ id: "two", code: "postmix", category: "pattern", title: "Nearby postmix consolidation", relevance: "received_context", severity: "info" })],
  coverage: { status: "partial", examined_transactions: 2, available_transactions: 4, missing_nodes: 3, stale_nodes: 0, conflicting_nodes: 0, truncated: true, stopped_reasons: ["node_limit"], checks: [{ code: "patterns", status: "partial", evaluated: 2, eligible: 4, reason: "missing_history" }] },
  entropy: { evaluated: 0, eligible: 0, omitted: 0, results: [] }, assumptions: ["Private wallet change metadata is not public knowledge."], ...overrides,
});
const render = (value = payload(), error?: string) => renderToStaticMarkup(<PrivacyMirrorPayloadView payload={value} refreshError={error} onInvestigate={() => {}} onAsk={() => {}} />);

describe("Privacy Mirror exposure report", () => {
  it("translates canonical finding bodies, patterns and computation limits in German", async () => {
    const previous = i18n.language;
    await i18n.changeLanguage("de");
    try {
      const value = payload();
      value.findings = [finding({ code: "common_input_control", detail: "Observed inputs are spent together.", assumptions: ["observed_co_spend"], limitations: [] }), finding({ id: "pattern", code: "fan_in", category: "pattern", detail: "Inputs from multiple observed parent transactions are spent together." })];
      value.coverage.stopped_reasons = ["unobserved_successor", "entropy_state_budget"];
      const html = render(value);
      expect(html).toContain("Beobachtete Inputs werden gemeinsam ausgegeben.");
      expect(html).toContain("Inputs treffen zusammen");
      expect(html).toContain("Das belegt die Topologie, nicht gemeinsames Eigentum.");
      expect(html).toContain("Zustandsbudget");
      expect(html).not.toContain("Observed inputs");
      expect(html).not.toContain("Inputs from multiple");
      expect(html).not.toContain("entropy state budget");
    } finally { await i18n.changeLanguage(previous); }
  });
  it("keeps concrete findings, observer assumptions and actual executed coverage together", () => {
    const html = render();
    expect(html).toContain("Public blockchain observer");
    expect(html).toContain("Common-input linkage");
    expect(html).toContain("Conditional inference");
    expect(html).toContain("Unrecognized Payjoin remains possible.");
    expect(html).toContain("Nearby postmix consolidation");
    expect(html).toContain('data-testid="privacy-mirror-context"');
    expect(html).toContain('data-relevance="received_context"');
    expect(html).toContain("What was examined");
    expect(html).toContain("Partial");
    expect(html).not.toContain("/ 100");
    expect(html).not.toContain("A+");
  });
  it("does not turn an empty or incomplete population into a privacy grade", () => {
    const empty = payload({ findings: [], summary: { status: "unavailable", finding_count: 0, attention_count: 0, owned_output_count: 0, analyzed_transaction_count: 0, local_transaction_count: 0, domain_count: 0 } });
    const html = render(empty);
    expect(html).toContain("More local evidence is needed");
    expect(html).not.toContain("No exposure identified");
    expect(html).not.toContain("A+");
    expect(html).not.toContain("privacy-score");
  });
  it("keeps a previous result visibly stale after refresh fails", () => {
    const html = render(payload(), "Local snapshot unavailable");
    expect(html).toContain('role="alert"');
    expect(html).toContain("These are the previous results");
    expect(html).toContain("Local snapshot unavailable");
    expect(html).toContain("Common-input linkage");
  });
  it("links to the single workbench instead of recreating tools and catalogs", () => {
    const html = render();
    expect(html).toContain("Open Chain Analysis");
    expect(html).toContain("Check a draft spend");
    expect(html).toContain("Attribution sources");
    expect(html).not.toContain("<textarea");
    expect(html).not.toContain("What's linkable");
    expect(html).not.toContain("Analysis capabilities");
    expect(html).not.toContain("How this score is built");
  });
  it("shows conditional interpretation results without publishing raw subject identities or an anonymity score", () => {
    const html = render(payload({ entropy: { evaluated: 1, eligible: 2, omitted: 1, results: [{ subject: "private-transaction-identity", status: "exact", interpretation_count: "3", entropy_bits: 1.585, conditional_on_model: true, investigation }] } }));
    expect(html).toContain("3 interpretations under the selected model");
    expect(html).toContain("not ownership probabilities or anonymity scores");
    expect(html).toContain("outside this report&#x27;s analysis budget");
    expect(html).not.toContain("private-transaction-identity");
    expect(html).not.toContain("1.585");
  });
  it.each(["budget_exhausted", "timeout", "model_bounded"])("labels a %s interpretation count as a lower bound", status => {
    const html = render(payload({ entropy: { evaluated: 1, eligible: 1, omitted: 0, results: [{ subject: "local-subject", status, interpretation_count_lower_bound: "2", investigation }] } }));
    expect(html).toContain("At least 2 interpretations found; search incomplete");
    expect(html).not.toContain("2 interpretations under the selected model");
  });
  it("groups repetition without collapsing public authority or ownership relevance", () => {
    const first = finding();
    const second = finding({ id: "second", assumptions: ["A separate premise."] });
    const context = finding({ id: "context", relevance: "received_context" });
    const observed = finding({ id: "observed", authority: "observed" });
    expect(groupPrivacyFindings([first, second, context, observed])).toEqual([[first, second], [context], [observed]]);
    const html = render(payload({ findings: [first, second, context, observed] }));
    expect(html).toContain("2 findings");
    expect(html).toContain("A separate premise.");
    expect(html.match(/data-testid="privacy-finding"/g)).toHaveLength(4);
  });
  it("registers its route and preserves public Workbench search fields", () => {
    expect(router.routesByPath["/privacy-mirror"]).toBeTruthy();
    const validate = router.routesByPath["/chain-analysis"].options.validateSearch;
    expect(typeof validate).toBe("function");
    if (typeof validate === "function") expect(validate({ observer: "public", chain: "bitcoin", network: "regtest", workspace: "psbt" })).toEqual({ observer: "public", chain: "bitcoin", network: "regtest", workspace: "psbt" });
  });
});
