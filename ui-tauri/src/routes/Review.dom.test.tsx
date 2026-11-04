// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

const daemon = vi.hoisted(() => ({
  badges: {} as Record<string, unknown>,
  candidates: [] as unknown[],
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind }: { kind: string }) => {
        if (kind === "ui.review.badges") return { kind, schema_version: 1, data: daemon.badges };
        if (kind === "ui.transfers.suggest") {
          return {
            kind,
            schema_version: 1,
            data: { candidates: daemon.candidates, counts: { total: daemon.candidates.length, exact: 0, strong: 0, conflicts: 0 } },
          };
        }
        return { kind, schema_version: 1, data: {} };
      },
    }),
  };
});

// Each section has its own tests; here only whether the screen says it is clear.
vi.mock("@/components/kb/quarantine", () => ({ QuarantineSection: () => null }));
vi.mock("@/components/review/ReviewTransfers", () => ({ ReviewTransfers: () => null }));
vi.mock("@/hooks/useJournalProcessingAction", () => ({
  useJournalProcessingAction: () => ({ runJournalProcessing: vi.fn(), isProcessingJournals: false }),
}));

import { Review } from "./Review";

function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <Review />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  daemon.badges = {
    quarantine: 0,
    quarantine_attention: 0,
    journals_needs_processing: false,
    swaps: 0,
    review: 0,
  };
  daemon.candidates = [];
});

afterEach(cleanup);

describe("review screen", () => {
  it("says nothing waits when nothing is held, journals are current and no transfer is open", async () => {
    mount();
    expect(await screen.findByText("Nothing to review")).toBeTruthy();
  });

  it("is not clear while the journals need recalculating", async () => {
    daemon.badges = { ...daemon.badges, journals_needs_processing: true };
    mount();
    await waitFor(() => expect(screen.queryByText("Nothing to review")).toBeNull());
  });

  it("is not clear while a transfer is open, even when the cached count says none", async () => {
    daemon.candidates = [{ out_id: "o", in_id: "i" }];
    mount();
    // Give both reads time to land before asserting the absence.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByText("Nothing to review")).toBeNull();
  });
});
