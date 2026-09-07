import type { PropsWithChildren } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { TransactionDetailSheet } from "./TransactionDetailSheet";
import { DEFAULT_EXPLORER_SETTINGS } from "@/lib/explorer";
import { draftForTransaction } from "./model";
import { toDashboardTransaction } from "./dashboard/model";

vi.mock("@/daemon/client", () => ({ useDaemon: () => ({ isLoading: true }) }));
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
  it("does not display a pending sheet after closing", () => {
    expect(renderToStaticMarkup(<TransactionDetailSheet {...props} open={false} />)).toBe("");
  });
});
