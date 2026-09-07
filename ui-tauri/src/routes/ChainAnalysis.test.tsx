import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import { DaemonScopeContext } from "@/daemon/client";
import { DEFAULT_ANALYSIS_QUERY, type AnalysisQuery, type AnalysisResult } from "@/lib/chainAnalysis";
import type { AnalysisSearch } from "@/lib/chainAnalysisNavigation";
import { ChainAnalysisWorkbench } from "./ChainAnalysis";

// Server rendering exercises the real component/hooks without a DOM dependency.
// Capture effect lifecycle and state writes to test async cleanup explicitly.
const mock = vi.hoisted(() => ({
  effects: [] as Array<() => void | (() => void)>, writes: [] as unknown[],
  invoke: vi.fn(), kinds: [] as string[],
  controls: null as null | { onRun: () => void; setQuery: (q: AnalysisQuery) => void },
}));
vi.mock("react", async original => {
  const react = await original<typeof import("react")>();
  return { ...react,
    useEffect: (effect: () => void | (() => void)) => { mock.effects.push(effect); },
    useState: <T,>(initial: T | (() => T)) => {
      const [value] = react.useState(initial);
      const [setter] = react.useState(() => (next: unknown) => { mock.writes.push(next); });
      return [value, setter];
    },
  };
});
vi.mock("@tanstack/react-router", () => ({ Link: ({ children }: { children: ReactNode }) => <span>{children}</span> }));
vi.mock("@/daemon/client", async original => ({
  ...await original<typeof import("@/daemon/client")>(),
  useDaemonMutation: (kind: string) => ({ mutateAsync: (args: unknown) => { mock.kinds.push(kind); return mock.invoke(args); }, isPending: false }),
}));
vi.mock("@/hooks/useChainAnalysisAssistant", () => ({ useChainAnalysisAssistant: () => ({ available: false, busy: false }) }));
vi.mock("./chain-analysis/QueryControls", () => ({ QueryControls: (props: NonNullable<typeof mock.controls>) => { mock.controls = props; return null; } }));
vi.mock("./chain-analysis/InvestigationPanels", () => ({ AcquisitionPanel: () => null, SavedInvestigations: () => null }));
vi.mock("./chain-analysis/PsbtPanel", () => ({ PsbtPanel: () => null }));
vi.mock("./chain-analysis/DatasetsPanel", () => ({ DatasetsPanel: () => null }));

function mount(search: AnalysisSearch = {}, isCurrent = () => true) {
  renderToStaticMarkup(<DaemonScopeContext.Provider value={{ expectedScope: { workspace_id: "w", profile_id: "p" }, daemonSession: 1, isCurrent }}><ChainAnalysisWorkbench initialSearch={search} /></DaemonScopeContext.Provider>);
  return mock.effects.at(-1)!;
}
function result(snapshot_id: string, query = DEFAULT_ANALYSIS_QUERY): AnalysisResult {
  return { schema_version: 1, snapshot_id, query, nodes: [], edges: [], findings: [], clusters: [], paths: [], frontier: [], summary: {}, coverage: {}, capabilities: {} };
}
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
beforeEach(() => { mock.effects.length = 0; mock.writes.length = 0; mock.kinds.length = 0; mock.invoke.mockReset(); mock.invoke.mockResolvedValue({ data: result("current") }); });

describe("bounded local investigation entry", () => {
  it("runs one bounded overview, including after StrictMode effect replay", async () => {
    const effect = mount();
    const provisionalCleanup = effect();
    provisionalCleanup?.();
    const cleanup = effect();
    await flush();
    expect(mock.kinds).toEqual(["ui.chain_analysis.query"]);
    expect(mock.invoke).toHaveBeenCalledWith({ ...DEFAULT_ANALYSIS_QUERY, subject: undefined, target: undefined });
    expect(mock.writes).toContainEqual(result("current"));
    cleanup?.();
  });
  it("executes a public regtest handoff once and leaves edited controls explicit", async () => {
    mount({ mode: "trace", subject: "a".repeat(64), chain: "bitcoin", network: "regtest", observer: "public", include_relations: false })();
    await flush();
    expect(mock.invoke).toHaveBeenCalledOnce();
    expect(mock.invoke.mock.calls[0][0]).toMatchObject({ mode: "trace", subject: "a".repeat(64), chain: "bitcoin", network: "regtest", observer: "public", include_relations: false });
    mock.controls!.setQuery({ ...DEFAULT_ANALYSIS_QUERY, depth: 3 });
    await flush();
    expect(mock.invoke).toHaveBeenCalledOnce();
    mock.controls!.onRun();
    await flush();
    expect(mock.invoke).toHaveBeenCalledTimes(2);
  });
  it.each([{ workspace: "psbt" }, { workspace: "datasets" }, { mode: "trace" }, { mode: "path", subject: "a" } ] as AnalysisSearch[])("does not execute graph queries for %j", async search => {
    mount(search)(); await flush(); expect(mock.invoke).not.toHaveBeenCalled();
  });
  it("does not start after scope has changed", async () => {
    mount({}, () => false)(); await flush(); expect(mock.invoke).not.toHaveBeenCalled();
  });
  it("ignores a late result after scope change or unmount", async () => {
    for (const invalidate of ["scope", "unmount"]) {
      let current = true;
      let resolve!: (value: unknown) => void;
      mock.invoke.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
      const cleanup = mount({}, () => current)(); await flush();
      if (invalidate === "scope") current = false; else cleanup?.();
      mock.writes.length = 0;
      resolve({ data: result("obsolete") }); await flush();
      expect(mock.writes).toEqual([]);
      cleanup?.();
    }
  });
  it("a newer explicit request wins over a late entry response", async () => {
    let resolve!: (value: unknown) => void;
    mock.invoke.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
    mount()(); await flush();
    mock.controls!.onRun(); await flush();
    expect(mock.writes).toContainEqual(result("current"));
    mock.writes.length = 0;
    resolve({ data: result("obsolete") }); await flush();
    expect(mock.writes).toEqual([]);
  });
  it("marks retained results stale on failed refresh", async () => {
    mount()(); await flush(); mock.writes.length = 0;
    mock.invoke.mockRejectedValueOnce(new Error("Local snapshot failed"));
    mock.controls!.onRun(); await flush();
    expect(mock.writes).toContain("Local snapshot failed");
    expect(mock.writes).toContain(true);
    expect(mock.writes).not.toContainEqual(result("current"));
  });
});
