import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";

const reads = vi.hoisted(() => vi.fn());
vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string, args: unknown) => {
    reads(kind, args);
    return kind === "ui.custody.gaps.review_context"
      ? { isError: true, error: new Error("Selected gap is stale; refresh the review queue") }
      : {};
  },
  useDaemonInfinite: (kind: string) => ({
    data: { pages: kind === "ui.custody.gaps.list" ? [{ data: {
      summary: { total: 0, needs_review: 0, conflicting: 0, resolved: 0, dismissed: 0, unresolved_msat: "0", derived_state_current: false },
      gaps: [],
    } }] : [] },
  }),
  useDaemonMutation: () => ({}),
}));

import { CustodyGapsContent } from "./CustodyGaps";

describe("custody gap navigation", () => {
  beforeEach(() => reads.mockClear());

  it("resolves a deep-linked gap outside the loaded page and surfaces stale evidence", () => {
    const html = renderToStaticMarkup(<CustodyGapsContent focusGapId="gap-outside-page" />);
    expect(reads).toHaveBeenCalledWith("ui.custody.gaps.review_context", { gap_id: "gap-outside-page" });
    expect(html).toContain("Selected gap is stale; refresh the review queue");
    expect(html).not.toContain("Create reviewed bridge");
  });

  it("does not fetch a selected review when the route has no gap", () => {
    renderToStaticMarkup(<CustodyGapsContent focusGapId={null} />);
    expect(reads.mock.calls.some(([kind]) => kind === "ui.custody.gaps.review_context")).toBe(false);
  });
});
