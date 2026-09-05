import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "@/i18n";
import { TargetStage, DiscloseStage, ExportStage } from "./stages";
import { useUiStore } from "@/store/ui";
vi.mock("@/store/ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/store/ui")>();
  const store = Object.assign((selector: (state: ReturnType<typeof actual.useUiStore.getState>) => unknown) => selector(actual.useUiStore.getState()), actual.useUiStore);
  return { ...actual, useUiStore: store };
});
const mocks = vi.hoisted(() => ({ reads: [] as { kind: string; args: unknown; options: unknown }[], responses: {} as Record<string, unknown>, pages: [{ data: { txs: [{ id: "first-unrelated", amount: 2 }] } }] }));
vi.mock("@/daemon/client", async () => ({
  DaemonScopeContext: (await import("react")).createContext(null),
  useDaemon: (kind: string, args: unknown, options: unknown) => { mocks.reads.push({ kind, args, options }); return { data: mocks.responses[kind] ? { data: mocks.responses[kind] } : undefined }; },
  useDaemonInfinite: (kind: string, args: unknown, next: (page: unknown) => unknown) => { mocks.reads.push({ kind, args, options: next({ data: { nextCursor: "second-page" } }) }); return { data: { pages: mocks.pages }, hasNextPage: true }; },
  useDaemonMutation: () => ({ mutateAsync: vi.fn(), reset: vi.fn(), isPending: false }),
}));
import { useSourceFundsCase, type SourceFundsCaseState } from "./useSourceFundsCase";
let state: SourceFundsCaseState;
function Capture({ target = "" }: { target?: string }) { state = useSourceFundsCase("scoped-draft", target); return null; }
function read(kind: string) { return mocks.reads.find((row) => row.kind === kind); }

beforeEach(() => { mocks.reads = []; mocks.responses = {}; mocks.pages = [{ data: { txs: [{ id: "first-unrelated", amount: 2 }] } }]; useUiStore.setState({ sourceFundsDrafts: {} }); });
afterEach(async () => { await i18n.changeLanguage("en"); });
describe("source-funds canonical case reads", () => {
  it("never substitutes the first transaction when no target was selected", () => {
    renderToStaticMarkup(<Capture />);
    expect(state.selectedTarget).toBe(""); expect(state.selectedTx).toBeUndefined();
    expect(read("ui.source_funds.review_context")?.options).toMatchObject({ enabled: false });
  });
  it("preserves an explicit missing target instead of selecting an unrelated row", () => {
    renderToStaticMarkup(<Capture target="missing" />);
    expect(state.selectedTarget).toBe("missing"); expect(state.selectedTx).toBeUndefined();
    expect(read("ui.source_funds.review_context")?.args).toMatchObject({ target_transaction: "missing" });
    expect(read("ui.transactions.resolve")?.options).toMatchObject({ enabled: false });
  });
  it("resolves the canonical target outside the loaded pages and scopes links to its investigation", () => {
    mocks.responses["ui.source_funds.review_context"] = { target: { transaction_id: "old-canonical" }, links: [{ id: "relevant" }], sources: [], evidence: [], report: { explain_gates: { blockers: [], warnings: [], exportable: false } } };
    mocks.responses["ui.transactions.resolve"] = { transaction: { id: "old-canonical", amount: 0.1 } };
    renderToStaticMarkup(<Capture target="external-txid" />);
    expect(state.selectedTxId).toBe("old-canonical"); expect(state.selectedTx?.id).toBe("old-canonical");
    expect(read("ui.transactions.resolve")?.args).toEqual({ query: "old-canonical" });
    expect(state.reviewQueueLinks.map((link) => link.id)).toEqual(["relevant"]);
    expect(read("ui.source_funds.links.list")).toBeUndefined();
  });
  it("keeps whole-book coverage, manual catalogs and printable SVGs lazy", () => {
    renderToStaticMarkup(<Capture target="selected" />);
    for (const kind of ["ui.source_funds.coverage", "ui.source_funds.sources.list", "ui.source_funds.evidence.list", "ui.source_funds.preview"]) expect(read(kind)?.options).toMatchObject({ enabled: false });
    expect(read("ui.transactions.list")?.options).toBe("second-page");
  });
  it("does not inherit another case amount when opened through an explicit target route", () => {
    useUiStore.setState({ sourceFundsDrafts: { "scoped-draft": { target: "other", targetAmount: "2", plannedDestination: "Previous recipient" } } });
    renderToStaticMarkup(<Capture target="new-target" />);
    expect(state.selectedTarget).toBe("new-target"); expect(state.targetAmount).toBe(""); expect(state.plannedDestination).toBe("");
  });
  it("renders the target, disclosure and export controls in German", async () => {
    await i18n.changeLanguage("de"); renderToStaticMarkup(<Capture />);
    const html = renderToStaticMarkup(<><TargetStage state={state} /><DiscloseStage state={state} /><ExportStage state={state} /></>);
    expect(html).toContain("Was soll erklärt werden?"); expect(html).toContain("Genaue Transaktionsreferenz"); expect(html).toContain("Ein Empfänger ist optional");
    expect(html).not.toContain("Report options"); expect(html).not.toContain("No recipients defined"); expect(html).not.toContain("Saving case");
  });
  it("restores only the explicitly scoped draft and flattens all loaded pages", () => {
    useUiStore.setState({ sourceFundsDrafts: { default: { target: "unsafe-legacy" }, "scoped-draft": { target: "saved-target", targetAmount: "0.00000001" } } });
    mocks.pages.push({ data: { txs: [{ id: "page-two", amount: 0.2 }] } });
    renderToStaticMarkup(<Capture />);
    expect(state.selectedTarget).toBe("saved-target"); expect(state.targetAmount).toBe("0.00000001"); expect(state.rows).toHaveLength(2);
  });
});
