// @vitest-environment happy-dom
//
// "Link manually" on its own: where it opens for each way in, what the list
// offers and refuses to offer, what the check says and asks, and how it
// behaves when the core is slow, refuses, or the book changes under it.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui";

import { forgetFinishedPreviews } from "./reviewPreview";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

type Fail = { code: string; message: string };

const daemon = vi.hoisted(() => ({
  txs: {} as Record<string, Record<string, unknown>>,
  candidates: [] as unknown[],
  hasMore: false,
  /** Errors the next plans answer with, first first. */
  planFails: [] as Array<{ code: string; message: string }>,
  /** Errors the next applies answer with. */
  applyFails: [] as Array<{ code: string; message: string }>,
  /** Holds every plan until released. */
  planGate: null as Promise<void> | null,
  version: 7,
  heldAfter: [] as string[],
  taxYears: [] as unknown[],
  /** The owner's pairs, as `ui.transfers.list` returns them. */
  pairs: [] as unknown[],
  /** What `ui.transfers.pair_options` answers; null is a core that doesn't know the kind. */
  pairOptions: null as unknown[] | null,
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  const ok = (kind: string, data: unknown) => ({ kind, schema_version: 1, data });
  const failed = (error: Fail) => ({ kind: "error", schema_version: 1, error: { ...error, retryable: false } });
  const effects = (held: string[]) => ({
    entries_count: 10,
    quarantine_count: held.length,
    report_ready: held.length === 0,
    quarantines: held.map((id) => ({ transaction_id: id, reason: "recorded_transfer_authority_conflict" })),
  });
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        switch (kind) {
          case "ui.profiles.snapshot":
            return ok(kind, { workspaces: [], activeWorkspaceId: "w1", activeProfileId: "p1" });
          case "ui.review.cases":
            return ok(kind, { input_version: daemon.version, cases: [] });
          case "ui.review.plan": {
            if (daemon.planGate) await daemon.planGate;
            const fail = daemon.planFails.shift();
            if (fail) return failed(fail);
            return ok(kind, {
              schema_version: 1,
              workspace_id: "w1",
              profile_id: "p1",
              base_input_version: daemon.version,
              // One digest per plan, so a test can tell which plan was applied.
              digest: String(daemon.calls.filter((call) => call.kind === "ui.review.plan").length).padStart(64, "0"),
              operations: args.operations,
              before: effects([]),
              after: { ...effects(daemon.heldAfter), tax: { fiat_currency: "EUR", jurisdiction: null, years: [] } },
              delta: { years: daemon.taxYears },
            });
          }
          case "ui.review.apply": {
            const fail = daemon.applyFails.shift();
            if (fail) return failed(fail);
            return ok(kind, { operations: [{ type: "pair", result: { pair_id: "pair-new" } }] });
          }
          case "ui.transactions.resolve":
            return ok(kind, { transaction: daemon.txs[String(args.query)] ?? null });
          case "ui.transactions.list": {
            const direction = args.direction === "outbound" ? -1 : 1;
            const query = typeof args.query === "string" ? args.query.toLowerCase() : "";
            const since = typeof args.since === "string" ? Date.parse(args.since) : null;
            const until = typeof args.until === "string" ? Date.parse(args.until) : null;
            const txs = Object.values(daemon.txs).filter((tx) => {
              const at = Date.parse(String(tx.occurredAt));
              return (
                tx.direction === direction &&
                (!query || String(tx.account).toLowerCase().includes(query)) &&
                (since === null || at >= since) &&
                (until === null || at <= until)
              );
            });
            return ok(kind, { txs, year: 2024, hasMore: daemon.hasMore });
          }
          case "ui.transfers.suggest":
            return ok(kind, { candidates: daemon.candidates });
          case "ui.transfers.list":
            return ok(kind, { pairs: daemon.pairs, unlinked: [] });
          case "ui.transfers.pair_options":
            return daemon.pairOptions ? ok(kind, { policies: daemon.pairOptions }) : failed({ code: "invalid_kind", message: "unknown kind" });
          default:
            return failed({ code: "not_found", message: "unavailable" });
        }
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, search, ...rest }: { children: React.ReactNode; to: string; search?: Record<string, string>; "aria-label"?: string }) => (
    <a href={`${to}?${new URLSearchParams(search ?? {}).toString()}`} aria-label={rest["aria-label"]}>
      {children}
    </a>
  ),
}));

import { LinkManuallyDialog, type LinkStart } from "./LinkManuallyDialog";

/** A transaction as the daemon lists it; `direction` only steers the mock's list. */
function tx(id: string, wallet: string, amountSat: number, overrides: Record<string, unknown> = {}) {
  return {
    id,
    externalId: `${id}-txid-0123456789abcdef0123456789abcdef`,
    date: "2024-09-17 02:14",
    occurredAt: "2024-09-17T02:14:00Z",
    type: amountSat < 0 ? "Withdrawal" : "Deposit",
    account: wallet,
    counter: "",
    amountSat,
    eur: null,
    rate: null,
    tag: "",
    conf: 1,
    direction: Math.sign(amountSat) || 1,
    ...overrides,
  };
}

const onLinked = vi.fn();
const onClose = vi.fn();

function mount(start: LinkStart, updating = false, variant: "dialog" | "panel" = "dialog") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const tree = (now: boolean) => (
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <LinkManuallyDialog start={start} variant={variant} updating={now} onClose={onClose} onLinked={onLinked} />
      </TooltipProvider>
    </QueryClientProvider>
  );
  const view = render(tree(updating));
  return { setUpdating: (now: boolean) => view.rerender(tree(now)) };
}

