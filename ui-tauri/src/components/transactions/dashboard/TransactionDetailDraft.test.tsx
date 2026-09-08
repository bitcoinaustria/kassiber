import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TransactionDetailController } from "./TransactionDetailController";
import { useOverviewTransactionDetail } from "@/components/overview-dashboard/useOverviewTransactionDetail";
import { TransactionClassifyTab, type TransactionDetailTabContext } from "../TransactionDetailSheetTabs";
import { draftForTransaction } from "../model";
import { TransactionDetailSheet } from "../TransactionDetailSheet";
import { toDashboardTransaction } from "./model";
import { DEFAULT_EXPLORER_SETTINGS } from "@/lib/explorer";
import type { OverviewSnapshot, Tx } from "@/mocks/seed";
import type { TransactionEditDraft } from "../model";

const hooks = vi.hoisted(() => ({
  slots: [] as unknown[], cursor: 0, effects: [] as Array<() => void>,
  query: {} as Record<string, unknown>, invoke: vi.fn(), resolve: {} as unknown,
}));
// Replay real callers across query completion, selection and save. The scheduler
// preserves state/memo dependency identity; all daemon responses are synthetic.
vi.mock("react", async original => {
  const react = await original<typeof import("react")>();
  const memo = (factory: () => unknown, deps: unknown[]) => {
    const index = hooks.cursor++;
    const prior = hooks.slots[index] as { deps: unknown[]; value: unknown } | undefined;
    if (!prior || deps.some((dep, i) => !Object.is(dep, prior.deps[i]))) hooks.slots[index] = { deps, value: factory() };
    return (hooks.slots[index] as { value: unknown }).value;
  };
  return { ...react,
    useState: (initial: unknown) => {
      const index = hooks.cursor++;
      if (!(index in hooks.slots)) hooks.slots[index] = typeof initial === "function" ? initial() : initial;
      return [hooks.slots[index], (next: unknown) => { hooks.slots[index] = typeof next === "function" ? next(hooks.slots[index]) : next; }];
    },
    useMemo: memo,
    useCallback: (callback: unknown, deps: unknown[]) => memo(() => callback, deps),
    useRef: (initial: unknown) => memo(() => ({ current: initial }), []),
    useEffect: (effect: () => void, deps: unknown[]) => memo(() => { hooks.effects.push(effect); }, deps),
  };
});
vi.mock("@tanstack/react-query", () => ({ useQueryClient: () => ({ invalidateQueries: vi.fn() }) }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/hooks/useJournalProcessingAction", () => ({ useJournalProcessingAction: () => ({}) }));
vi.mock("@/daemon/client", () => ({
  useDaemon: () => hooks.query,
  useDaemonMutation: (kind: string) => ({
    mutateAsync: (args: unknown) => hooks.invoke(kind, args),
    mutate: (_args: unknown, options: { onSuccess: (result: unknown) => void }) => options.onSuccess({ data: { transaction: hooks.resolve } }),
  }),
}));
type DetailProps = { draft: TransactionEditDraft; onSave: (id: string, draft: TransactionEditDraft) => Promise<void> };
function find(node: ReactNode): DetailProps | undefined {
  if (Array.isArray(node)) return node.map(find).find(Boolean);
  if (!isValidElement<{ children?: ReactNode }>(node)) return;
  if (node.type === TransactionDetailSheet) return node.props as unknown as DetailProps;
  return find(node.props.children);
}
function sheet(node: ReactNode): DetailProps { const found = find(node); if (!found) throw new Error("Detail sheet missing"); return found; }
function render<T>(callback: () => T): T { hooks.cursor = 0; return callback(); }
function effects() { hooks.effects.splice(0).forEach(effect => effect()); }
const raw: Tx = { id: "incoming", date: "2026-04-15 08:00", type: "Transfer", account: "Synthetic wallet", counter: "Transfer BTC -> BTC", amountSat: 600_000, eur: null, rate: null, tag: "Transfer", conf: 3, feeSat: 0 };
const transaction = toDashboardTransaction(raw, 0);
const common = { hideSensitive: false, currency: "eur" as const, explorerSettings: DEFAULT_EXPLORER_SETTINGS };
beforeEach(() => {
  hooks.slots = []; hooks.cursor = 0; hooks.effects = []; hooks.query = {};
  hooks.invoke.mockReset().mockResolvedValue({ data: {} }); hooks.resolve = raw;
});
afterEach(() => vi.unstubAllGlobals());
describe("detail draft identity and exact save baseline", () => {
  it("preserves typed edits when an equal metadata snapshot or display currency arrives", () => {
    vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn() });
    let draft = draftForTransaction(transaction);
    let currency: "eur" | "btc" = "eur";
    const draw = () => {
      const frame = TransactionDetailSheet({ ...common, transaction, draft, currency,
        initialTab: "classify", onOpenChange: vi.fn(), onOpenExplorer: vi.fn(), onSave: vi.fn() });
      const body = frame.props.children;
      if (!isValidElement(body) || typeof body.type !== "function") throw new Error("Missing detail body");
      const tree = render(() => (body.type as (props: unknown) => ReactNode)(body.props));
      const context = (node: ReactNode): TransactionDetailTabContext | undefined => {
        if (Array.isArray(node)) return node.map(context).find(Boolean);
        if (!isValidElement<{ children?: ReactNode; ctx?: TransactionDetailTabContext }>(node)) return;
        if (node.type === TransactionClassifyTab) return node.props.ctx;
        return context(node.props.children);
      };
      const ctx = context(tree);
      if (!ctx) throw new Error("Missing actual classify context");
      return ctx;
    };
    draw(); effects();
    draw().updateDraft("note", "Unsaved note");
    draft = { ...draft, tags: [...draft.tags] };
    draw(); effects();
    expect(draw().localDraft.note).toBe("Unsaved note");
    currency = "btc";
    draw(); effects();
    expect(draw().localDraft.note).toBe("Unsaved note");
    draft = { ...draft, note: "Saved note" };
    draw(); effects();
    expect(draw().localDraft.note).toBe("Saved note");
  });
  it("keeps the SourceFunds draft across unrelated detail reads, but resets for a new selection", () => {
    let selected = transaction;
    const draw = () => sheet(render(() => TransactionDetailController({ ...common, transaction: selected, onOpenChange: vi.fn() })));
    const initial = draw().draft; effects();
    hooks.query = { data: { data: { items: [] } } };
    expect(draw().draft).toBe(initial);
    selected = { ...transaction, id: "next", note: "Next note" };
    draw(); effects();
    expect(draw().draft).not.toBe(initial);
    expect(draw().draft.note).toBe("Next note");
  });
  it("keeps the Overview draft across query arrivals and clears an override against the resolved row", async () => {
    const snapshot = { txs: [], activityTxs: [{ ...raw, summaryOnly: true }], fiat: { fiatCurrency: "EUR" }, priceEur: 100_000 } as unknown as OverviewSnapshot;
    hooks.resolve = { ...raw, kindOverride: "buy" };
    const draw = () => render(() => useOverviewTransactionDetail({ ...common, snapshot }));
    draw().openTransactionDetail(raw.id);
    const initial = sheet(draw().detailSheet).draft;
    expect(initial.kind).toBe("buy");
    hooks.query = { data: { data: { items: [] } } };
    expect(sheet(draw().detailSheet).draft).toBe(initial);
    const cleared = { ...initial, kind: null };
    await sheet(draw().detailSheet).onSave(raw.id, cleared);
    expect(hooks.invoke).toHaveBeenLastCalledWith("ui.transactions.metadata.update", expect.objectContaining({ transaction: raw.id, kind: null }));
    expect(sheet(draw().detailSheet).draft).toBe(cleared);
    await sheet(draw().detailSheet).onSave(raw.id, cleared);
    expect(hooks.invoke.mock.lastCall?.[1]).not.toHaveProperty("kind");
  });
});
