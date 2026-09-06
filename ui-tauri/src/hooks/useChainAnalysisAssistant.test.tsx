import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import { AssistantSessionContext, type AssistantSessionContextValue } from "@/components/ai/assistantSession";
import { DaemonScopeContext } from "@/daemon/client";
import { DEFAULT_ANALYSIS_QUERY } from "@/lib/chainAnalysis";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useChainAnalysisAssistant } from "./useChainAnalysisAssistant";

const mock = vi.hoisted(() => ({ project: vi.fn(), kinds: [] as string[] }));
vi.mock("@/daemon/client", async importOriginal => ({
  ...await importOriginal<typeof import("@/daemon/client")>(),
  useDaemonMutation: (kind: string) => {
    mock.kinds.push(kind);
    return { mutateAsync: mock.project, isPending: false };
  },
}));
const investigation = { query: { ...DEFAULT_ANALYSIS_QUERY, observer: "public" as const, subject: "raw-chain-identity" }, snapshot_id: "raw-snapshot" };
function harness({ current = () => true, model = true, streaming = false, overviewPrompt }: { current?: () => boolean; model?: boolean; streaming?: boolean; overviewPrompt?: string } = {}) {
  const sendPrompt = vi.fn(), error = vi.fn();
  let result: ReturnType<typeof useChainAnalysisAssistant>;
  function Probe() { result = useChainAnalysisAssistant(error, overviewPrompt); return null; }
  renderToStaticMarkup(
    <DaemonScopeContext.Provider value={{ expectedScope: { workspace_id: "w", profile_id: "p" }, daemonSession: 1, isCurrent: current }}>
      <AssistantSessionContext.Provider value={{ sendPrompt, selection: model ? { provider: "remote", model: "model" } : null, isStreaming: streaming } as unknown as AssistantSessionContextValue}>
        <Probe />
      </AssistantSessionContext.Provider>
    </DaemonScopeContext.Provider>,
  );
  return { action: result!, sendPrompt, error };
}
const projected = { data: { query: { subject: "opaque-node", observer: "public" }, subject: "opaque-node", snapshot_id: "opaque-snapshot" } };
beforeEach(() => { mock.project.mockReset(); mock.kinds.length = 0; useAssistantDraftStore.getState().setDraft(""); });

describe("shared investigation assistant handoff", () => {
  it("sends only provider-projected identities after checking the exact source snapshot", async () => {
    mock.project.mockResolvedValue(projected);
    const { action, sendPrompt } = harness();
    await action.ask(investigation, "raw-chain-identity");
    expect(mock.kinds).toEqual(["ui.chain_analysis.ai_context"]);
    expect(mock.project).toHaveBeenCalledWith({ query: investigation.query, expected_snapshot_id: "raw-snapshot", subject: "raw-chain-identity" });
    const prompt = sendPrompt.mock.calls[0][0];
    expect(prompt).toContain("opaque-node");
    expect(prompt).toContain("opaque-snapshot");
    expect(prompt).not.toContain("raw-chain-identity");
    expect(prompt).not.toContain("raw-snapshot");
  });
  it("does not send a late projection into a different book", async () => {
    let current = true;
    let resolve!: (value: typeof projected) => void;
    mock.project.mockImplementation(() => new Promise(value => { resolve = value; }));
    const { action, sendPrompt } = harness({ current: () => current });
    const pending = action.ask(investigation);
    current = false;
    resolve(projected);
    await pending;
    expect(sendPrompt).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toBe("");
  });
  it("fails closed before projection when scope is stale and after projection when the snapshot guard rejects", async () => {
    const stale = harness({ current: () => false });
    await stale.action.ask(investigation);
    expect(mock.project).not.toHaveBeenCalled();
    const active = harness();
    mock.project.mockRejectedValue(new Error("chain_analysis_snapshot_changed"));
    await active.action.ask(investigation);
    expect(active.error).toHaveBeenCalledOnce();
    expect(active.sendPrompt).not.toHaveBeenCalled();
  });
  it("keeps only a projected draft when no model has been selected", async () => {
    mock.project.mockResolvedValue(projected);
    const { action, sendPrompt } = harness({ model: false });
    await action.ask(investigation);
    expect(sendPrompt).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toContain("opaque-snapshot");
    expect(useAssistantDraftStore.getState().draft).not.toContain("raw-chain-identity");
  });
  it("prevents duplicate handoffs while projection is in flight", async () => {
    let resolve!: (value: typeof projected) => void;
    mock.project.mockImplementation(() => new Promise(value => { resolve = value; }));
    const { action, sendPrompt } = harness();
    const first = action.ask(investigation);
    await action.ask(investigation);
    expect(mock.project).toHaveBeenCalledOnce();
    resolve(projected);
    await first;
    expect(sendPrompt).toHaveBeenCalledOnce();
  });
  it("uses a static report-tool prompt for the Mirror overview without copying its payload", async () => {
    const prompt = "Read ui_reports_privacy_mirror using local evidence.";
    const { action, sendPrompt } = harness({ overviewPrompt: prompt });
    await action.ask();
    expect(mock.project).not.toHaveBeenCalled();
    expect(sendPrompt).toHaveBeenCalledWith(prompt);
    const streaming = harness({ streaming: true, overviewPrompt: prompt });
    await streaming.action.ask();
    expect(streaming.sendPrompt).not.toHaveBeenCalled();
  });
});