const calls = (kind: string) => daemon.calls.filter((call) => call.kind === kind);
const dialog = () => screen.findByTestId("link-manually");
const rows = (root: HTMLElement) => within(root).findAllByTestId("link-candidate");
const confirmButton = (root: HTMLElement, name: string) => within(root).getByRole("button", { name }) as HTMLButtonElement;

beforeEach(async () => {
  await i18n.changeLanguage("en");
  useUiStore.setState({ hideSensitive: false, currency: "btc" });
  daemon.txs = {
    send: tx("send", "Cold storage", -4200000, { feeSat: 1000 }),
    near: tx("near", "Spending", 4200000, { occurredAt: "2024-09-17T02:30:00Z" }),
  };
  daemon.candidates = [];
  daemon.hasMore = false;
  daemon.planFails = [];
  daemon.applyFails = [];
  daemon.planGate = null;
  daemon.version = 7;
  daemon.heldAfter = [];
  daemon.taxYears = [];
  daemon.pairs = [];
  daemon.pairOptions = null;
  daemon.calls = [];
  onLinked.mockClear();
  onClose.mockClear();
  forgetFinishedPreviews();
});

afterEach(() => cleanup());

describe("Link manually: where it opens", () => {
  it("opens on the receipts for a send it is given, and on the sends for a receipt", async () => {
    mount({ transactionId: "send" });
    const first = await dialog();
    expect((await within(first).findByTestId("link-pick")).getAttribute("data-side")).toBe("receipt");
    expect(within(first).getAllByTestId("move-leg")).toHaveLength(1);
    // The open side is a button that says what to do, and is marked as the one being chosen.
    expect(within(first).getByRole("button", { name: /Received.*Choose the receipt from the list below/ }).getAttribute("aria-pressed")).toBe("true");
    cleanup();

    mount({ transactionId: "near" });
    const second = await dialog();
    expect((await within(second).findByTestId("link-pick")).getAttribute("data-side")).toBe("send");
    expect(calls("ui.transactions.list").at(-1)?.args).toEqual(
      // Sends in the three days before the receipt, with an hour of clock drift after it.
      expect.objectContaining({ direction: "outbound", since: "2024-09-14T02:30:00Z", until: "2024-09-17T03:30:00Z" }),
    );
  });

  it("says so when the transaction it was given is not in this book, and lets both sides be chosen", async () => {
    mount({ transactionId: "not-in-this-book" });
    const root = await dialog();
    expect((await within(root).findByTestId("link-notice")).textContent).toBe("That transaction isn't in this book. Choose both sides here.");
    expect(within(root).getAllByTestId("link-slot")).toHaveLength(2);
    expect(within(root).getByTestId("link-pick").getAttribute("data-side")).toBe("send");
    // Either side can be chosen first.
    fireEvent.click(within(root).getByRole("button", { name: /Received.*Choose the receipt…/ }));
    expect(within(root).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt");
  });

  it("opens a side again when a transaction of a pair it was given is gone", async () => {
    mount({ sendId: "send", receiptId: "deleted", relink: true });
    const root = await dialog();
    expect((await within(root).findByTestId("link-notice")).textContent).toContain("no longer in this book");
    expect(within(root).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt");
    expect(calls("ui.review.plan")).toHaveLength(0);
  });

  it("warns at once when the transaction it was given is already linked", async () => {
    daemon.txs.send = tx("send", "Cold storage", -4200000, {
      pair: { id: "p", type: "transfer", kind: "manual", outWallet: "Cold storage", inWallet: "Savings" },
      internal: true,
    });
    mount({ transactionId: "send" });
    const root = await dialog();
    expect((await within(root).findByTestId("link-warnings")).textContent).toContain("The send is already linked to Savings.");
  });
});

describe("Link manually: as a dialog", () => {
  it("stays open when the search or a candidate is pressed, and closes with Escape", async () => {
    mount({ transactionId: "send" });
    const root = await dialog();
    expect(root.getAttribute("data-variant")).toBe("dialog");
    const search = await within(root).findByRole("textbox", { name: "Search" });
    // A press as a mouse makes it: the content is inside the dialog, not outside it.
    fireEvent.pointerDown(search, { button: 0, pointerType: "mouse" });
    fireEvent.focus(search);
    fireEvent.focusIn(search);
    const [candidate] = await rows(root);
    fireEvent.pointerDown(candidate, { button: 0, pointerType: "mouse" });
    fireEvent.pointerUp(candidate, { button: 0, pointerType: "mouse" });
    fireEvent.click(candidate);
    await within(root).findByTestId("preview-outcome");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByTestId("link-manually")).toBe(root);
    // Escape still closes it.
    fireEvent.keyDown(document.activeElement ?? root, { key: "Escape" });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });
});

describe("Link manually: docked beside the page", () => {
  it("is a panel with a name instead of a dialog, takes focus once and closes with its button or Escape", async () => {
    mount({ transactionId: "send" }, false, "panel");
    const panel = await dialog();
    expect(panel.getAttribute("data-variant")).toBe("panel");
    expect(panel.tagName).toBe("ASIDE");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("complementary", { name: "Link manually" })).toBe(panel);
    expect(document.activeElement).toBe(within(panel).getByRole("heading", { name: "Link manually" }));
    // The same list and the same check as the dialog.
    fireEvent.click((await rows(panel))[0]);
    await within(panel).findByTestId("preview-outcome");
    fireEvent.keyDown(within(panel).getByTestId("link-check"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.click(within(panel).getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("renders nothing while there is nothing to link", () => {
    mount(null as unknown as LinkStart, false, "panel");
    expect(screen.queryByTestId("link-manually")).toBeNull();
  });
});

describe("Link manually: one tree for both frames", () => {
  it("keeps the picks, the search and the check when the page changes between panel and dialog", async () => {
    daemon.txs.far = tx("far", "Treasury", 4100000, { occurredAt: "2024-09-18T02:14:00Z" });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const tree = (variant: "dialog" | "panel") => (
      <QueryClientProvider client={client}>
        <TooltipProvider>
          <LinkManuallyDialog start={{ transactionId: "send" }} variant={variant} onClose={onClose} onLinked={onLinked} />
        </TooltipProvider>
      </QueryClientProvider>
    );
    const view = render(tree("panel"));
    const panel = await dialog();
    await rows(panel);
    fireEvent.change(within(panel).getByRole("textbox", { name: "Search" }), { target: { value: "treas" } });
    await waitFor(async () => expect(await rows(panel)).toHaveLength(1));
    // Narrower: the same search and list, now in a dialog.
    view.rerender(tree("dialog"));
    const narrow = await dialog();
    expect(narrow.getAttribute("data-variant")).toBe("dialog");
    expect((within(narrow).getByRole("textbox", { name: "Search" }) as HTMLInputElement).value).toBe("treas");
    fireEvent.click((await rows(narrow))[0]);
    await within(narrow).findByTestId("preview-outcome");
    const plans = calls("ui.review.plan").length;
    // Wider again, with the check done: nothing is picked or checked a second time.
    view.rerender(tree("panel"));
    const wide = await dialog();
    expect(wide.getAttribute("data-variant")).toBe("panel");
    expect(within(wide).getByTestId("preview-outcome")).toBeTruthy();
    expect(within(wide).getAllByTestId("move-leg")).toHaveLength(2);
    expect(calls("ui.review.plan")).toHaveLength(plans);
    fireEvent.click(confirmButton(wide, "Link these two"));
    await waitFor(() => expect(onLinked).toHaveBeenCalledTimes(1));
    expect(calls("ui.review.apply")).toHaveLength(1);
  });

  it("gives focus back to where the panel was opened from", async () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const view = render(
      <QueryClientProvider client={client}>
        <TooltipProvider>
          <LinkManuallyDialog start={{ transactionId: "send" }} variant="panel" onClose={onClose} onLinked={onLinked} />
        </TooltipProvider>
      </QueryClientProvider>,
    );
    const panel = await dialog();
    expect(document.activeElement).toBe(within(panel).getByRole("heading", { name: "Link manually" }));
    view.rerender(
      <QueryClientProvider client={client}>
        <TooltipProvider>
          <LinkManuallyDialog start={null} variant="panel" onClose={onClose} onLinked={onLinked} />
        </TooltipProvider>
      </QueryClientProvider>,
    );
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });
});

describe("Link manually: a send that arrived in parts", () => {
  beforeEach(() => {
    // The send's first receipt took 2,500,000 of its 4,200,000 sats.
    daemon.txs.first = tx("first", "Exchange", 2500000, { occurredAt: "2024-09-17T02:20:00Z" });
    daemon.txs.near = tx("near", "Spending", 1700000, { occurredAt: "2024-09-17T02:30:00Z" });
    daemon.pairs = [
      {
        id: "pair-1",
        shares_legs: true,
        out: { transaction_id: "send", amount_msat: 2500000000, unlinked_msat: 1700000000 },
        in: { transaction_id: "first", amount_msat: 2500000000, unlinked_msat: 0 },
      },
    ];
  });

  it("can be picked again for the part that is left, which the list and the check are then about", async () => {
    mount({ transactionId: "send" });
    const root = await dialog();
    // Linked in part is no reason to warn: it says what is left instead.
    expect((await within(root).findByTestId("move-leg-part")).textContent).toBe("₿0.01700000 of ₿0.04200000 still unlinked");
    expect(within(root).queryByTestId("link-warnings")).toBeNull();
    const list = await rows(root);
    // What matches the rest comes first; the receipt that is covered in full can't be picked.
    expect(list[0].textContent).toContain("Spending");
    expect(list[0].textContent).toContain("Same amount");
    expect(list[1].getAttribute("data-blocked")).toBe("linked");
    fireEvent.click(list[0]);
    await within(root).findByTestId("preview-outcome");
    expect(within(root).getByTestId("link-part-note").textContent).toBe(
      "₿0.02500000 of this send is already linked. This link is about the ₿0.01700000 that is left.",
    );
    expect(within(root).getByTestId("link-difference").textContent).toBe("This link covers the rest of the send.");
    expect(within(root).queryByTestId("link-warnings")).toBeNull();
    // An ordinary link of the two: the core takes what is left of the send.
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([
      { type: "pair", out_transaction_id: "send", in_transaction_id: "near", pair_kind: "manual", policy: "carrying-value", reason: "Linked by hand on Transfers" },
    ]);
    expect(confirmButton(root, "Link these two").disabled).toBe(false);
  });

  it("says what stays open when the next receipt does not cover the rest either", async () => {
    daemon.txs.near = tx("near", "Spending", 700000, { occurredAt: "2024-09-17T02:30:00Z" });
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect(within(root).getByTestId("link-difference").textContent).toBe("After this link, ₿0.01000000 of the send is still unlinked.");
    // Less than the rest is expected of a part, so nothing is flagged.
    expect(within(root).queryByTestId("link-warnings")).toBeNull();
    expect(within(root).queryByTestId("link-rest-hint")).toBeNull();
  });

  it("points to linking the rest when a first link leaves most of the send", async () => {
    daemon.pairs = [];
    mount({ sendId: "send", receiptId: "first" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    // One message for the shortfall, naming both things it can mean, and one button.
    const warnings = within(root).getByTestId("link-warnings");
    expect(within(warnings).getAllByRole("listitem")).toHaveLength(1);
    expect(warnings.textContent).toBe(
      "Far less arrived than was sent. Either these are not the same coins, or the rest arrived in another transaction. If it did, link this part now and the rest right after.",
    );
    expect(confirmButton(root, "Link anyway")).toBeTruthy();
  });

  it("does not offer the rest of a send to a swap or a change of network, which the core would refuse", async () => {
    daemon.txs.liquid = tx("liquid", "Liquid", 1700000, { asset: "LBTC", paymentMethod: "Liquid", occurredAt: "2024-09-17T02:25:00Z" });
    daemon.txs.ln = tx("ln", "Node", 1700000, { paymentMethod: "Lightning", occurredAt: "2024-09-17T02:26:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    await waitFor(async () => expect(await rows(root)).toHaveLength(4));
    const list = await rows(root);
    const blocked = list.filter((row) => row.getAttribute("data-blocked") === "rest");
    expect(blocked.map((row) => within(row).getAllByRole("cell")[1].textContent)).toEqual([
      expect.stringContaining("Can't take the rest of a transfer: this would be a swap or a change of network."),
      expect.stringContaining("Can't take the rest of a transfer"),
    ]);
    // They stay listed, after what can be picked, and a click does nothing.
    expect(list[0].textContent).toContain("Spending");
    fireEvent.click(blocked[0]);
    expect(within(root).getByTestId("link-pick").getAttribute("data-side")).toBe("receipt");
    cleanup();
    // Given both from outside, the check says the send is linked already instead of promising the rest.
    mount({ sendId: "send", receiptId: "liquid" });
    const check = await dialog();
    expect((await within(check).findByTestId("link-warnings")).textContent).toContain("The send is already linked");
    expect(within(check).queryByTestId("link-part-note")).toBeNull();
  });

  it("reads a small shortfall on an ordinary link as a difference, not a part to link", async () => {
    // The exchange took 20,000 sats off the amount: under 1% of the send.
    daemon.pairs = [
      {
        id: "pair-1",
        shares_legs: true,
        out: { transaction_id: "send", amount_msat: 4180000000, unlinked_msat: 20000000 },
        in: { transaction_id: "first", amount_msat: 4180000000, unlinked_msat: 0 },
      },
    ];
    mount({ transactionId: "send" });
    const root = await dialog();
    expect((await within(root).findByTestId("link-warnings")).textContent).toContain("The send is already linked");
    expect(within(root).queryByTestId("move-leg-part")).toBeNull();
  });

  it("stays one to one where the core says a link takes no other", async () => {
    daemon.pairs = [{ ...(daemon.pairs[0] as object), shares_legs: false }];
    mount({ transactionId: "send" });
    const root = await dialog();
    expect((await within(root).findByTestId("link-warnings")).textContent).toContain("The send is already linked");
    expect(within(root).queryByTestId("move-leg-part")).toBeNull();
  });
});

describe("Link manually: the list", () => {
  it("is a table with what each transaction differs by and is booked as, the matcher's suggestion first", async () => {
    daemon.txs.far = tx("far", "Treasury", 4100000, { occurredAt: "2024-09-18T02:14:00Z" });
    daemon.txs.grew = tx("grew", "Exchange", 5000000, { occurredAt: "2024-09-17T02:20:00Z", quarantineReason: "missing_price" });
    daemon.candidates = [{ out_id: "send", in_id: "far", default_kind: "manual", default_policy: "carrying-value" }];
    mount({ transactionId: "send" });
    const root = await dialog();
    await waitFor(async () => expect(await rows(root)).toHaveLength(3));
    const table = within(root).getByRole("table", { name: "Receipts" });
    expect(within(table).getAllByRole("columnheader").map((cell) => cell.textContent)).toEqual([
      "When",
      "Wallet",
      "Amount",
      "Difference",
      "Booked as",
      "Show in Transactions",
    ]);
    // Every row leads to its transaction on the Transactions page.
    expect(within((await rows(root))[0]).getByRole("link", { name: "Show in Transactions" }).getAttribute("href")).toBe("/transactions?tx=far");
    const list = await rows(root);
    expect(list.map((row) => within(row).getAllByRole("cell")[1].textContent)).toEqual(["TreasurySuggested", "SpendingClose match", "ExchangeOn hold"]);
    expect(list[0].textContent).toContain("101,000 sats less");
    expect(list[1].textContent).toContain("16 min later");
    expect(list[1].textContent).toContain("1,000 sats less");
    expect(list[1].textContent).toContain("Deposit");
    // More arrived than left: listed, not hidden, and said in words.
    expect(list[2].textContent).toContain("799,000 sats more");
    expect(within(list[2]).getAllByRole("cell").at(-2)?.textContent).toBe("On hold");
  });

  it("keeps linked and excluded transactions listed last, where they can't be picked", async () => {
    daemon.txs.linked = tx("linked", "Savings <- Old", 0, {
      direction: 1,
      type: "Transfer",
      internal: true,
      pair: { id: "p", type: "transfer", kind: "journal-derived", outWallet: "Old", inWallet: "Savings", outAsset: "BTC", inAsset: "BTC", outAmountSat: 4200000, inAmountSat: 4200000 },
    });
    daemon.txs.out = tx("out", "Dust", 4200000, { excluded: true });
    mount({ transactionId: "send" });
    const root = await dialog();
    await waitFor(async () => expect(await rows(root)).toHaveLength(3));
    const list = await rows(root);
    expect(list.map((row) => row.getAttribute("data-blocked"))).toEqual([null, "linked", "ignored"]);
    // The linked row shows its own leg, not the move the transaction list makes of it.
    expect(list[1].textContent).toContain("SavingsLinked to Old. Unlink it first.");
    expect(list[1].textContent).toContain("₿0.04200000");
    expect(list[2].textContent).toContain("Excluded. Include it again first.");
    expect(list[1].getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(list[1]);
    fireEvent.keyDown(list[2], { key: "Enter" });
    expect(root.getAttribute("data-state")).toBe("pick");
  });

  it("is worked with the keyboard: down from the search, arrows between rows, Enter to choose", async () => {
    daemon.txs.far = tx("far", "Treasury", 4100000, { occurredAt: "2024-09-18T02:14:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    await waitFor(async () => expect(await rows(root)).toHaveLength(2));
    const list = await rows(root);
    // One row is in the tab order.
    expect(list.map((row) => row.tabIndex)).toEqual([0, -1]);
    fireEvent.keyDown(within(root).getByRole("textbox", { name: "Search" }), { key: "ArrowDown" });
    expect(document.activeElement).toBe(list[0]);
    fireEvent.keyDown(list[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(list[1]);
    fireEvent.keyDown(list[1], { key: "ArrowDown" });
    expect(document.activeElement).toBe(list[1]);
    fireEvent.keyDown(list[1], { key: "Home" });
    expect(document.activeElement).toBe(list[0]);
    fireEvent.keyDown(list[0], { key: "End" });
    fireEvent.keyDown(list[1], { key: "Enter" });
    await waitFor(() => expect(root.getAttribute("data-state")).toBe("check"));
    expect(within(root).getAllByTestId("move-leg")[1].textContent).toContain("Treasury");
  });

  it("searches by text, and brings the nearest amounts first when an amount is typed", async () => {
    daemon.txs.far = tx("far", "Treasury", 4100000, { occurredAt: "2024-09-18T02:14:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    await waitFor(async () => expect(await rows(root)).toHaveLength(2));
    const search = within(root).getByRole("textbox", { name: "Search" });
    fireEvent.change(search, { target: { value: "0.041" } });
    await waitFor(async () => expect((await rows(root))[0].textContent).toContain("Treasury"));
    // An amount orders the list; it is not sent as a text search.
    expect(calls("ui.transactions.list").every((call) => call.args.query === undefined)).toBe(true);
    fireEvent.change(search, { target: { value: "spend" } });
    await waitFor(async () => expect(await rows(root)).toHaveLength(1));
    expect(calls("ui.transactions.list").at(-1)?.args.query).toBe("spend");
  });

  it("says what time it looked through when nothing is in reach, and widens it one step at a time", async () => {
    daemon.txs.near = tx("near", "Spending", 4200000, { occurredAt: "2024-12-01T00:00:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    const empty = await within(root).findByTestId("link-pick-empty");
    expect(empty.textContent).toContain("No receipt within 3 days of the send.");
    // One button, the next wider time each press: a week, 30 days, any time.
    for (const [button, said] of [
      ["Search 1 week", "No receipt within 1 week of the send."],
      ["Search 30 days", "No receipt within 30 days of the send."],
    ] as const) {
      fireEvent.click(within(root).getByRole("button", { name: button }));
      await waitFor(() => expect(within(root).getByTestId("link-pick-empty").textContent).toContain(said));
    }
    expect(calls("ui.transactions.list").at(-1)?.args).toEqual(
      expect.objectContaining({ since: "2024-09-17T01:14:00Z", until: "2024-10-17T02:14:00Z" }),
    );
    fireEvent.click(within(root).getByRole("button", { name: "Search any time" }));
    await waitFor(async () => expect(await rows(root)).toHaveLength(1));
    expect(calls("ui.transactions.list").at(-1)?.args).toEqual({ direction: "inbound", limit: 200 });
    // The select follows, and nothing wider is left to offer.
    expect(within(root).getByRole("combobox", { name: "Arrived within" }).textContent).toBe("Any time");
    expect(within(root).queryByTestId("link-widen")).toBeNull();
    // Months later is listed, with how far off it is.
    expect((await rows(root))[0].textContent).toContain("75 days later");
  });

  it("offers to widen the time when the list has nothing that fits", async () => {
    // Twice what was sent: listed, but not what the send looks like.
    daemon.txs.near = tx("near", "Spending", 9000000, { occurredAt: "2024-09-17T02:30:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    await rows(root);
    const thin = within(root).getByTestId("link-pick-thin");
    expect(thin.textContent).toContain("Not here? Only receipts within 3 days of the send are listed.");
    fireEvent.click(within(thin).getByRole("button", { name: "Search 1 week" }));
    await waitFor(() =>
      expect(calls("ui.transactions.list").at(-1)?.args).toEqual(expect.objectContaining({ until: "2024-09-24T02:14:00Z" })),
    );
    cleanup();
    // A close match needs no wider search, and a text search is not a time problem.
    daemon.txs.near = tx("near", "Spending", 4200000, { occurredAt: "2024-09-17T02:30:00Z" });
    mount({ transactionId: "send" });
    const close = await dialog();
    await rows(close);
    expect(within(close).queryByTestId("link-pick-thin")).toBeNull();
    fireEvent.change(within(close).getByRole("textbox", { name: "Search" }), { target: { value: "nothing like it" } });
    const empty = await within(close).findByTestId("link-pick-empty");
    expect(empty.textContent).toContain("No receipt found.");
    expect(within(empty).getByRole("button", { name: "Show all" })).toBeTruthy();
    expect(within(close).queryByTestId("link-widen")).toBeNull();
  });

  it("says when the book has more than it lists", async () => {
    daemon.hasMore = true;
    mount({});
    const root = await dialog();
    await rows(root);
    expect(within(root).getByTestId("link-pick-limited").textContent).toBe("Showing the newest 1. Search or pick a year to find older ones.");
    // Nothing to compare with yet, so no difference is claimed.
    expect(within(within(root).getByRole("table", { name: "Sends" })).queryByRole("columnheader", { name: "Difference" })).toBeNull();
  });

  it("offers another choice for a side and keeps the one it had", async () => {
    daemon.txs.far = tx("far", "Treasury", 4100000, { occurredAt: "2024-09-18T02:14:00Z" });
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("link-check");
    fireEvent.click(within(root).getByRole("button", { name: "Choose another receipt" }));
    await waitFor(async () => expect(await rows(root)).toHaveLength(2));
    expect((await rows(root)).map((row) => row.getAttribute("aria-selected"))).toEqual(["true", "false"]);
    fireEvent.click(within(root).getByRole("button", { name: "Keep this receipt" }));
    expect(root.getAttribute("data-state")).toBe("check");
    expect(within(root).getAllByTestId("move-leg")[1].textContent).toContain("Spending");
  });
});

describe("Link manually: the check", () => {
  it("states what a move within one asset counts as, without asking, and links on one confirmation", async () => {
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect(within(root).getByTestId("link-counts-as").textContent).toBe("Moving your own bitcoin: no tax, and the cost basis carries over.");
    expect(within(root).queryByTestId("link-question")).toBeNull();
    expect(within(root).queryByRole("combobox")).toBeNull();
    expect(within(root).getByTestId("link-difference").textContent).toBe("1,000 sats less arrived than was sent (0.02% of the send).");
    expect(within(root).getByText("Tax reports are not held up by anything after this.")).toBeTruthy();
    expect(within(root).getByText("No tax year changes.")).toBeTruthy();
    expect(within(root).queryByTestId("link-warnings")).toBeNull();
    expect(calls("ui.review.plan").map((call) => call.args.operations)).toEqual([
      [{ type: "pair", out_transaction_id: "send", in_transaction_id: "near", pair_kind: "manual", policy: "carrying-value", reason: "Linked by hand on Transfers" }],
    ]);
    const confirm = confirmButton(root, "Link these two");
    // A double click is one link.
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    await waitFor(() => expect(onLinked).toHaveBeenCalledWith({ pairId: "pair-new", sendId: "send", receiptId: "near" }));
    expect(calls("ui.review.apply")).toHaveLength(1);
    expect(onLinked).toHaveBeenCalledTimes(1);
  });

  it("starts from the matcher's own proposal for a pair it suggests", async () => {
    daemon.txs.near = tx("near", "Liquid", 4190000, { asset: "LBTC", paymentMethod: "Liquid", occurredAt: "2024-09-17T02:30:00Z" });
    daemon.candidates = [{ out_id: "send", in_id: "near", default_kind: "chain-swap", default_policy: "carrying-value" }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect((within(root).getByRole("radio", { name: /No, it's still my bitcoin/ }) as HTMLInputElement).checked).toBe(true);
    expect(within(within(root).getByTestId("link-question")).getByText("Suggested")).toBeTruthy();
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([
      expect.objectContaining({ pair_kind: "chain-swap", policy: "carrying-value" }),
    ]);
  });

  it("asks about a swap into another asset in its own words, and compares no amounts", async () => {
    daemon.txs.near = tx("near", "Exchange", 250000000000, { asset: "USDT", occurredAt: "2024-09-17T02:30:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    const list = await rows(root);
    expect(list[0].textContent).toContain("Other asset");
    expect(list[0].textContent).toContain("2500 USDT");
    fireEvent.click(list[0]);
    await within(root).findByTestId("link-unanswered");
    expect(within(root).getByTestId("link-difference").textContent).toBe("The two sides are different assets, so their amounts can't be compared.");
    expect(within(root).getByRole("radiogroup", { name: "Was this a sale?" })).toBeTruthy();
    expect(within(root).getByRole("radio", { name: /No, a swap without tax/ })).toBeTruthy();
    fireEvent.click(within(root).getByRole("radio", { name: /Yes, a sale and a purchase/ }));
    await within(root).findByTestId("preview-outcome");
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([expect.objectContaining({ pair_kind: "manual", policy: "taxable" })]);
  });

  it("states the one answer the book allows instead of asking, with the reason", async () => {
    daemon.txs.near = tx("near", "Exchange", 250000000000, { asset: "USDT", occurredAt: "2024-09-17T02:30:00Z" });
    daemon.pairOptions = [
      { policy: "carrying-value", allowed: false, reason: "cross_asset", message: "Cross-asset carrying-value pairs are only supported for Austrian profiles" },
      { policy: "taxable", allowed: true, reason: null, message: null },
    ];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    // Nothing to choose, so the check runs at once with the answer the book allows.
    await within(root).findByTestId("preview-outcome");
    expect(within(root).queryByRole("radiogroup")).toBeNull();
    const counts = within(root).getByTestId("link-counts-as");
    expect(counts.getAttribute("data-answer")).toBe("taxable");
    expect(counts.textContent).toContain("A sale and a purchase");
    expect(counts.textContent).toContain("Your book's tax rules count a swap into another asset as a sale, so there is nothing to choose.");
    expect(calls("ui.transfers.pair_options").at(-1)?.args).toEqual({ tx_out: "send", tx_in: "near" });
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([expect.objectContaining({ policy: "taxable" })]);
    expect(confirmButton(root, "Link these two").disabled).toBe(false);
  });

  it("does not offer again an answer the pair had when the book no longer allows it", async () => {
    daemon.txs.near = tx("near", "Liquid", 4190000, { asset: "LBTC", paymentMethod: "Liquid", occurredAt: "2024-09-17T02:30:00Z" });
    daemon.pairOptions = [
      { policy: "carrying-value", allowed: false, reason: "network_mismatch", message: "A carrying-value pair cannot cross Bitcoin network boundaries" },
      { policy: "taxable", allowed: true },
    ];
    mount({ sendId: "send", receiptId: "near", relink: true, preset: { kind: "peg-in", policy: "carrying-value" } });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect(within(root).getByTestId("link-counts-as").textContent).toContain(
      "Kassiber's records don't agree on which Bitcoin network these two are on, so the cost basis can't carry over.",
    );
    expect(calls("ui.review.plan").at(-1)?.args.operations).toEqual([expect.objectContaining({ pair_kind: "peg-in", policy: "taxable" })]);
  });

  it("keeps both answers on offer when the core's answer is not the one case it has, and lets the check decide", async () => {
    daemon.txs.near = tx("near", "Exchange", 250000000000, { asset: "USDT", occurredAt: "2024-09-17T02:30:00Z" });
    daemon.pairOptions = [
      { policy: "carrying-value", allowed: false, reason: "cross_asset", message: "Not in this book." },
      { policy: "taxable", allowed: false, reason: "cross_asset", message: "Not either." },
    ];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    // Nothing of the core's own wording is shown; the owner is asked as before.
    expect(await within(root).findByRole("radiogroup", { name: "Was this a sale?" })).toBeTruthy();
    expect(root.textContent).not.toContain("Not in this book.");
    expect(within(root).getByTestId("link-unanswered").textContent).toBe("Answer the question above to see what changes.");
    expect(calls("ui.review.plan")).toHaveLength(0);
  });

  it("asks nothing of the core about a move within one asset", async () => {
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect(calls("ui.transfers.pair_options")).toHaveLength(0);
  });

  it("warns once, in one place, and asks to link anyway when the two don't look like one transfer", async () => {
    daemon.txs.near = tx("near", "Cold storage", 5000000, { occurredAt: "2024-09-15T02:14:00Z" });
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect(within(root).getByTestId("link-difference").textContent).toBe("799,000 sats more arrived than was sent.");
    const warnings = within(within(root).getByTestId("link-warnings")).getAllByRole("listitem");
    expect(warnings.map((item) => item.textContent)).toEqual([
      "More arrived than was sent. These are probably not the same coins.",
      "The receipt is dated before the send.",
      "Both sides are in the same wallet.",
    ]);
    expect(within(root).queryByRole("button", { name: "Link these two" })).toBeNull();
    expect(confirmButton(root, "Link anyway").disabled).toBe(false);
  });

  it("says how each tax year moves in a sentence", async () => {
    daemon.taxYears = [
      { year: 2024, realized_gain_loss: "0", income: "12.5" },
      { year: 2025, realized_gain_loss: "-806.03", income: "0", estimated_tax_eur_cents: -22166 },
    ];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    const sentences = within(root).getAllByTestId("preview-tax").map((item) => item.textContent ?? "");
    expect(sentences).toHaveLength(3);
    expect(sentences[0]).toMatch(/^2024: .*12[.,]50.* more income\.$/);
    expect(sentences[1]).toMatch(/^2025: .*806[.,]03.* less realized gain\.$/);
    expect(sentences[2]).toMatch(/^2025: about .*221[.,]66.* less tax\.$/);
    expect(within(root).queryByText(/Gain\/loss/)).toBeNull();
  });

  it("shows fiat for a priced side and says when one has no price", async () => {
    daemon.txs.send = tx("send", "Cold storage", -4200000, { eur: -2400 });
    useUiStore.setState({ currency: "eur" });
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("link-check");
    const legs = within(root).getAllByTestId("move-leg");
    await waitFor(() => expect(legs[0].textContent).toMatch(/2[.,]400/));
    expect(legs[1].textContent).toContain("Unpriced");
  });
});

describe("Link manually: when the core is slow, refuses or the book changes", () => {
  it("stays usable while the dry run runs, and joins the same run when reopened", async () => {
    let release = () => {};
    daemon.planGate = new Promise<void>((resolve) => {
      release = resolve;
    });
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await waitFor(() => expect(calls("ui.review.plan")).toHaveLength(1));
    expect(within(root).getByTestId("preview-checking").textContent).toContain("Recalculating your book with this change…");
    // Nothing can be linked on a run that has not finished, and the rest still works.
    expect(confirmButton(root, "Link these two").disabled).toBe(true);
    expect(within(root).getByTestId("link-difference").textContent).toContain("1,000 sats less");
    expect((within(root).getByRole("button", { name: "Choose another receipt" }) as HTMLButtonElement).disabled).toBe(false);
    expect((within(root).getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(false);
    // Closed mid-run and opened again on the same pair: no second recalculation.
    cleanup();
    mount({ sendId: "send", receiptId: "near" });
    const again = await dialog();
    await within(again).findByTestId("preview-checking");
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 700));
    });
    expect(calls("ui.review.plan")).toHaveLength(1);
    release();
    await within(again).findByTestId("preview-outcome");
    expect(calls("ui.review.plan")).toHaveLength(1);
    expect(confirmButton(again, "Link these two").disabled).toBe(false);
  });

  it("puts a refusal in plain words, keeps the core's own, and links nothing", async () => {
    daemon.planFails = [{ code: "conflict", message: "--tx-out already belongs to active pair id=p1." }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    const failed = await within(root).findByTestId("preview-failed");
    expect(failed.textContent).toContain("One of these two is already part of another link or review. Unlink that first.");
    expect(failed.textContent).toContain("--tx-out already belongs to active pair id=p1.");
    // Asking again would get the same answer.
    expect(within(failed).queryByRole("button", { name: "Try again" })).toBeNull();
    expect(confirmButton(root, "Link these two").disabled).toBe(true);
  });

  it("offers to try again when the dry run fails for another reason", async () => {
    daemon.planFails = [{ code: "internal_error", message: "database is busy" }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    const failed = await within(root).findByTestId("preview-failed");
    expect(failed.textContent).toContain("Kassiber can't preview this: database is busy");
    fireEvent.click(within(failed).getByRole("button", { name: "Try again" }));
    await within(root).findByTestId("preview-outcome");
    expect(calls("ui.review.plan")).toHaveLength(2);
  });

  it("runs the check again on its own when a newer check stopped it", async () => {
    daemon.planFails = [{ code: "review_plan_cancelled", message: "Review preview was cancelled" }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    expect(calls("ui.review.plan")).toHaveLength(2);
    expect(within(root).queryByTestId("preview-failed")).toBeNull();
  });

  it("stops trying after being stopped again and again, in plain words", async () => {
    daemon.planFails = Array.from({ length: 3 }, () => ({ code: "review_plan_cancelled", message: "Review preview was cancelled" }));
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    const failed = await within(root).findByTestId("preview-failed");
    expect(calls("ui.review.plan")).toHaveLength(3);
    expect(failed.textContent).not.toContain("cancelled");
    fireEvent.click(within(failed).getByRole("button", { name: "Try again" }));
    await within(root).findByTestId("preview-outcome");
  });

  it("checks again instead of linking when the book changed since the check", async () => {
    daemon.applyFails = [{ code: "review_plan_stale", message: "Review plan is stale" }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    daemon.version = 8;
    fireEvent.click(confirmButton(root, "Link these two"));
    expect((await within(root).findByRole("alert")).textContent).toBe("Your book changed since this check, so nothing was linked. Kassiber is checking again.");
    await waitFor(() => expect(calls("ui.review.plan")).toHaveLength(2));
    expect(calls("ui.review.plan").at(-1)?.args.expected_input_version).toBe(8);
    expect(onLinked).not.toHaveBeenCalled();
    await within(root).findByTestId("preview-outcome");
    fireEvent.click(confirmButton(root, "Link these two"));
    await waitFor(() => expect(onLinked).toHaveBeenCalledTimes(1));
  });

  it("plans again when the core calls the check stale though the book's version did not change", async () => {
    // A dismissal ran out since the check: the same version, the same link, a stale plan.
    daemon.applyFails = [{ code: "review_plan_stale", message: "Review plan is stale" }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    fireEvent.click(confirmButton(root, "Link these two"));
    expect((await within(root).findByRole("alert")).textContent).toBe("Your book changed since this check, so nothing was linked. Kassiber is checking again.");
    await waitFor(() => expect(calls("ui.review.plan")).toHaveLength(2));
    await within(root).findByTestId("preview-outcome");
    fireEvent.click(confirmButton(root, "Link these two"));
    await waitFor(() => expect(onLinked).toHaveBeenCalledTimes(1));
    // The second confirmation sent the new plan, not the one refused.
    const applied = calls("ui.review.apply").map((call) => (call.args.artifact as { digest: string }).digest);
    expect(applied).toEqual(["1".padStart(64, "0"), "2".padStart(64, "0")]);
  });

  it("keeps the dialog open with the reason when the core rejects the link itself", async () => {
    daemon.applyFails = [{ code: "transfer_network_mismatch", message: "A carrying-value pair cannot cross Bitcoin network boundaries (main -> regtest)." }];
    mount({ sendId: "send", receiptId: "near" });
    const root = await dialog();
    await within(root).findByTestId("preview-outcome");
    fireEvent.click(confirmButton(root, "Link these two"));
    const alert = await within(root).findByRole("alert");
    expect(alert.textContent).toContain("These two are on different Bitcoin networks, so they can't be one transfer.");
    expect(alert.textContent).toContain("(main -> regtest)");
    expect(onLinked).not.toHaveBeenCalled();
    expect(root.getAttribute("data-state")).toBe("check");
  });

  it("waits for the journals before it checks", async () => {
    const view = mount({ sendId: "send", receiptId: "near" }, true);
    const root = await dialog();
    expect((await within(root).findByTestId("preview-waiting")).textContent).toBe("Waiting for the journals to finish recalculating…");
    expect(confirmButton(root, "Link these two").disabled).toBe(true);
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 700));
    });
    expect(calls("ui.review.plan")).toHaveLength(0);
    view.setUpdating(false);
    await within(root).findByTestId("preview-outcome");
    expect(calls("ui.review.plan")).toHaveLength(1);
  });
});

describe("Link manually: in German and with details hidden", () => {
  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    daemon.txs.near = tx("near", "Liquid", 4190000, { asset: "LBTC", paymentMethod: "Liquid", occurredAt: "2024-09-17T02:30:00Z" });
    mount({ transactionId: "send" });
    const root = await dialog();
    const list = await rows(root);
    expect(within(within(root).getByRole("table", { name: "Eingänge" })).getAllByRole("columnheader").map((cell) => cell.textContent)).toEqual([
      "Wann",
      "Wallet",
      "Betrag",
      "Differenz",
      "Gebucht als",
      "In Transaktionen zeigen",
    ]);
    fireEvent.click(list[0]);
    expect(await within(root).findByRole("radiogroup", { name: "War das ein Verkauf?" })).toBeTruthy();
    expect(within(root).getByText("Beantworte die Frage oben, dann siehst du, was sich ändert.")).toBeTruthy();
    expect(confirmButton(root, "Diese beiden verknüpfen").disabled).toBe(true);
  });

  it("blurs wallets and amounts while sensitive details are hidden", async () => {
    useUiStore.setState({ hideSensitive: true });
    mount({ transactionId: "send" });
    const root = await dialog();
    const list = await rows(root);
    const cells = within(list[0]).getAllByRole("cell");
    expect(cells[1].querySelector(".sensitive")?.textContent).toBe("Spending");
    expect(cells[2].querySelector(".sensitive")?.textContent).toContain("0.04200000");
  });
});
