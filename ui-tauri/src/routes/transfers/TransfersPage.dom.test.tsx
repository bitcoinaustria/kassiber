// @vitest-environment happy-dom
//
// The Transfers overview: the verdict first, automatic links listed for
// everyone (developer tools off) with an unlink dialog that shows both legs,
// the fee, how the move was linked and the core's dry run before it applies
// it; a pair the owner unlinked listed under "Recently unlinked" with "Link
// again" whether or not the matcher suggests it; "Link manually" from the
// send to the receipt to the check, and its note with Undo; and an unlinked
// move the journals hold asked about once, as a question, after a reload.
// The dialog's own edge cases are in LinkManuallyDialog.dom.test.tsx.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui";

import { AUTOMATIC_UNDO_REASON, type CustodyLineageItem } from "../custodyGapsModel";
import type { TransferDismissal } from "./linkedMovesModel";
import { forgetFinishedPreviews } from "./reviewPreview";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const daemon = vi.hoisted(() => ({
  items: [] as unknown[],
  dismissals: [] as unknown[],
  held: [] as string[],
  heldItems: [] as unknown[],
  pairs: [] as unknown[],
  unlinked: [] as unknown[],
  txs: {} as Record<string, unknown>,
  failDismiss: false,
  failApply: false,
  /** Reads that answer with an error. */
  failing: new Set<string>(),
  heldAfter: [] as string[],
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  const ok = (kind: string, data: unknown) => ({ kind, schema_version: 1, data });
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (daemon.failing.has(kind)) {
          return { kind: "error", schema_version: 1, error: { code: "internal", message: "read failed", retryable: false } };
        }
        switch (kind) {
          case "ui.custody.lineage.snapshot":
            return ok(kind, { summary: { total_count: daemon.items.length, truncated: false }, items: daemon.items });
          case "ui.transfers.list":
            return ok(kind, { pairs: daemon.pairs, ...(args.include_unlinked ? { unlinked: daemon.unlinked } : {}) });
          case "ui.profiles.snapshot":
            return ok(kind, { workspaces: [], activeWorkspaceId: "w1", activeProfileId: "p1" });
          case "ui.review.cases":
            return ok(kind, { input_version: 7, cases: [] });
          case "ui.review.plan":
            return ok(kind, artifact(args.operations as Array<Record<string, unknown>>));
          case "ui.review.apply": {
            if (daemon.failApply) {
              return { kind: "error", schema_version: 1, error: { code: "not_found", message: "Transaction not found", retryable: false } };
            }
            const operations = ((args.artifact as { operations: Array<Record<string, unknown>> }).operations ?? []).map((operation) => ({
              type: operation.type,
              result: operation.type === "pair" ? { pair_id: "pair-new" } : { dismissal_id: "dismissal-7" },
            }));
            return ok(kind, { operations });
          }
          case "ui.transactions.resolve":
            return ok(kind, { transaction: daemon.txs[String(args.query)] ?? null });
          case "ui.transactions.list": {
            const direction = args.direction === "outbound" ? -1 : 1;
            return ok(kind, {
              txs: Object.values(daemon.txs).filter((tx) => Math.sign((tx as { amountSat: number }).amountSat) === direction),
              year: 2024,
            });
          }
          case "ui.transfers.unpair":
            return ok(kind, { deleted: args.pair_id, pair: daemon.pairs[0] ?? null });
          case "ui.transfers.dismissals.list":
            return ok(kind, { dismissals: daemon.dismissals, has_more: false });
          case "ui.transfers.suggest":
            return ok(kind, { candidates: [] });
          case "ui.review.badges":
            return ok(kind, { quarantine: daemon.held.length, journals_needs_processing: false, swaps: 0 });
          case "ui.journals.quarantine":
            return ok(kind, {
              summary: { count: daemon.held.length, by_reason: [], limit: 100 },
              items: [...daemon.held.map((id) => ({ transaction_id: id })), ...daemon.heldItems],
            });
          case "ui.transfers.dismiss":
            if (daemon.failDismiss) {
              return { kind: "error", schema_version: 1, error: { code: "not_found", message: "Transaction not found", retryable: false } };
            }
            return ok(kind, { id: "dismissal-7" });
          case "ui.transfers.dismissals.delete":
            return ok(kind, { deleted: true });
          case "ui.saved_views.list":
            return ok(kind, {
              views:
                args.surface === "swap_candidates"
                  ? [{ id: "view-swaps", surface: "swap_candidates", name: "Liquid swaps", filter: { asset_pair: "LBTC/USDT" } }]
                  : [],
            });
          default:
            return { kind: "error", schema_version: 1, error: { code: "not_found", message: "unavailable", retryable: false } };
        }
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  // Props a menu hands its item (the role, the focus handling) pass through, as on the real link.
  Link: ({ children, to, search, ...rest }: { children: React.ReactNode; to: string; search?: Record<string, string> }) => (
    <a {...rest} href={`${to}?${new URLSearchParams(search ?? {}).toString()}`}>
      {children}
    </a>
  ),
  useNavigate: () => () => undefined,
  useSearch: () => ({}),
}));

import { TransfersOverview } from "./TransfersPage";

function effects(held: string[]) {
  return {
    entries_count: 10,
    quarantine_count: held.length,
    report_ready: held.length === 0,
    quarantines: held.map((id) => ({ transaction_id: id, reason: "recorded_transfer_authority_conflict" })),
  };
}

/** A review artifact as the core plans it; holds `heldAfter` after the change. */
function artifact(operations: Array<Record<string, unknown>>) {
  return {
    schema_version: 1,
    workspace_id: "w1",
    profile_id: "p1",
    base_input_version: 7,
    digest: "a".repeat(64),
    operations,
    before: effects(daemon.held),
    after: effects(daemon.heldAfter),
  };
}

function tx(id: string, overrides: Record<string, unknown> = {}) {
  return {
    id,
    externalId: `${id}-txid-0123456789abcdef0123456789abcdef`,
    date: "2024-09-17 02:14",
    occurredAt: "2024-09-17T02:14:00Z",
    type: "Transfer",
    account: "Wallet",
    counter: "",
    amountSat: -4200000,
    eur: null,
    rate: null,
    tag: "",
    conf: 1,
    ...overrides,
  };
}

/** The peg-out the owner unlinked: Liquid out, on-chain in, a fee the matcher never suggests. */
const PEG_OUT_PAIR = {
  id: "pair-peg",
  out_transaction_id: "peg-out",
  in_transaction_id: "peg-in",
  kind: "peg-out",
  policy: "taxable",
  swap_fee_msat: 144910000,
  pair_source: "manual",
  out_amount: 4200000000,
  created_at: "2026-11-13T12:35:09Z",
  deleted_at: "2026-11-13T14:02:00Z",
  out: { transaction_id: "peg-out", wallet: "Liquid Treasury", wallet_kind: "descriptor", asset: "LBTC", occurred_at: "2024-09-17T02:14:00Z", amount_msat: 4200000000 },
  in: { transaction_id: "peg-in", wallet: "Treasury 2020", wallet_kind: "address", asset: "BTC", occurred_at: "2024-09-17T03:22:00Z", amount_msat: 4080000000 },
};

