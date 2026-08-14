import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW } from "@/mocks/seed";

import { ConnectionDetail } from "./ConnectionDetail";

const mocks = vi.hoisted(() => ({
  refetch: vi.fn(),
  useDaemon: vi.fn(),
}));

vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    useParams: () => ({ connectionId: "c3" }),
  };
});

vi.mock("@/daemon/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/client")>();
  return { ...actual, useDaemon: mocks.useDaemon };
});

vi.mock("./NodeConnectionDetail", () => ({
  NodeConnectionDetail: () => <div>Node connection</div>,
}));

describe("ConnectionDetail network consent", () => {
  it("does not fetch a Lightning node snapshot on mount", () => {
    mocks.useDaemon.mockImplementation((kind: string) =>
      kind === "ui.overview.snapshot"
        ? { data: { kind, data: MOCK_OVERVIEW }, isLoading: false }
        : {
            data: undefined,
            error: null,
            isFetching: false,
            refetch: mocks.refetch,
          },
    );

    renderToStaticMarkup(<ConnectionDetail />);

    expect(mocks.useDaemon).toHaveBeenCalledWith(
      "ui.connections.node.snapshot",
      { connection: "c3" },
      expect.objectContaining({ enabled: false }),
    );
    expect(mocks.refetch).not.toHaveBeenCalled();
  });
});
