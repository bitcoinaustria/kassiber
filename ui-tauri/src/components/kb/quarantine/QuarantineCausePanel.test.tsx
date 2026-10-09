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
  DaemonRequestError: class extends Error {},
  useDaemonMutation: () => ({ mutateAsync: vi.fn() }),
  useDaemonStreamMutation: () => ({ mutate, isPending: false }),
}));

import { useUiStore } from "@/store/ui";

import { QuarantineCausePanel } from "./QuarantineCausePanel";
import type { QuarantineGroup, QuarantineItem, QuarantineSnapshot } from "./types";

const ROOT: QuarantineItem = {
  transaction_id: "out",
  external_id: "out",
  occurred_at: "2024-01-01T00:00:00Z",
  confirmed_at: null,
  wallet: "Cold",
  direction: "outbound",
  asset: "BTC",
  amount: 1,
  amount_msat: 100_000_000_000,
  fee: 0,
  fee_msat: 0,
  reason: "custody_quantity_unresolved",
  detail: {},
  created_at: "2026-09-30T19:38:23Z",
  category: "missing_wallet_history",
  blocks_reports: true,
  is_downstream: false,
  root: null,
  evidence: { blocker_code: "custody_gap_review_required", gap_id: "gap-1", wallet_label: "Cold" },
  actions: [{ kind: "connect_wallet" }, { kind: "review_custody_gap", gap_id: "gap-1" }],
  group_key: "custody_quantity_unresolved:custody_gap_review_required:gap-1",
};

const GAP_GROUP: QuarantineGroup = {
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
  root_transaction_ids: ["out"],
  root_count: 1,
};

const PAIR_EVIDENCE = {
  blocker_code: "reviewed_residual_suspense",
  pair_id: "pair-1",
  pair_counterpart_transaction_id: "in",
  pair_txids_differ: true,
  pair_receipt_before_spend: true,
};

const PAIR_GROUP: QuarantineGroup = {
  ...GAP_GROUP,
  key: "custody_quantity_unresolved:reviewed_residual_suspense:",
  category: "needs_decision",
  evidence: PAIR_EVIDENCE,
  actions: [{ kind: "review_pair", transaction_id: "out", pair_id: "pair-1" }],
  root_transaction_ids: ["out", "later-root"],
  root_count: 3,
};

function snapshot(
  overrides: Partial<QuarantineSnapshot["summary"]> = {},
  items: QuarantineItem[] = [ROOT],
): QuarantineSnapshot {
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
      groups: [GAP_GROUP],
      group_count: 1,
      attention_count: 1,
      waiting_count: 2,
      scope: "attention",
      scope_count: 1,
      ...overrides,
    },
    items,
  };
}

function render(
  data: QuarantineSnapshot,
  {
    onOpenTransaction = vi.fn(),
    onShowWaiting = vi.fn(),
    hideSensitive = false,
  }: {
    onOpenTransaction?: ReturnType<typeof vi.fn>;
    onShowWaiting?: ReturnType<typeof vi.fn>;
    hideSensitive?: boolean;
  } = {},
) {
  return renderToStaticMarkup(
    <QuarantineCausePanel
      snapshot={data}
      isProcessingJournals={false}
      onProcessJournals={() => {}}
      onOpenTransaction={onOpenTransaction}
      onConnectWallet={() => {}}
      onImportHistory={() => {}}
      onShowWaiting={onShowWaiting}
      onRefresh={async () => ({ items: data.items })}
      hideSensitive={hideSensitive}
    />,
  );
}

const sensitiveCount = (html: string) => (html.match(/class="[^"]*\bsensitive\b/g) ?? []).length;

