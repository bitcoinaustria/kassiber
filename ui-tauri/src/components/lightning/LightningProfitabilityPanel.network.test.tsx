import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW } from "@/mocks/seed";

import { LightningProfitabilityPanel } from "./LightningProfitabilityPanel";

const mocks = vi.hoisted(() => ({
  useDaemon: vi.fn(),
  refetch: vi.fn(),
}));

vi.mock("@/daemon/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/client")>();
  return { ...actual, useDaemon: mocks.useDaemon };
});

describe("Lightning profitability network consent", () => {
  it("does not read the node when the Reports page mounts", () => {
    // `/reports` renders this panel unconditionally and the panel
    // auto-selects the first reportable connection, so an enabled query here
    // means opening Reports RPCs the node.
    mocks.useDaemon.mockImplementation((kind: string) =>
      kind === "ui.overview.snapshot"
        ? { data: { kind, data: MOCK_OVERVIEW }, isLoading: false }
        : {
            data: undefined,
            error: null,
            isError: false,
            isFetched: false,
            isFetching: false,
            refetch: mocks.refetch,
          },
    );

    renderToStaticMarkup(<LightningProfitabilityPanel />);

    const profitabilityCalls = mocks.useDaemon.mock.calls.filter(
      ([kind]) => kind === "ui.reports.lightning_profitability",
    );
    expect(profitabilityCalls.length).toBeGreaterThan(0);
    for (const call of profitabilityCalls) {
      expect(call[2]).toEqual(expect.objectContaining({ enabled: false }));
    }
    expect(mocks.refetch).not.toHaveBeenCalled();
  });
});
