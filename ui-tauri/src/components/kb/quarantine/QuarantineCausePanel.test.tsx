import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import i18n from "@/i18n";

const buttons = vi.hoisted(
  () => [] as Array<{ label: string; onClick?: () => void; disabled?: boolean }>,
);
const navigate = vi.hoisted(() => vi.fn());
const mutate = vi.hoisted(() => vi.fn());

vi.mock("@/components/ui/button", () => ({
  Button: (props: ComponentProps<"button">) => {
    buttons.push({
      label: renderToStaticMarkup(<>{props.children}</>).replace(/<[^>]+>/g, ""),
      onClick: props.onClick as (() => void) | undefined,
      disabled: props.disabled,
    });
    return <button disabled={props.disabled}>{props.children}</button>;
  },
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("@/daemon/client", () => ({
  useDaemonStreamMutation: () => ({ mutate, isPending: false }),
}));
vi.mock("@/components/kb/AddConnectionDialog", () => ({
  AddConnectionDialog: () => <div data-testid="add-connection" />,
}));

import { useUiStore } from "@/store/ui";

import { QuarantineCausePanel } from "./QuarantineCausePanel";
import type { QuarantineSnapshot } from "./types";

function snapshot(overrides: Partial<QuarantineSnapshot["summary"]> = {}): QuarantineSnapshot {
  return {
    summary: {
      workspace: "Books",
      profile: "Book",
      count: 3,
      by_reason: [],
      limit: 100,
      offset: 0,
      blocking_count: 1,
      reports_blocked: true,
      by_category: [
        { category: "missing_wallet_history", count: 1 },
        { category: "downstream", count: 2 },
      ],
      freshness: { needs_processing: false, last_processed_at: "2026-09-01T00:00:00Z", last_error: null },
      groups: [
        {
          key: "custody_quantity_unresolved:custody_gap_review_required:gap-1",
          category: "missing_wallet_history",
          reason: "custody_quantity_unresolved",
          root_transaction_id: "out",
          root_occurred_at: "2024-01-01T00:00:00Z",
          root_wallet: "Cold",
          root_external_id: "out",
          root_amount_msat: 100_000_000_000,
          root_direction: "outbound",
          root_asset: "BTC",
          count: 3,
          downstream_count: 2,
          blocks_reports: true,
          wallets: ["Cold", "Hot"],
          earliest_occurred_at: "2024-01-01T00:00:00Z",
          evidence: { blocker_code: "custody_gap_review_required", gap_id: "gap-1", wallet_label: "Cold" },
          actions: [{ kind: "connect_wallet" }, { kind: "review_custody_gap", gap_id: "gap-1" }],
        },
      ],
      group_count: 1,
      assumptions: {
        presumed_external_outbound: {
          count: 1,
          amount_msat: 50_000_000_000,
          items: [{ transaction_id: "pay", occurred_at: "2025-02-01T00:00:00Z", wallet: "Hot", amount_msat: 50_000_000_000, external_id: "" }],
        },
        unclassified_inbound: { count: 0, amount_msat: 0, items: [] },
      },
      ...overrides,
    },
    items: [],
  };
}

function render(data: QuarantineSnapshot, onOpenTransaction = vi.fn(), hideSensitive = false) {
  return renderToStaticMarkup(
    <QuarantineCausePanel
      snapshot={data}
      isProcessingJournals={false}
      onProcessJournals={() => {}}
      onOpenTransaction={onOpenTransaction}
      hideSensitive={hideSensitive}
    />,
  );
}

describe("quarantine cause panel", () => {
  beforeEach(() => {
    buttons.length = 0;
    navigate.mockReset();
    void i18n.changeLanguage("en");
    useUiStore.setState({ developerToolsEnabled: false });
  });

  it("explains the root cause, what to provide and what follows from it", () => {
    const html = render(snapshot());
    expect(html).toContain("Why is something in quarantine?");
    expect(html).toContain("1 blocks all reports");
    expect(html).toContain("2 clear with their cause");
    expect(html).toContain("An intermediate wallet is missing");
    expect(html).toContain("Coins left Cold and a similar amount came back later");
    expect(html).toContain("Blocks reports");
    expect(html).toContain("2 dependent transactions clear automatically once this is resolved.");
    expect(html).toContain("1 outflow booked as a disposal");
    expect(buttons.map((button) => button.label)).toContain("Connect wallet");
  });

  it("keeps the custody-gap editor behind developer tools", () => {
    render(snapshot());
    buttons.find((button) => button.label === "Review custody gap")?.onClick?.();
    expect(navigate).not.toHaveBeenCalled();

    useUiStore.setState({ developerToolsEnabled: true });
    buttons.length = 0;
    render(snapshot());
    buttons.find((button) => button.label === "Review custody gap")?.onClick?.();
    expect(navigate).toHaveBeenCalledWith({ to: "/swaps", search: { tab: "gaps", gap: "gap-1" } });
  });

  it("warns when the list is outdated or the last rebuild failed", () => {
    expect(
      render(
        snapshot({
          freshness: { needs_processing: true, last_processed_at: "2026-09-01T00:00:00Z", last_error: null },
        }),
      ),
    ).toContain("This list may be outdated");
    expect(
      render(
        snapshot({
          freshness: {
            needs_processing: false,
            last_processed_at: "2026-09-01T00:00:00Z",
            last_error: { code: "sync_conflicts_open", message: "Resolve sync conflicts.", at: "2026-09-02T00:00:00Z" },
          },
        }),
      ),
    ).toContain("Resolve sync conflicts. The list shows the state before that attempt.");
  });

  it("renders the same explanation in Austrian German", () => {
    void i18n.changeLanguage("de");
    const html = render(snapshot());
    expect(html).toContain("Warum ist etwas in Quarantäne?");
    expect(html).toContain("Eine Zwischen-Wallet fehlt");
    expect(html).toContain("Blockiert Berichte");
    void i18n.changeLanguage("en");
  });

  it("masks amounts when sensitive values are hidden", () => {
    const sensitive = (html: string) => (html.match(/class="[^"]*\bsensitive\b/g) ?? []).length;
    expect(sensitive(render(snapshot()))).toBe(0);
    // root amount, assumption total and assumption item
    expect(sensitive(render(snapshot(), vi.fn(), true))).toBeGreaterThanOrEqual(3);
  });

  it("hands the cause's reading to the sheet when its root is on another page", () => {
    const onOpenTransaction = vi.fn();
    render(snapshot(), onOpenTransaction);
    buttons.find((button) => button.label === "Open transaction")?.onClick?.();
    expect(onOpenTransaction).toHaveBeenCalledWith("out", "details", {
      reason: "custody_quantity_unresolved",
      category: "missing_wallet_history",
      evidence: { blocker_code: "custody_gap_review_required", gap_id: "gap-1", wallet_label: "Cold" },
      rootLabel: null,
    });
  });

  it("renders nothing for an empty, current quarantine", () => {
    expect(
      render(
        snapshot({
          count: 0,
          groups: [],
          blocking_count: 0,
          reports_blocked: false,
          assumptions: null,
        }),
      ),
    ).toBe("");
  });
});
