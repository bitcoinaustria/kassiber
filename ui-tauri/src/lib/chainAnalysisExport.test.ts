import { afterEach, describe, expect, it, vi } from "vitest";
import type { AnalysisResult } from "./chainAnalysis";
import { exportChainAnalysis } from "./chainAnalysisExport";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), native: true }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/filePicker", () => ({
  get isFileSaveAvailable() {
    return mocks.native;
  },
}));

const result = {
  schema_version: 1,
  snapshot_id: "abc123",
  query: {},
  nodes: [],
  edges: [],
  findings: [],
  frontier: [],
  clusters: [],
  paths: [],
  summary: {},
  coverage: {},
  capabilities: {},
} as unknown as AnalysisResult;

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  mocks.invoke.mockReset();
  mocks.native = true;
});

describe("investigation artifact export", () => {
  it("hands contents and format to the native dialog without a renderer-controlled destination", async () => {
    mocks.invoke.mockResolvedValue("/user/chosen/result.json");
    expect(
      await exportChainAnalysis(result, "json", "Save investigation"),
    ).toBe("saved");
    expect(mocks.invoke).toHaveBeenCalledWith("save_chain_analysis_export_as", {
      contents: JSON.stringify(result, null, 2) + "\n",
      format: "json",
      defaultName: "kassiber-chain-analysis-abc123.json",
      title: "Save investigation",
    });
    expect(mocks.invoke.mock.calls[0][1]).not.toHaveProperty("destinationPath");
  });
  it("preserves native cancellation without reporting a saved artifact", async () => {
    mocks.invoke.mockResolvedValue(null);
    expect(await exportChainAnalysis(result, "csv", "Save investigation")).toBe(
      "cancelled",
    );
    expect(mocks.invoke.mock.calls[0][1].contents).toContain('"amount_msat"');
  });
  it("downloads an actual browser JSON artifact when native dialogs are unavailable", async () => {
    mocks.native = false;
    const anchor = { href: "", download: "", click: vi.fn(), remove: vi.fn() };
    const append = vi.fn();
    vi.stubGlobal("document", {
      createElement: vi.fn(() => anchor),
      body: { append },
    });
    vi.stubGlobal("window", { setTimeout: vi.fn() });
    const create = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:synthetic-test");
    expect(
      await exportChainAnalysis(result, "json", "Save investigation"),
    ).toBe("download");
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(anchor.download).toBe("kassiber-chain-analysis-abc123.json");
    expect(anchor.click).toHaveBeenCalledOnce();
    expect(append).toHaveBeenCalledWith(anchor);
    const blob = create.mock.calls[0][0] as Blob;
    expect(JSON.parse(await blob.text())).toEqual(result);
  });
});
