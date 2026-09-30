import type { PropsWithChildren } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { TransactionDetailSheet } from "./TransactionDetailSheet";
import { DEFAULT_EXPLORER_SETTINGS } from "@/lib/explorer";
import { draftForTransaction } from "./model";
import { toDashboardTransaction } from "./dashboard/model";

vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({ isLoading: true }),
  useDaemonMutation: () => ({ isPending: false, mutateAsync: async () => ({}) }),
}));
vi.mock("@/components/ui/sheet", () => ({
  // Model Radix exit presence: closed content can remain mounted.
  Sheet: ({ children }: PropsWithChildren<{ open: boolean }>) => <div data-frame="sheet">{children}</div>,
  SheetContent: ({ children, className }: PropsWithChildren<{ className: string }>) => <section data-slot="sheet-content" className={className}>{children}</section>,
  SheetHeader: ({ children }: PropsWithChildren) => <header>{children}</header>,
  SheetFooter: ({ children }: PropsWithChildren) => <footer>{children}</footer>,
  SheetTitle: ({ children }: PropsWithChildren) => <h2>{children}</h2>,
  SheetDescription: ({ children }: PropsWithChildren) => <p>{children}</p>,
}));
const transaction = toDashboardTransaction({ id: "incoming", date: "2026-04-15 08:00", type: "Transfer", account: "Synthetic wallet", counter: "Transfer BTC -> BTC", amountSat: 600_000, eur: null, rate: null, tag: "Transfer", conf: 3, feeSat: 0 }, 0);
const props = {
  transaction: null, draft: null, open: true, isLoading: true,
  initialTab: "details", hideSensitive: false, currency: "eur" as const,
  explorerSettings: DEFAULT_EXPLORER_SETTINGS,
  onOpenChange: vi.fn(), onOpenExplorer: vi.fn(), onSave: vi.fn(), onRetry: vi.fn(),
};

describe("transaction detail opening surface", () => {
  it("opens the final sheet geometry immediately while resolving the exact row", () => {
    const html = renderToStaticMarkup(<TransactionDetailSheet {...props} />);
    expect(html).toContain('data-slot="sheet-content"');
    expect(html).toContain('w-[min(100vw,1120px)]');
    expect(html).toContain('role="status"');
    expect(html).not.toContain('data-slot="dialog-content"');
    expect(html).not.toContain("Synthetic wallet");
  });
  it("keeps loading and retry inside the same sheet instead of a centered dialog", () => {
    const html = renderToStaticMarkup(<TransactionDetailSheet {...props} isLoading={false} />);
    expect(html).toContain('data-slot="sheet-content"');
    expect(html).toContain("Retry");
    expect(html).not.toContain('data-slot="dialog-content"');
  });
  it("uses the same frame when the exact record arrives before its graph", () => {
    const html = renderToStaticMarkup(<TransactionDetailSheet {...props} transaction={transaction} draft={draftForTransaction(transaction)} />);
    expect(html.match(/data-slot="sheet-content"/g)).toHaveLength(1);
    expect(html).toContain('w-[min(100vw,1120px)]');
    expect(html).toContain("Synthetic wallet");
    expect(html).not.toContain('data-slot="dialog-content"');
  });
  it("explains a custody hold with the daemon's blocker instead of the bare reason", () => {
    const loaded = { ...props, transaction, draft: draftForTransaction(transaction), isLoading: false };
    const reason = "custody_quantity_unresolved";
    const generic = renderToStaticMarkup(<TransactionDetailSheet {...loaded} quarantineReasonOverride={reason} />);
    const html = renderToStaticMarkup(
      <TransactionDetailSheet
        {...loaded}
        quarantineReasonOverride={reason}
        quarantineContext={{ reason, category: "needs_decision", evidence: { blocker_code: "reviewed_residual_suspense" }, rootLabel: null }}
      />,
    );
    expect(generic).not.toContain("A reviewed route keeps part in suspense");
    expect(html).toContain("A reviewed route keeps part in suspense");
  });
  it("masks the amounts the explanation quotes when values are hidden", () => {
    const loaded = { ...props, transaction, draft: draftForTransaction(transaction), isLoading: false };
    const reason = "insufficient_lots";
    const context = {
      reason,
      category: "missing_acquisition_history" as const,
      evidence: { required_msat: 123_456_789_000, available_msat: 87_654_321_000 },
      rootLabel: null,
    };
    const quoting = /class="[^"]*\bsensitive\b[^"]*">[^<]*123,456,789 sats/;
    const shown = renderToStaticMarkup(
      <TransactionDetailSheet {...loaded} quarantineReasonOverride={reason} quarantineContext={context} />,
    );
    const hidden = renderToStaticMarkup(
      <TransactionDetailSheet {...loaded} hideSensitive quarantineReasonOverride={reason} quarantineContext={context} />,
    );
    expect(shown).toContain("123,456,789 sats");
    expect(shown).not.toMatch(quoting);
    expect(hidden).toMatch(quoting);
  });
  it("does not display a pending sheet after closing", () => {
    expect(renderToStaticMarkup(<TransactionDetailSheet {...props} open={false} />)).toBe("");
  });
});