describe("quarantine cause panel", () => {
  beforeEach(() => {
    buttons.length = 0;
    navigate.mockReset();
    void i18n.changeLanguage("en");
    useUiStore.setState({ developerToolsEnabled: false });
  });

  it("leads with what needs the user, then says what only waits", () => {
    const html = render(snapshot());
    expect(html).toContain("1 transaction needs you");
    expect(html).toContain("2 more only wait on these and clear by themselves.");
    expect(html).toContain("Tax and portfolio reports stay blocked until this is fixed.");
    expect(html).toContain("An intermediate wallet is missing");
    expect(html).toContain("Coins left Cold and a similar amount came back later");
    expect(html).toContain("Blocks reports");
    expect(html).toContain("2 later transactions wait on this and clear once it is fixed.");
    // The card says how many, and which.
    expect(html).toContain("1 transaction");
    expect(html).toContain("2024-01-01 · Cold · out");
    expect(buttons.map((button) => button.label)).toEqual(
      expect.arrayContaining(["Connect wallet", "Review custody gap", "Show them"]),
    );
    // Assumptions are not quarantine; they render apart from the causes.
    expect(html).not.toContain("outflow booked as a disposal");
  });

  it("lists every waiting transaction on request", () => {
    const onShowWaiting = vi.fn();
    render(snapshot(), { onShowWaiting });
    buttons.find((button) => button.label === "Show them")?.onClick?.();
    expect(onShowWaiting).toHaveBeenCalledOnce();
  });

  it("names what was seen in a pair that leaves a suspense, and opens the pair", () => {
    const onOpenTransaction = vi.fn();
    const html = render(
      snapshot({ groups: [PAIR_GROUP] }, [{ ...ROOT, evidence: PAIR_EVIDENCE, category: "needs_decision", actions: PAIR_GROUP.actions }]),
      { onOpenTransaction },
    );
    expect(html).toContain("A transfer pair doesn&#x27;t add up");
    expect(html).toContain("The two sides are different on-chain transactions");
    // A hint, not a verdict: a hop through an untracked wallet also has two txids.
    expect(html).toContain("this alone doesn&#x27;t make the pair wrong");
    expect(html).toContain("The receipt is dated before the payment it is paired with.");
    expect(html).toContain("3 transactions");
    buttons.find((button) => button.label === "Review the pair")?.onClick?.();
    expect(onOpenTransaction).toHaveBeenCalledWith("out", "linked", {
      reason: "custody_quantity_unresolved",
      category: "needs_decision",
      evidence: PAIR_EVIDENCE,
      rootLabel: null,
    }, PAIR_GROUP.key);
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

  it("says the list changed since its calculation, not that journals never ran", () => {
    // Every invalidation clears the timestamp; the rows keep theirs.
    const stale = render(
      snapshot({ freshness: { needs_processing: true, last_processed_at: null, last_error: null } }),
    );
    expect(stale).toContain("This list may be outdated");
    expect(stale).toContain("changed after the journals were last calculated (2026-09-30)");
    expect(stale).not.toContain("have not been processed yet");
    expect(buttons.map((button) => button.label)).toContain("Recalculate journals");
    expect(
      render(
        snapshot(
          { count: 0, groups: [], freshness: { needs_processing: true, last_processed_at: null, last_error: null } },
          [],
        ),
      ),
    ).toContain("Journals have not been processed yet");
    expect(
      render(
        snapshot({
          freshness: {
            needs_processing: false,
            last_processed_at: "2026-09-01T00:00:00Z",
            last_error: { code: "tax_failed", message: "Tax processing failed.", at: "2026-09-02T00:00:00Z" },
          },
        }),
      ),
    ).toContain("Tax processing failed. The list shows the state before that attempt.");
  });

  it("renders the same explanation in Austrian German", () => {
    void i18n.changeLanguage("de");
    const html = render(snapshot());
    expect(html).toContain("1 Transaktion braucht dich");
    expect(html).toContain("Eine Zwischen-Wallet fehlt");
    expect(html).toContain("Blockiert Berichte");
    void i18n.changeLanguage("en");
  });

  it("masks an explanation that quotes amounts when sensitive values are hidden", () => {
    const oversell = snapshot({
      groups: [
        {
          ...GAP_GROUP,
          key: "insufficient_lots:BTC:Cold",
          category: "missing_acquisition_history",
          reason: "insufficient_lots",
          evidence: { required_msat: 12 * 100_000_000_000, available_msat: 10 * 100_000_000_000, wallet_label: "Cold" },
          actions: [{ kind: "import_history" }],
        },
      ],
    });
    expect(sensitiveCount(render(oversell))).toBe(0);
    // Only the explanation: none of that cause's rows are loaded.
    expect(sensitiveCount(render(oversell, { hideSensitive: true }))).toBe(1);
    // The gap's explanation quotes nothing; its listed row's line and amount do.
    expect(sensitiveCount(render(snapshot(), { hideSensitive: true }))).toBe(2);
  });

  it("opens a cause's first transaction when it offers no action and lists none", () => {
    const onOpenTransaction = vi.fn();
    render(snapshot({ groups: [{ ...GAP_GROUP, actions: [] }] }, []), { onOpenTransaction });
    buttons.find((button) => button.label === "Open transaction")?.onClick?.();
    expect(onOpenTransaction).toHaveBeenCalledWith("out", "details", expect.objectContaining({ reason: "custody_quantity_unresolved" }), GAP_GROUP.key);
  });

  it("renders nothing for an empty, current quarantine", () => {
    expect(
      render(
        snapshot(
          { count: 0, groups: [], blocking_count: 0, reports_blocked: false },
          [],
        ),
      ),
    ).toBe("");
  });
});
