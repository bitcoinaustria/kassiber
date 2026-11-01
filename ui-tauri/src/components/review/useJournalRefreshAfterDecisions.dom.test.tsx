// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

const daemon = vi.hoisted(() => ({
  streams: [] as Array<{ kind: string; args?: Record<string, unknown> }>,
  hookOptions: [] as unknown[],
  runs: 0,
}));

// The processing hook's own run is covered by its tests; here only how
// this hook asks for it.
vi.mock("@/hooks/useJournalProcessingAction", () => ({
  useJournalProcessingAction: (options: unknown) => {
    daemon.hookOptions.push(options);
    return { runJournalProcessing: () => (daemon.runs += 1), isProcessingJournals: false };
  },
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind }: { kind: string }) => ({ kind, schema_version: 1, data: {} }),
      stream: async (request: { kind: string; args?: Record<string, unknown> }) => {
        daemon.streams.push(request);
        return { kind: request.kind, schema_version: 1, data: {} };
      },
    }),
  };
});

import { useJournalRefreshAfterDecisions } from "./useJournalRefreshAfterDecisions";

let request: () => void = () => undefined;

function Probe() {
  request = useJournalRefreshAfterDecisions().requestRefresh;
  return null;
}

function mount() {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Probe />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  daemon.streams = [];
  daemon.hookOptions = [];
  daemon.runs = 0;
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("journal refresh after review decisions", () => {
  it("recalculates once the owner stops deciding, without pairing anything itself", async () => {
    mount();
    act(() => request());
    act(() => request());
    expect(daemon.runs).toBe(0);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(daemon.runs).toBe(1);
    expect(daemon.hookOptions[0]).toEqual({ autoPair: false });
  });

  it("still recalculates when the screen closes before the run started", async () => {
    const view = mount();
    act(() => request());
    view.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(daemon.streams).toEqual([
      expect.objectContaining({ kind: "ui.journals.process", args: { auto_pair: false } }),
    ]);
  });
});