const PEG_TXS = {
  "peg-out": tx("peg-out", { account: "Liquid Treasury", asset: "LBTC", chain: "liquid", paymentMethod: "Liquid", amountSat: -4200000, feeSat: 24910 }),
  "peg-in": tx("peg-in", { account: "Treasury 2020", asset: "BTC", chain: "bitcoin", amountSat: 4080000, occurredAt: "2024-09-17T03:22:00Z" }),
  "other-in": tx("other-in", { account: "Spending", asset: "BTC", amountSat: 4100000, occurredAt: "2024-09-25T09:00:00Z" }),
};

function lineageItem(overrides: Partial<CustodyLineageItem> = {}): CustodyLineageItem {
  return {
    out_transaction_id: "private-withdrawal-id",
    in_transaction_id: "private-receipt-id",
    occurred_at: "2024-02-01T10:00:00Z",
    asset: "BTC",
    amount_msat: "600000000",
    from_wallet_id: "private-wallet-strike",
    from_wallet_label: "Strike",
    to_wallet_id: "private-wallet-cold",
    to_wallet_label: "Cold storage",
    custody_state: "internal_verified",
    basis_state: "eligible",
    basis_barrier_at: null,
    evidence_reason: "exchange_txid_exact",
    network: "main",
    rail: "bitcoin",
    component_id: "private-component-id",
    ...overrides,
  };
}

function undone(overrides: Partial<TransferDismissal> = {}): TransferDismissal {
  return {
    id: "dismissal-9",
    out_transaction_id: "out-bark",
    in_transaction_id: "in-store",
    reason: AUTOMATIC_UNDO_REASON,
    created_at: "2026-11-13T12:15:31Z",
    expires_at: "2036-11-10T12:15:31Z",
    active: true,
    permanent: false,
    out: { transaction_id: "out-bark", wallet: "BTCPay Bark", asset: "BTC", amount_msat: "600000000", occurred_at: "2026-11-13T11:51:50Z" },
    in: { transaction_id: "in-store", wallet: "Store on-chain", asset: "BTC", amount_msat: "600000000", occurred_at: "2026-11-13T11:52:02Z" },
    ...overrides,
  };
}

const onOpen = vi.fn();

function mount(props: { savedViewId?: string | null } = {}) {
  // The daemon hook retries a failed read on its own; without a delay, so a failure shows at once.
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, retryDelay: 0 }, mutations: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <TransfersOverview onOpen={onOpen} savedViewId={props.savedViewId ?? null} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return {
    ...view,
    /** Opens the page on a saved view, as following its link does. */
    openView: (savedViewId: string | null) =>
      view.rerender(
        <QueryClientProvider client={client}>
          <TooltipProvider>
            <TransfersOverview onOpen={onOpen} savedViewId={savedViewId} />
          </TooltipProvider>
        </QueryClientProvider>,
      ),
  };
}

const calls = (kind: string) => daemon.calls.filter((call) => call.kind === kind);

/** Opens one of the status tabs (Needs you, Linked, Unlinked by you, Set aside). */
async function openTab(name: RegExp) {
  const trigger = await within(await screen.findByTestId("transfers-tabs")).findByRole("button", { name });
  fireEvent.click(trigger);
  await waitFor(() => expect(trigger.getAttribute("aria-pressed")).toBe("true"));
}

async function openRowMenu(name: RegExp) {
  const trigger = await screen.findByRole("button", { name });
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: "mouse" });
  return screen.findByRole("menu");
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  useUiStore.setState({ developerToolsEnabled: false });
  daemon.items = [lineageItem()];
  daemon.dismissals = [];
  daemon.held = [];
  daemon.heldItems = [];
  daemon.pairs = [];
  daemon.unlinked = [];
  daemon.txs = {};
  daemon.failDismiss = false;
  daemon.failApply = false;
  daemon.failing = new Set();
  daemon.heldAfter = [];
  daemon.calls = [];
  onOpen.mockClear();
  forgetFinishedPreviews();
});

afterEach(() => cleanup());

