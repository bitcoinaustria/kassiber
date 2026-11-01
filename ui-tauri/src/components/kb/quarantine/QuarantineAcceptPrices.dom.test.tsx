// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const daemon = vi.hoisted(() => ({
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
  staleApply: false,
}));

const effects = (count: number) => ({
  entries_count: 10,
  quarantine_count: count,
  quarantines: [],
  report_ready: count === 0,
  wallet_holdings: [],
  journal_digest: "d",
  tax: { fiat_currency: "EUR", jurisdiction: "AT", years: [], years_omitted: 0 },
});

const ARTIFACT = {
  schema_version: 1,
  workspace_id: "ws",
  profile_id: "book",
  base_input_version: 7,
  operations: [{ type: "accept_coarse_prices", reason: "r", transaction_ids: ["a", "b", "c"] }],
  before: effects(5),
  after: effects(0),
  delta: { years: [{ year: 2025, realized_gain_loss: "123.00", income: "10.00" }] },
  digest: "a".repeat(64),
};

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (kind === "ui.review.cases") return { kind, schema_version: 1, data: { input_version: 7 } };
        if (kind === "ui.review.plan") return { kind, schema_version: 1, data: ARTIFACT };
        if (kind === "ui.review.apply" && daemon.staleApply) {
          return {
            kind: "error",
            schema_version: 1,
            error: { code: "review_plan_stale", message: "Book changed", retryable: false },
          };
        }
        return { kind, schema_version: 1, data: { status: "verified" } };
      },
    }),
  };
});

import { useUiStore } from "@/store/ui";

import { AcceptCoarsePricesDialog } from "./QuarantineAcceptPrices";

const SCOPE = { workspace_id: "ws", profile_id: "book" };

function mount(onClose = vi.fn(), hideSensitive = false) {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AcceptCoarsePricesDialog scope={SCOPE} onClose={onClose} hideSensitive={hideSensitive} />
    </QueryClientProvider>,
  );
  return onClose;
}

beforeEach(() => {
  daemon.calls = [];
  daemon.staleApply = false;
});

afterEach(cleanup);

describe("accepting approximate prices", () => {
  it("previews every held price for the book, then applies exactly that preview", async () => {
    const onClose = mount();
    expect(await screen.findByText("Accept 3 approximate prices?")).toBeTruthy();
    expect(screen.getByTestId("review-proposal-effect").textContent).toContain("5");

    const plan = daemon.calls.find((call) => call.kind === "ui.review.plan")!;
    expect(plan.args).toMatchObject({
      operations: [{ type: "accept_coarse_prices" }],
      expected_input_version: 7,
      expected_scope: SCOPE,
    });
    // The daemon names the transactions; the request leaves them to it.
    expect((plan.args.operations as Array<Record<string, unknown>>)[0].transaction_ids).toBeUndefined();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Accept 3 prices" }));
    });
    const apply = daemon.calls.find((call) => call.kind === "ui.review.apply")!;
    expect(apply.args).toMatchObject({ artifact: ARTIFACT, expected_scope: SCOPE });
    expect(typeof apply.args.idempotency_key).toBe("string");
    expect(onClose).toHaveBeenCalled();
    expect(
      useUiStore.getState().notifications.some((entry) => entry.title === "3 approximate prices accepted"),
    ).toBe(true);
  });

  it("masks the tax changes when sensitive amounts are hidden", async () => {
    mount(vi.fn(), true);
    await screen.findByText("Accept 3 approximate prices?");
    expect(screen.getByRole("region", { name: "Tax change by year" }).closest(".sensitive")).toBeTruthy();
  });

  it("asks for a fresh preview when the book changed before the confirmation", async () => {
    daemon.staleApply = true;
    const onClose = mount();
    await screen.findByText("Accept 3 approximate prices?");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Accept 3 prices" }));
    });
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Check again" })).toBeTruthy();
  });
});
