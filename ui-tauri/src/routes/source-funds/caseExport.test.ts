import { describe, expect, it, vi } from "vitest";
import { exportCurrentCase } from "./caseExport";
import type { SourceFundsPreview } from "./model";
const saved = { case: { id: "frozen-case", status: "saved", snapshot_hash: "hash" } } as SourceFundsPreview;

describe("source-funds frozen export continuation", () => {
  it("exports only the saved snapshot and returns the verified file result", async () => {
    const render = vi.fn(async () => ({ filename: "report.pdf" }));
    expect(await exportCurrentCase({ save: async () => saved, render, isCurrent: () => true })).toEqual({ savedCase: saved.case, output: { filename: "report.pdf" } });
    expect(render).toHaveBeenCalledWith({ case: "frozen-case" });
  });
  it("does not start export after the reviewed recipe or mounted book changes during save", async () => {
    let current = true; const render = vi.fn();
    const result = await exportCurrentCase({ save: async () => { current = false; return saved; }, render, isCurrent: () => current });
    expect(result).toBeNull(); expect(render).not.toHaveBeenCalled();
  });
  it("does not display a completed file in a replacement case", async () => {
    let current = true;
    expect(await exportCurrentCase({ save: async () => saved, render: async () => { current = false; return { filename: "old.pdf" }; }, isCurrent: () => current })).toBeNull();
  });
  it("keeps failures visible to the caller and never renders an unsaved preview", async () => {
    const render = vi.fn();
    expect(await exportCurrentCase({ save: async () => undefined, render, isCurrent: () => true })).toBeNull();
    await expect(exportCurrentCase({ save: async () => { throw new Error("stale"); }, render, isCurrent: () => true })).rejects.toThrow("stale");
    expect(render).not.toHaveBeenCalled();
  });
});