describe("Transfers overview", () => {
  it("leads with a settled verdict and lists automatic links with developer tools off", async () => {
    mount();
    const verdict = await screen.findByTestId("transfers-verdict");
    await waitFor(() => expect(verdict.getAttribute("data-state")).toBe("settled"));
    expect(within(verdict).getByText("Nothing waits on you")).toBeTruthy();
    const list = await screen.findByTestId("transfers-linked-list");
    expect(within(list).getByText("Strike")).toBeTruthy();
    expect(within(list).getByText("Exchange txid")).toBeTruthy();
    // Why it was linked sits behind the row's info icon.
    expect(within(list).getByRole("button", { name: "Linked automatically. Exchange withdrawal names this transaction" })).toBeTruthy();
    // The amount is written the way the Transactions table writes it, its leading zeros dimmed.
    expect(within(list).getAllByRole("row")[1].textContent).toContain("₿0.00600000");
    // The column headers of a table, as on Transactions.
    for (const header of ["Date", "From → To", "Amount", "Fee", "Type", "Linked"]) {
      expect(within(list).getByRole("columnheader", { name: header })).toBeTruthy();
    }
    expect(screen.getByRole("region", { name: "Transfers at a glance" }).textContent).toContain("1 automatic · 0 by you");
    // Ids stay out of the page.
    expect(document.body.innerHTML).not.toContain("private-");
  });

  it("opens a move's details from its row, without a dry run until asked to unlink", async () => {
    daemon.txs = {
      "private-withdrawal-id": tx("private-withdrawal-id", { account: "Strike", amountSat: -600000, feeSat: 0 }),
      "private-receipt-id": tx("private-receipt-id", { account: "Cold storage", amountSat: 600000, occurredAt: "2024-02-01T10:20:00Z" }),
    };
    mount();
    fireEvent.click(await screen.findByRole("row", { name: /Show the transfer Strike to Cold storage/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    expect(dialog.getAttribute("data-intent")).toBe("details");
    expect(within(dialog).getByText("Linked transfer")).toBeTruthy();
    expect(within(dialog).getByText("Automatically: Exchange withdrawal names this transaction.")).toBeTruthy();
    expect(calls("ui.review.plan")).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: /Unlink/ }));
    await waitFor(() => expect(screen.getByTestId("unlink-dialog").getAttribute("data-intent")).toBe("unlink"));
    expect(within(screen.getByTestId("unlink-dialog")).getByText("Unlink this transfer?")).toBeTruthy();
  });

  it("opens a transfer in a panel beside the table when there is room, and marks its row", async () => {
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 1300, height: 800, top: 0, left: 0, right: 1300, bottom: 800, x: 0, y: 0, toJSON: () => ({}),
    } as DOMRect);
    try {
      mount();
      const row = await screen.findByRole("row", { name: /Show the transfer Strike to Cold storage/ });
      fireEvent.click(row);
      const panel = await screen.findByTestId("unlink-dialog");
      expect(panel.getAttribute("data-variant")).toBe("panel");
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(row.getAttribute("aria-current")).toBe("true");
      fireEvent.click(within(panel).getByRole("button", { name: "Close" }));
      await waitFor(() => expect(screen.queryByTestId("unlink-dialog")).toBeNull());
    } finally {
      rect.mockRestore();
    }
  });

  it("shows both legs, the fee, how it was linked and the core's preview before unlinking; cancelling sends nothing", async () => {
    daemon.txs = {
      "private-withdrawal-id": tx("private-withdrawal-id", { account: "Strike", amountSat: -600000, feeSat: 0 }),
      "private-receipt-id": tx("private-receipt-id", { account: "Cold storage", amountSat: 600000, occurredAt: "2024-02-01T10:20:00Z" }),
    };
    daemon.heldAfter = ["private-withdrawal-id"];
    mount();
    const menu = await openRowMenu(/Actions for Strike to Cold storage/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Unlink/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    expect(within(dialog).getByText("Unlink this transfer?")).toBeTruthy();
    const legs = await within(dialog).findAllByTestId("move-leg");
    expect(legs).toHaveLength(2);
    expect(await within(legs[0]).findByRole("button", { name: "Copy the transaction id" })).toBeTruthy();
    expect(within(legs[1]).getByRole("link", { name: "Show in Transactions" }).getAttribute("href")).toBe("/transactions?tx=private-receipt-id");
    expect(within(dialog).getByText("Nothing was lost on the way.")).toBeTruthy();
    expect(within(dialog).getByText("Automatically: Exchange withdrawal names this transaction.")).toBeTruthy();
    // The core's dry run of the undo says the send is held.
    expect(await within(dialog).findByText(/The send from Strike is held until you decide what it was/)).toBeTruthy();
    expect(within(dialog).getByText(/Tax reports still wait for 1 held transaction/)).toBeTruthy();
    expect(calls("ui.review.plan")[0].args.operations).toEqual([
      expect.objectContaining({ type: "dismiss_pair", reason: AUTOMATIC_UNDO_REASON, expires_in_days: 3650 }),
    ]);
    expect(within(dialog).getByText(/under Unlinked by you on this page, or under To sort out/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep the link" }));
    await waitFor(() => expect(screen.queryByTestId("unlink-dialog")).toBeNull());
    expect(calls("ui.review.apply")).toHaveLength(0);
    expect(calls("ui.transfers.dismiss")).toHaveLength(0);
  });

  it("applies the previewed undo and says so in a strip at the row, whose Undo links back by the dismissal it returned", async () => {
    mount();
    const menu = await openRowMenu(/Actions for Strike to Cold storage/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Unlink/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    await within(dialog).findByTestId("preview-outcome");
    fireEvent.click(within(dialog).getByRole("button", { name: "Unlink" }));

    // On the list the transfer was in, the note sits at its row, not in a toast as well.
    const strip = await screen.findByTestId("transfers-strip");
    expect(strip.textContent).toContain("Unlinked.");
    expect(strip.previousElementSibling?.getAttribute("aria-label")).toBe("Show the transfer Strike to Cold storage");
    expect(screen.queryByTestId("transfers-toast")).toBeNull();
    expect(calls("ui.review.apply")).toHaveLength(1);
    fireEvent.click(within(strip).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(screen.getByTestId("transfers-strip").textContent).toContain("Linked again."));
    expect(calls("ui.transfers.dismissals.delete").map((call) => call.args)).toEqual([
      expect.objectContaining({ dismissal_id: "dismissal-7" }),
    ]);
    // It stays until it is closed.
    fireEvent.click(within(screen.getByTestId("transfers-strip")).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByTestId("transfers-strip")).toBeNull());
  });

  it("says it in a toast instead when the transfer's list is not the one in view", async () => {
    daemon.dismissals = [undone()];
    mount();
    const menu = await openRowMenu(/Actions for Strike to Cold storage/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Unlink/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    await within(dialog).findByTestId("preview-outcome");
    fireEvent.click(within(dialog).getByRole("button", { name: "Unlink" }));
    await screen.findByTestId("transfers-strip");
    await openTab(/Unlinked by you/);
    expect(screen.queryByTestId("transfers-strip")).toBeNull();
    expect((await screen.findByTestId("transfers-toast")).textContent).toContain("Unlinked. Kassiber now asks about the send");
  });

  it("keeps the dialog open with the daemon's error when the undo fails", async () => {
    daemon.failApply = true;
    mount();
    const menu = await openRowMenu(/Actions for Strike to Cold storage/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Unlink/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    await within(dialog).findByTestId("preview-outcome");
    fireEvent.click(within(dialog).getByRole("button", { name: "Unlink" }));
    expect(await within(dialog).findByText("Transaction not found")).toBeTruthy();
  });

  it("explains a fan-out move instead of offering to unlink it", async () => {
    daemon.items = [lineageItem({ out_transaction_id: "o2", in_transaction_id: "i2", from_wallet_label: "Vault", evidence_reason: "recorded_fanout" })];
    mount();
    // Its menu has no Unlink: the Linked column names the kind, its hint says why, and the row opens the explanation.
    const row = await screen.findByRole("row", { name: /Show the transfer Vault/ });
    expect(within(row).getByText("Paid to several")).toBeTruthy();
    const menu = await openRowMenu(/Actions for Vault/);
    expect(within(menu).queryByRole("menuitem", { name: /Unlink/ })).toBeNull();
    // Both sides open on the Transactions page from any row.
    expect(within(menu).getByRole("menuitem", { name: "Show the send in Transactions" }).getAttribute("href")).toBe("/transactions?tx=o2");
    expect(within(menu).getByRole("menuitem", { name: "Show the receipt in Transactions" }).getAttribute("href")).toBe("/transactions?tx=i2");
    fireEvent.keyDown(menu, { key: "Escape" });
    expect(within(row).getByRole("button", { name: /One transaction paid several of your wallets/ })).toBeTruthy();
    fireEvent.click(screen.getByRole("row", { name: /Show the transfer Vault/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    expect(dialog.getAttribute("data-mode")).toBe("fixed");
    expect(within(dialog).getByText("Why it can't be unlinked")).toBeTruthy();
    expect(within(dialog).getByText(/One transaction paid several of your wallets, so there is nothing to unlink/)).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Unlink" })).toBeNull();
  });

  it("says plainly what unlinking a pair you made does, since the core can't preview it", async () => {
    daemon.items = [];
    daemon.pairs = [{ ...PEG_OUT_PAIR, deleted_at: null }];
    daemon.txs = PEG_TXS;
    mount();
    const menu = await openRowMenu(/Actions for Liquid Treasury to Treasury 2020/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Unlink/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    expect(within(dialog).getByText("By you on 2026-11-13 12:35.")).toBeTruthy();
    expect(within(dialog).getByText("Recorded as: Peg-out from Liquid · Counts as a sale and a purchase (taxable)")).toBeTruthy();
    // Recorded as taxable, so unlinking it moves no tax figure.
    expect(within(dialog).getByTestId("unlink-plain-effects").textContent).toContain(
      "Tax figures stay the same: this link already counted as a sale and a purchase",
    );
    // The fee the pair records, flagged as worth a second look.
    expect(within(dialog).getByText(/144,910 sats/)).toBeTruthy();
    expect(within(dialog).getByTestId("fee-flag")).toBeTruthy();
    expect(within(dialog).getByText(/Unlinked by you on this page with Link again/)).toBeTruthy();
    expect(calls("ui.review.plan")).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Unlink" }));
    await waitFor(() => expect(calls("ui.transfers.unpair").map((call) => call.args)).toEqual([{ pair_id: "pair-peg" }]));
    expect((await screen.findByTestId("transfers-strip")).textContent).toContain("Unlinked.");
  });

  it("warns that unlinking a tax-neutral pair you made books a sale and a purchase", async () => {
    daemon.items = [];
    daemon.pairs = [{ ...PEG_OUT_PAIR, policy: "carrying-value", deleted_at: null }];
    daemon.txs = PEG_TXS;
    mount();
    const menu = await openRowMenu(/Actions for Liquid Treasury to Treasury 2020/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Unlink/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    expect(within(dialog).getByTestId("unlink-plain-effects").textContent).toContain(
      "The send from Liquid Treasury will count as a sale and the receipt in Treasury 2020 as a purchase",
    );
  });

  it("asks about a held unlinked move once, as a question with real answers, after a reload", async () => {
    daemon.items = [];
    daemon.dismissals = [undone()];
    daemon.held = ["out-bark"];
    daemon.txs = { "out-bark": tx("out-bark", { account: "BTCPay Bark", amountSat: -600000 }) };
    mount();
    const verdict = await screen.findByTestId("transfers-verdict");
    await waitFor(() => expect(verdict.getAttribute("data-state")).toBe("sortOut"));
    expect(within(verdict).getByText("1 transfer to sort out")).toBeTruthy();
    const held = await screen.findByTestId("transfers-held-move");
    expect(within(held).getByText("You unlinked this ₿0.00600000 send from BTCPay Bark.")).toBeTruthy();
    expect(within(held).getByText(/arrived in Store on-chain, which you own, so Kassiber can't book it as a payment/)).toBeTruthy();
    expect(within(held).getByText("What was it?")).toBeTruthy();
    // Asked once: not a banner, and not again as a row in the list.
    expect(screen.getAllByText(/You unlinked this/)).toHaveLength(1);
    expect(screen.queryByTestId("transfers-toast")?.textContent ?? "").toBe("");
    expect(screen.getByText("Tax reports wait until each one is answered.")).toBeTruthy();
    // What needs the owner opens first; the linked list is one tab over.
    await openTab(/^Linked/);
    const list = await screen.findByTestId("transfers-linked-list");
    expect(within(list).queryByText("BTCPay Bark")).toBeNull();
    await openTab(/^Needs you/);
    const asked = await screen.findByTestId("transfers-held-move");
    // The advanced answer stays with developer tools.
    expect(within(asked).queryByRole("button", { name: /Describe what happened/ })).toBeNull();

    fireEvent.click(within(asked).getByRole("button", { name: /It arrived somewhere else/ }));
    // The send is known, so the dialog opens on the list of receipts for it.
    const link = await screen.findByTestId("link-manually");
    expect((await within(link).findByTestId("link-pick")).getAttribute("data-side")).toBe("receipt");
    fireEvent.click(within(link).getByRole("button", { name: "Cancel" }));
    fireEvent.click(within(asked).getByRole("button", { name: "Yes, my own transfer to Store on-chain" }));
    await waitFor(() =>
      expect(calls("ui.transfers.dismissals.delete").map((call) => call.args)).toEqual([
        expect.objectContaining({ dismissal_id: "dismissal-9" }),
      ]),
    );
  });

  it("offers the custody-component answer only with developer tools on", async () => {
    useUiStore.setState({ developerToolsEnabled: true });
    daemon.items = [];
    daemon.dismissals = [undone()];
    daemon.held = ["out-bark"];
    mount();
    const held = await screen.findByTestId("transfers-held-move");
    fireEvent.click(within(held).getByRole("button", { name: /Describe what happened/ }));
    expect(onOpen).toHaveBeenCalledWith({ section: "advanced", tab: "components" });
  });

  it("lists an undone automatic link that no longer holds anything under Recently unlinked, with Link back", async () => {
    daemon.items = [];
    daemon.dismissals = [undone()];
    mount();
    expect(within(await screen.findByTestId("transfers-linked-list")).queryByText("BTCPay Bark")).toBeNull();
    await openTab(/^Unlinked by you/);
    const card = await screen.findByTestId("transfers-recently-unlinked");
    expect(screen.queryByTestId("transfers-held-move")).toBeNull();
    expect(within(card).getByText("BTCPay Bark")).toBeTruthy();
    expect(within(card).getByText("was linked automatically")).toBeTruthy();
    fireEvent.click(within(card).getByRole("button", { name: /Link BTCPay Bark to Store on-chain again/ }));
    await waitFor(() =>
      expect(calls("ui.transfers.dismissals.delete").map((call) => call.args)).toEqual([
        expect.objectContaining({ dismissal_id: "dismissal-9" }),
      ]),
    );
  });

  it("links an unlinked peg-out again with its old terms, though the matcher never suggests it", async () => {
    daemon.items = [];
    daemon.unlinked = [PEG_OUT_PAIR];
    daemon.txs = PEG_TXS;
    mount();
    await openTab(/^Unlinked by you/);
    const card = await screen.findByTestId("transfers-recently-unlinked");
    expect(within(card).getByText("Unlinked 2026-11-13")).toBeTruthy();
    expect(within(card).getByText("was Peg-out from Liquid, taxable")).toBeTruthy();
    fireEvent.click(within(card).getByRole("button", { name: /Link Liquid Treasury to Treasury 2020 again/ }));
    const dialog = await screen.findByTestId("link-manually");
    // Both sides are known: it opens on the check, with the answer the pair had.
    expect(dialog.getAttribute("data-state")).toBe("check");
    expect(within(dialog).getByRole("heading", { name: "Link again" })).toBeTruthy();
    await within(dialog).findByTestId("preview-outcome");
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([
      {
        type: "pair",
        out_transaction_id: "peg-out",
        in_transaction_id: "peg-in",
        pair_kind: "peg-out",
        policy: "taxable",
        reason: "Linked again after unlinking",
      },
    ]);
    const sale = within(dialog).getByRole("radio", { name: /Yes, a sale and a purchase/ }) as HTMLInputElement;
    expect(sale.checked).toBe(true);
    expect(within(within(dialog).getByTestId("link-question")).getByText("As before")).toBeTruthy();
    // 3.45% between Liquid and the chain is what a peg-out costs: named, not warned about.
    expect(within(dialog).getByTestId("link-difference").textContent).toBe("144,910 sats less arrived than was sent (3.45% of the send).");
    expect(within(dialog).queryByTestId("link-warnings")).toBeNull();
    // The whole send was swapped, so no portion is named and the network fee stays in the fee.
    expect(within(dialog).queryByText(/of the send was swapped/)).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "Link again" }));
    await waitFor(() => expect(screen.queryByTestId("link-manually")).toBeNull());
    expect(calls("ui.review.apply")).toHaveLength(1);
    expect((await screen.findByTestId("transfers-toast")).textContent).toContain("Linked.");
  });

  it("links manually: picks the send, ranks the receipts, asks what it was and checks before linking", async () => {
    daemon.items = [];
    daemon.txs = PEG_TXS;
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Link manually…" }));
    const dialog = await screen.findByTestId("link-manually");
    expect(dialog.getAttribute("data-state")).toBe("pick");
    expect((await within(dialog).findByTestId("link-pick")).getAttribute("data-side")).toBe("send");
    const sends = await within(dialog).findAllByTestId("link-candidate");
    expect(sends).toHaveLength(1);
    fireEvent.click(sends[0]);
    await waitFor(() => expect(within(dialog).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt"));
    await waitFor(() => expect(within(dialog).getAllByTestId("link-candidate")[0].textContent).toContain("Treasury 2020"));
    const receipts = within(dialog).getAllByTestId("link-candidate");
    // The receipt 68 minutes later comes before the one eight days later.
    expect(receipts[0].textContent).toContain("Treasury 2020");
    expect(receipts[0].textContent).toContain("68 min later");
    expect(receipts[0].textContent).toContain("144,910 sats less");
    expect(receipts[1].textContent).toContain("Spending");
    expect(calls("ui.transactions.list").at(-1)?.args).toEqual(
      expect.objectContaining({ direction: "inbound", since: "2024-09-17T01:14:00Z", until: "2024-09-20T02:14:00Z" }),
    );
    fireEvent.click(receipts[0]);
    await waitFor(() => expect(dialog.getAttribute("data-state")).toBe("check"));
    // Liquid to the chain can count either way, and nothing suggests one: the owner is asked, and nothing runs before the answer.
    expect(await within(dialog).findByTestId("link-unanswered")).toBeTruthy();
    expect((within(dialog).getByRole("button", { name: "Link these two" }) as HTMLButtonElement).disabled).toBe(true);
    expect(calls("ui.review.plan")).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("radio", { name: /No, it's still my bitcoin/ }));
    await within(dialog).findByTestId("preview-outcome");
    // Recorded as the peg-out the two sides show it to be.
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([
      {
        type: "pair",
        out_transaction_id: "peg-out",
        in_transaction_id: "peg-in",
        pair_kind: "peg-out",
        policy: "carrying-value",
        reason: "Linked by hand on Transfers",
      },
    ]);
    fireEvent.click(within(dialog).getByRole("button", { name: "Link these two" }));
    // The dialog closes; on the Linked list the note is a strip with the way back.
    await waitFor(() => expect(screen.queryByTestId("link-manually")).toBeNull());
    const strip = await screen.findByTestId("transfers-strip");
    expect(strip.textContent).toContain("Linked.");
    expect(screen.queryByTestId("transfers-toast")).toBeNull();
    fireEvent.click(within(strip).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(calls("ui.transfers.unpair").map((call) => call.args)).toEqual([{ pair_id: "pair-new" }]));
    await waitFor(() => expect(screen.getByTestId("transfers-strip").textContent).toContain("The link was removed."));
  });

  it("warns, and asks to link anyway, when the link itself puts transactions on hold", async () => {
    daemon.items = [];
    daemon.txs = PEG_TXS;
    daemon.heldAfter = ["peg-out", "peg-in"];
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Link manually…" }));
    const dialog = await screen.findByTestId("link-manually");
    fireEvent.click((await within(dialog).findAllByTestId("link-candidate"))[0]);
    await waitFor(() => expect(within(dialog).getAllByTestId("link-candidate")[0].textContent).toContain("Treasury 2020"));
    fireEvent.click(within(dialog).getAllByTestId("link-candidate")[0]);
    fireEvent.click(await within(dialog).findByRole("radio", { name: /Yes, a sale and a purchase/ }));
    await within(dialog).findByTestId("preview-outcome");
    expect(within(dialog).getByText("Linking these puts 2 transactions on hold. Tax reports wait until you resolve them.")).toBeTruthy();
    // Said once: the line about reports still waiting would repeat it.
    expect(within(dialog).queryByText(/Tax reports still wait/)).toBeNull();
    expect(within(dialog).queryByText(/still held for another reason/)).toBeNull();
    expect(within(dialog).getByRole("button", { name: "Link anyway" })).toBeTruthy();
  });

  it("lists suggestions set aside on their own tab and restores one without linking it", async () => {
    daemon.dismissals = [
      undone({ id: "dismissal-aside", reason: "user dismissed from review queue", out_transaction_id: "o9", in_transaction_id: "i9" }),
    ];
    mount();
    await openTab(/^Set aside/);
    const card = await screen.findByTestId("transfers-set-aside");
    expect(within(card).getByTestId("transfers-set-aside-row")).toBeTruthy();
    fireEvent.click(within(card).getByRole("button", { name: /Restore the suggestion/ }));
    await waitFor(() =>
      expect(calls("ui.transfers.dismissals.delete").map((call) => call.args)).toEqual([{ dismissal_id: "dismissal-aside" }]),
    );
    expect(calls("ui.transfers.pair")).toHaveLength(0);
  });

  it.each(["ui.journals.quarantine", "ui.review.badges", "ui.transfers.dismissals.list"])(
    "says it could not check everything, not that nothing waits, when %s fails",
    async (kind) => {
      daemon.failing = new Set([kind]);
      mount();
      const verdict = await screen.findByTestId("transfers-verdict");
      await waitFor(() => expect(verdict.getAttribute("data-state")).toBe("unavailable"));
      expect(within(verdict).queryByText("Nothing waits on you")).toBeNull();
      expect(within(verdict).getByRole("button", { name: "Couldn't check everything" })).toBeTruthy();
      if (kind === "ui.review.badges") return;
      // The holds are unknown: the list says so instead of "no suggestions to confirm".
      await openTab(/^Needs you/);
      const alert = await screen.findByTestId("transfers-held-unavailable");
      expect(screen.queryByTestId("transfers-suggestions-empty")).toBeNull();
      daemon.failing = new Set();
      fireEvent.click(within(alert).getByRole("button", { name: "Try again" }));
      await waitFor(() => expect(verdict.getAttribute("data-state")).toBe("settled"));
      expect(screen.queryByTestId("transfers-held-unavailable")).toBeNull();
    },
  );

  it("shows the suggestions a saved view opens, from another tab or a link, even when none match", async () => {
    const view = mount();
    // Nothing to confirm: the page opens on the linked transfers.
    const linked = within(await screen.findByTestId("transfers-tabs")).getByRole("button", { name: /^Linked/ });
    await waitFor(() => expect(linked.getAttribute("aria-pressed")).toBe("true"));
    view.openView("view-swaps");
    const needs = within(screen.getByTestId("transfers-tabs")).getByRole("button", { name: /^Needs you/ });
    await waitFor(() => expect(needs.getAttribute("aria-pressed")).toBe("true"));
    expect(await screen.findByText("View: Liquid swaps")).toBeTruthy();
    expect(screen.getByText("No suggestion matches this view.")).toBeTruthy();
    cleanup();
    // Followed as a link, it opens there too.
    mount({ savedViewId: "view-swaps" });
    expect(await screen.findByText("View: Liquid swaps")).toBeTruthy();
    expect(within(screen.getByTestId("transfers-tabs")).getByRole("button", { name: /^Needs you/ }).getAttribute("aria-pressed")).toBe("true");
  });

  it("says what Kassiber checks when there is nothing to confirm", async () => {
    daemon.items = [];
    mount();
    expect(await screen.findByTestId("transfers-linked-empty")).toBeTruthy();
    await openTab(/^Needs you/);
    const empty = await screen.findByTestId("transfers-suggestions-empty");
    // What it checks sits behind the info icon, to keep the line short.
    expect(within(empty).getByRole("button", { name: /Kassiber checks every send against what arrived in your other wallets/ })).toBeTruthy();
    expect(within(empty).getByText(/link it yourself with Link manually/)).toBeTruthy();
  });

  it("lists a swap into another asset with a Different asset chip", async () => {
    daemon.items = [];
    daemon.pairs = [
      {
        id: "pair-usdt",
        out_transaction_id: "o-usdt",
        in_transaction_id: "i-usdt",
        kind: "manual",
        policy: "taxable",
        swap_fee_msat: null,
        out: { transaction_id: "o-usdt", wallet: "Liquid hot", wallet_kind: "liquid", asset: "LBTC", occurred_at: "2026-01-01T00:00:00Z", amount_msat: "100000000" },
        in: { transaction_id: "i-usdt", wallet: "Liquid USDT", wallet_kind: "liquid", asset: "USDT", occurred_at: "2026-01-01T00:01:00Z", amount_msat: "100000000" },
      },
    ];
    mount();
    const list = await screen.findByTestId("transfers-linked-list");
    await waitFor(() => expect(within(list).getByText("Different asset")).toBeTruthy());
    expect(within(list).getByText("By you")).toBeTruthy();
  });

  it("writes a send of another asset in that asset on every list", async () => {
    const usdtOut = { transaction_id: "o-usdt", wallet: "Liquid USDT", wallet_kind: "liquid", asset: "USDT", occurred_at: "2026-01-01T00:00:00Z", amount_msat: "10000000000000" };
    const lbtcIn = { transaction_id: "i-lbtc", wallet: "Liquid hot", wallet_kind: "liquid", asset: "LBTC", occurred_at: "2026-01-01T00:01:00Z", amount_msat: "100000000" };
    daemon.items = [];
    daemon.pairs = [{ id: "pair-usdt", out_transaction_id: "o-usdt", in_transaction_id: "i-lbtc", kind: "manual", policy: "taxable", swap_fee_msat: null, out: usdtOut, in: lbtcIn }];
    daemon.unlinked = [
      {
        id: "pair-old", out_transaction_id: "o-usdt-2", in_transaction_id: "i-lbtc-2", kind: "manual", policy: "taxable", swap_fee_msat: null, deleted_at: "2026-11-13T14:02:00Z",
        out: { ...usdtOut, transaction_id: "o-usdt-2", amount_msat: "2500000000000" },
        in: { ...lbtcIn, transaction_id: "i-lbtc-2" },
      },
    ];
    daemon.dismissals = [
      undone({
        id: "dismissal-aside", reason: "user dismissed from review queue", out_transaction_id: "o9", in_transaction_id: "i9",
        out: { transaction_id: "o9", wallet: "Liquid USDT", asset: "USDT", amount_msat: "750000000000", occurred_at: "2026-01-02T00:00:00Z" },
      }),
    ];
    mount();
    const list = await screen.findByTestId("transfers-linked-list");
    await waitFor(() => expect(within(list).getByText("100 USDT")).toBeTruthy());
    expect(list.textContent).not.toContain("₿100");
    // Not bitcoin, so not counted as bitcoin moved.
    expect(screen.getByRole("region", { name: "Transfers at a glance" }).textContent).toContain("₿0.00000000");
    await openTab(/^Unlinked by you/);
    expect(await screen.findByText("25 USDT")).toBeTruthy();
    await openTab(/^Set aside/);
    expect(within(await screen.findByTestId("transfers-set-aside")).getByText("7.5 USDT")).toBeTruthy();
  });

  it("docks Link manually beside the lists when there is room, one panel at a time", async () => {
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 1300, height: 800, top: 0, left: 0, right: 1300, bottom: 800, x: 0, y: 0, toJSON: () => ({}),
    } as DOMRect);
    try {
      daemon.txs = PEG_TXS;
      mount();
      // A transfer's details are open in the panel; starting a link takes their place.
      fireEvent.click(await screen.findByRole("row", { name: /Show the transfer Strike to Cold storage/ }));
      expect((await screen.findByTestId("unlink-dialog")).getAttribute("data-variant")).toBe("panel");
      fireEvent.click(screen.getByRole("button", { name: "Link manually…" }));
      const panel = await screen.findByTestId("link-manually");
      expect(panel.getAttribute("data-variant")).toBe("panel");
      expect(screen.queryByTestId("unlink-dialog")).toBeNull();
      expect(screen.queryByRole("dialog")).toBeNull();
      // The list stays usable: its tabs still switch with the link in progress.
      fireEvent.click((await within(panel).findAllByTestId("link-candidate"))[0]);
      await waitFor(() => expect(within(panel).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt"));
      await openTab(/^Needs you/);
      await openTab(/^Linked/);
      expect(within(screen.getByTestId("link-manually")).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt");
      // A transfer opened meanwhile comes as a dialog over the page, so the link is not lost.
      fireEvent.click(screen.getByRole("row", { name: /Show the transfer Strike to Cold storage/ }));
      expect((await screen.findByTestId("unlink-dialog")).getAttribute("data-variant")).toBe("dialog");
      expect(screen.getByTestId("link-manually").getAttribute("data-variant")).toBe("panel");
      fireEvent.click(within(screen.getByTestId("unlink-dialog")).getAllByRole("button", { name: "Close" })[0]);
      await waitFor(() => expect(screen.queryByTestId("unlink-dialog")).toBeNull());
      // The link in the panel is the one from before, with its pick, and the transfer's
      // dialog could not have replaced it: it offers no link of its own meanwhile.
      expect(within(screen.getByTestId("link-manually")).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt");
      expect(within(screen.getByTestId("link-manually")).getAllByTestId("move-leg")).toHaveLength(1);
    } finally {
      rect.mockRestore();
    }
  });

  it("asks about the rest of a send linked in part, and links it as its answer", async () => {
    daemon.items = [];
    daemon.txs = {
      "split-out": tx("split-out", { account: "Treasury", amountSat: -4200000 }),
      "split-in": tx("split-in", { account: "Spending", amountSat: 2500000, occurredAt: "2024-09-17T02:30:00Z" }),
    };
    daemon.pairs = [
      {
        ...PEG_OUT_PAIR,
        id: "pair-split",
        out_transaction_id: "split-out",
        in_transaction_id: "split-in",
        kind: "manual",
        policy: "carrying-value",
        deleted_at: null,
        shares_legs: true,
        out: { transaction_id: "split-out", wallet: "Treasury", asset: "BTC", occurred_at: "2024-09-17T02:14:00Z", amount_msat: 2500000000, unlinked_msat: 1700000000 },
        in: { transaction_id: "split-in", wallet: "Spending", asset: "BTC", occurred_at: "2024-09-17T02:30:00Z", amount_msat: 2500000000, unlinked_msat: 0 },
      },
    ];
    daemon.heldItems = [
      {
        transaction_id: "split-out",
        occurred_at: "2024-09-17T02:14:00Z",
        wallet: "Treasury",
        amount_msat: 4200000000,
        reason: "custody_quantity_unresolved",
        actions: [{ kind: "review_pair", transaction_id: "split-out", pair_id: "pair-split" }],
      },
    ];
    mount();
    // It needs the owner, and reads as the question it leaves open rather than as a missing wallet.
    const hold = await screen.findByTestId("transfers-rest-hold");
    expect(screen.queryByTestId("transfers-gap-hold")).toBeNull();
    expect(hold.textContent).toContain("₿0.01700000 of your send from Treasury isn't linked to a receipt yet.");
    expect(hold.textContent).toContain("Where did the rest go?");
    expect(within(await screen.findByTestId("transfers-tabs")).getByRole("button", { name: /^Needs you/ }).textContent).toContain("1");
    fireEvent.click(within(hold).getByRole("button", { name: "It arrived in another transaction: link the rest…" }));
    const dialog = await screen.findByTestId("link-manually");
    // The send is chosen and says what is left; the receipt is the side to pick.
    expect((await within(dialog).findByTestId("move-leg-part")).textContent).toBe("₿0.01700000 of ₿0.04200000 still unlinked");
    expect(within(dialog).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt");
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    // On the Linked list its row is marked, and its details offer the same.
    await openTab(/^Linked/);
    const row = await screen.findByRole("row", { name: /Show the transfer Treasury to Spending/ });
    // Said in words where the others say "By you", with the amount behind the hint.
    expect(within(row).getByTestId("transfers-part-linked").textContent).toBe("Part linked");
    expect(within(row).getByRole("button", { name: /^₿0\.01700000 of the send is still unlinked\. You linked/ })).toBeTruthy();
    fireEvent.click(row);
    const details = await screen.findByTestId("unlink-dialog");
    expect((await within(details).findByTestId("unlink-rest")).textContent).toBe("₿0.01700000 of the send is still unlinked.");
    fireEvent.click(within(details).getByRole("button", { name: "Link the rest…" }));
    await waitFor(() => expect(screen.queryByTestId("unlink-dialog")).toBeNull());
    expect(await screen.findByTestId("link-manually")).toBeTruthy();
  });

  it("offers the rest right after a link that leaves part of its send", async () => {
    daemon.items = [];
    daemon.txs = {
      "split-out": tx("split-out", { account: "Treasury", amountSat: -4200000 }),
      "split-in": tx("split-in", { account: "Spending", amountSat: 2500000, occurredAt: "2024-09-17T02:30:00Z" }),
    };
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Link manually…" }));
    const dialog = await screen.findByTestId("link-manually");
    fireEvent.click((await within(dialog).findAllByTestId("link-candidate"))[0]);
    await waitFor(() => expect(within(dialog).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt"));
    fireEvent.click((await within(dialog).findAllByTestId("link-candidate"))[0]);
    await within(dialog).findByTestId("preview-outcome");
    // The core lists the new pair with what it leaves of the send.
    daemon.pairs = [
      {
        ...PEG_OUT_PAIR,
        id: "pair-new",
        out_transaction_id: "split-out",
        in_transaction_id: "split-in",
        kind: "manual",
        policy: "carrying-value",
        deleted_at: null,
        shares_legs: true,
        out: { transaction_id: "split-out", wallet: "Treasury", asset: "BTC", occurred_at: "2024-09-17T02:14:00Z", amount_msat: 2500000000, unlinked_msat: 1700000000 },
        in: { transaction_id: "split-in", wallet: "Spending", asset: "BTC", occurred_at: "2024-09-17T02:30:00Z", amount_msat: 2500000000, unlinked_msat: 0 },
      },
    ];
    fireEvent.click(within(dialog).getByRole("button", { name: "Link anyway" }));
    const strip = await screen.findByTestId("transfers-strip");
    await waitFor(() => expect(screen.getByTestId("transfers-strip").textContent).toContain("Linked. ₿0.01700000 of the send is still unlinked."));
    // Under its own row, with the next step first and the way back beside it.
    await waitFor(() =>
      expect(screen.getByTestId("transfers-strip").previousElementSibling?.getAttribute("aria-label")).toBe("Show the transfer Treasury to Spending"),
    );
    expect(within(strip).getByRole("button", { name: "Undo" })).toBeTruthy();
    fireEvent.click(within(screen.getByTestId("transfers-strip")).getByRole("button", { name: "Link the rest…" }));
    const next = await screen.findByTestId("link-manually");
    expect((await within(next).findByTestId("move-leg-part")).textContent).toBe("₿0.01700000 of ₿0.04200000 still unlinked");
    // Not going on with the rest keeps the first link's way back.
    fireEvent.click(within(next).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByTestId("link-manually")).toBeNull());
    expect(within(screen.getByTestId("transfers-strip")).getByRole("button", { name: "Undo" })).toBeTruthy();
    // The strip's words are also in the page's one live region, so they are read out.
    const live = screen.getByTestId("transfers-strip").ownerDocument.querySelector('[aria-live="polite"]');
    expect(live?.textContent).toBe("Linked. ₿0.01700000 of the send is still unlinked.");
    expect(screen.queryByTestId("transfers-toast")).toBeNull();
  });

  it("keeps the missing-wallet review on the rest of a send, and reads a small shortfall as what it was before", async () => {
    daemon.items = [];
    const split = (unlinked: number) => ({
      ...PEG_OUT_PAIR,
      id: "pair-split",
      out_transaction_id: "split-out",
      in_transaction_id: "split-in",
      kind: "manual",
      policy: "carrying-value",
      deleted_at: null,
      swap_fee_msat: null,
      shares_legs: true,
      out: { transaction_id: "split-out", wallet: "Treasury", asset: "BTC", occurred_at: "2024-09-17T02:14:00Z", amount_msat: 4200000000 - unlinked, unlinked_msat: unlinked },
      in: { transaction_id: "split-in", wallet: "Spending", asset: "BTC", occurred_at: "2024-09-17T02:30:00Z", amount_msat: 4200000000 - unlinked, unlinked_msat: 0 },
    });
    daemon.txs = {
      "split-out": tx("split-out", { account: "Treasury", amountSat: -4200000 }),
      "split-in": tx("split-in", { account: "Spending", amountSat: 4180000, occurredAt: "2024-09-17T02:30:00Z" }),
    };
    // The core's residual hold names the pair and the missing-wallet review.
    daemon.heldItems = [
      {
        transaction_id: "split-out",
        occurred_at: "2024-09-17T02:14:00Z",
        wallet: "Treasury",
        amount_msat: 4200000000,
        reason: "custody_quantity_unresolved",
        evidence: { blocker_code: "reviewed_residual_suspense", pair_id: "pair-split" },
        actions: [
          { kind: "review_pair", transaction_id: "split-out", pair_id: "pair-split" },
          { kind: "review_custody_gap", gap_id: "gap-7" },
        ],
      },
    ];
    // Most of the send is missing: another receipt is one answer, and the hint says there are others.
    daemon.pairs = [split(1700000000)];
    const first = mount();
    const rest = await screen.findByTestId("transfers-rest-hold");
    expect(within(rest).getByText(/If something else happened to the rest, turn on developer tools/)).toBeTruthy();
    expect(within(rest).queryByRole("button", { name: /Review in advanced tools/ })).toBeNull();
    first.unmount();
    useUiStore.setState({ developerToolsEnabled: true });
    const second = mount();
    fireEvent.click(within(await screen.findByTestId("transfers-rest-hold")).getByRole("button", { name: /Review in advanced tools/ }));
    expect(onOpen).toHaveBeenCalledWith({ section: "advanced", tab: "gaps", gap: "gap-7" });
    second.unmount();
    useUiStore.setState({ developerToolsEnabled: false });
    // An exchange took 20,000 sats off the amount: no second receipt exists to ask for.
    daemon.pairs = [split(20000000)];
    mount();
    const hold = await screen.findByTestId("transfers-gap-hold");
    expect(screen.queryByTestId("transfers-rest-hold")).toBeNull();
    expect(within(hold).getByText(/Turn on developer tools in Settings/)).toBeTruthy();
    await openTab(/^Linked/);
    const row = await screen.findByRole("row", { name: /Show the transfer Treasury to Spending/ });
    expect(within(row).queryByTestId("transfers-part-linked")).toBeNull();
    fireEvent.click(row);
    const details = await screen.findByTestId("unlink-dialog");
    // Its details name the difference as the fee between the two, as for any link.
    expect(await within(details).findByText("Fee between them")).toBeTruthy();
    expect(within(details).queryByTestId("unlink-rest")).toBeNull();
    expect(within(details).queryByRole("button", { name: "Link the rest…" })).toBeNull();
  });

  it("does not read a hold for another reason as a rest because it names a pair", async () => {
    daemon.items = [];
    daemon.pairs = [
      {
        ...PEG_OUT_PAIR,
        id: "pair-old",
        kind: "manual",
        policy: "carrying-value",
        deleted_at: null,
        shares_legs: true,
        out: { ...PEG_OUT_PAIR.out, asset: "BTC", amount_msat: 2500000000, unlinked_msat: 1700000000 },
        in: { ...PEG_OUT_PAIR.in, amount_msat: 2500000000, unlinked_msat: 0 },
      },
    ];
    daemon.heldItems = [
      {
        transaction_id: "peg-out",
        occurred_at: "2024-09-17T02:14:00Z",
        wallet: "Liquid Treasury",
        amount_msat: 4200000000,
        reason: "custody_quantity_unresolved",
        evidence: { review_codes: ["legacy_pair_fee_allocation_review_required"] },
        actions: [{ kind: "review_pair", transaction_id: "peg-out", pair_id: "pair-old" }],
      },
    ];
    mount();
    await screen.findByTestId("transfers-linked-list");
    expect(screen.queryByTestId("transfers-rest-hold")).toBeNull();
    expect(within(await screen.findByTestId("transfers-tabs")).getByRole("button", { name: /^Needs you/ }).textContent).toContain("0");
  });

  it("looks for transfers again from the menu and from the empty suggestions", async () => {
    mount();
    await openTab(/^Needs you/);
    const empty = await screen.findByTestId("transfers-suggestions-empty");
    expect((within(empty).getByRole("button", { name: "Check again" }) as HTMLButtonElement).disabled).toBe(false);
    const menu = await openRowMenu(/More transfer tools/);
    const item = within(menu).getByRole("menuitem", { name: /Look for transfers again/ });
    expect(item.textContent).toContain("Recalculates the journals and checks every send again");
    expect(item.getAttribute("aria-disabled")).not.toBe("true");
  });

  it("leads from an unlinked transfer to saying what each side really was", async () => {
    daemon.items = [];
    daemon.dismissals = [undone()];
    mount();
    await openTab(/^Unlinked by you/);
    const menu = await openRowMenu(/More for BTCPay Bark to Store on-chain/);
    // Each side is decided on its own page, where the core previews the change.
    expect(within(menu).getByRole("menuitem", { name: "Sent to someone else? Say what it was…" }).getAttribute("href")).toBe(
      "/transactions?tx=out-bark&tab=tax",
    );
    expect(within(menu).getByRole("menuitem", { name: "Received from someone else? Say what it was…" }).getAttribute("href")).toBe(
      "/transactions?tx=in-store&tab=tax",
    );
    expect(within(menu).getByRole("menuitem", { name: "Show the send in Transactions" }).getAttribute("href")).toBe("/transactions?tx=out-bark");
  });

  it("explains a missing-wallet hold read-only without developer tools", async () => {
    daemon.items = [];
    daemon.heldItems = [
      {
        transaction_id: "gap-root",
        occurred_at: "2026-02-01T10:00:00Z",
        wallet: "Cold storage",
        amount_msat: 250000000,
        reason: "custody_quantity_unresolved",
        actions: [{ kind: "review_custody_gap", gap_id: "gap-1" }],
      },
    ];
    mount();
    const hold = await screen.findByTestId("transfers-gap-hold");
    expect(within(hold).getByText(/may have come back through a wallet you haven't connected/)).toBeTruthy();
    expect(within(hold).queryByRole("button")).toBeNull();
    expect(within(hold).getByText(/Turn on developer tools in Settings/)).toBeTruthy();
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    mount();
    const verdict = await screen.findByTestId("transfers-verdict");
    await waitFor(() => expect(within(verdict).getByText("Nichts wartet auf dich")).toBeTruthy());
    const menu = await openRowMenu(/Aktionen für Strike/);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Trennen/ }));
    const dialog = await screen.findByTestId("unlink-dialog");
    expect(within(dialog).getByText("Diese Verknüpfung trennen?")).toBeTruthy();
    expect(within(dialog).getByText("Gebühr dazwischen")).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Verknüpfung behalten" })).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Verknüpfung behalten" }));
    fireEvent.click((await screen.findAllByRole("button", { name: "Manuell verknüpfen…" }))[0]);
    const link = await screen.findByTestId("link-manually");
    expect(within(link).getByRole("button", { name: /Gesendet.*Wähle den Ausgang aus der Liste unten/ })).toBeTruthy();
    expect(within(link).getByRole("button", { name: /Erhalten.*Eingang wählen…/ })).toBeTruthy();
  });
});
