/**
 * Transactions screen — full ledger list, searchable + type-filterable.
 *
 * Visual translation of claude-design/screens/transactions.jsx. Hand-rolled
 * table (12-20 rows of mock data — virtualization overkill until the cursor
 * paginated daemon kind lands; TanStack Table can slot in then).
 *
 * Inline styles become Tailwind classes against the theme tokens. The
 * "More" dropdown for secondary tx types (Swap/Mint/Melt/etc) closes on
 * outside click, matching the source.
 *
 * Outstanding before this screen is feature-complete:
 *  - Real `ledger.list` daemon kind (cursor pagination) instead of the
 *    static MOCK_TRANSACTIONS fixture
 *  - Wire Import / Export label buttons to the BIP329 commands
 *  - CSV / JSON exports (currently no-op)
 *  - Manual entry modal — deferred until the journal-entry contract lands
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowDownToLine, ArrowUpToLine, ChevronDown, Plus, Search } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { useCurrency, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { Tx, TxType } from "@/mocks/seed";
import type { TransactionsLedger } from "@/mocks/transactions";

const PRIMARY_TYPES = ["all", "Income", "Expense", "Transfer"] as const;
const SECONDARY_TYPES: TxType[] = [
  "Swap",
  "Consolidation",
  "Rebalance",
  "Mint",
  "Melt",
  "Fee",
];

type Filter = "all" | TxType;

const TX_TYPE_HEX: Record<TxType, string> = {
  Income: "#3fa66a",
  Expense: "var(--color-accent)",
  Transfer: "#6b7280",
  Swap: "#8b6f3c",
  Consolidation: "#5d6b7a",
  Rebalance: "#7d6b8a",
  Mint: "#3f7aa6",
  Melt: "#a66a3f",
  Fee: "var(--color-ink-3)",
};

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtSat = (n: number) =>
  (n > 0 ? "+ " : "− ") + Math.abs(n).toLocaleString("en-US");

const fmtEurSigned = (n: number) =>
  (n > 0 ? "+ €" : "− €") +
  Math.abs(n).toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

const fmtRate = (n: number) =>
  "€ " +
  n.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

export function Transactions() {
  const { data, isLoading } = useDaemon<TransactionsLedger>("ui.transactions.list");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();

  const [q, setQ] = useState("");
  const [typeFilter, setTypeFilter] = useState<Filter>("all");
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement | null>(null);

  const secondaryActive = SECONDARY_TYPES.includes(typeFilter as TxType);

  useEffect(() => {
    if (!moreOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (moreRef.current && !moreRef.current.contains(e.target as Node)) {
        setMoreOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [moreOpen]);

  const ledger = data?.data;
  const year = ledger?.year ?? new Date().getFullYear();

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (ledger?.txs ?? []).filter((tx) => {
      if (typeFilter !== "all" && tx.type !== typeFilter) return false;
      if (
        needle &&
        !`${tx.counter} ${tx.account} ${tx.tag}`.toLowerCase().includes(needle)
      ) {
        return false;
      }
      return true;
    });
  }, [ledger, q, typeFilter]);

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center font-mono text-xs text-ink-3">
        loading…
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-auto bg-paper p-[18px]">
      <div className="mb-[18px] flex items-end justify-between">
        <div>
          <div className="kb-mono-caption">
            Ledger · {filtered.length} entries · {year}
          </div>
          <h2 className="m-0 mt-1 font-sans text-[32px] font-semibold tracking-[-0.01em] text-ink">
            Transactions
          </h2>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="secondary" size="sm" className="rounded-none">
            <ArrowDownToLine className="size-3" />
            Import labels
          </Button>
          <Button variant="secondary" size="sm" className="rounded-none">
            <ArrowUpToLine className="size-3" />
            Export labels
          </Button>
          <div className="mx-0.5 h-[22px] w-px self-center bg-line" />
          <Button variant="secondary" size="sm" className="rounded-none">
            <ArrowDownToLine className="size-3" />
            CSV
          </Button>
          <Button variant="secondary" size="sm" className="rounded-none">
            <ArrowDownToLine className="size-3" />
            JSON
          </Button>
          <Button size="sm" className="rounded-none">
            <Plus className="size-3" />
            Manual entry
          </Button>
        </div>
      </div>

      {/* filter strip */}
      <div className="mb-2.5 flex items-center gap-3 border border-line bg-paper-2 px-3 py-2.5">
        <div className="flex flex-1 items-center gap-1.5">
          <Search className="size-3 text-ink-3" strokeWidth={1.5} />
          <input
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search counterparty, tag, account…"
            className="flex-1 border-none bg-transparent font-sans text-xs text-ink outline-none placeholder:text-ink-3"
          />
        </div>
        <div className="h-5 w-px bg-line" />
        <div className="flex items-center gap-1">
          {PRIMARY_TYPES.map((t) => {
            const active = typeFilter === t;
            return (
              <TxFilterPill
                key={t}
                active={active}
                onClick={() => setTypeFilter(t)}
              >
                {t}
              </TxFilterPill>
            );
          })}
          <div ref={moreRef} className="relative">
            <TxFilterPill
              active={secondaryActive}
              onClick={() => setMoreOpen((o) => !o)}
            >
              <span className="flex items-center gap-1">
                {secondaryActive ? typeFilter : "More"}
                <ChevronDown className="size-3 opacity-60" />
              </span>
            </TxFilterPill>
            {moreOpen && (
              <div className="absolute right-0 top-8 z-40 flex min-w-[160px] flex-col gap-0.5 border border-ink bg-paper p-1.5 shadow-[4px_4px_0_var(--color-ink)]">
                <div className="px-2 pb-0.5 pt-1 font-mono text-[9px] uppercase tracking-[0.12em] text-ink-3">
                  Advanced types
                </div>
                {SECONDARY_TYPES.map((t) => {
                  const active = typeFilter === t;
                  return (
                    <button
                      key={t}
                      type="button"
                      onClick={() => {
                        setTypeFilter(t);
                        setMoreOpen(false);
                      }}
                      className={cn(
                        "flex cursor-pointer items-center justify-between border-none px-2 py-1 text-left font-sans text-xs",
                        active
                          ? "bg-ink text-paper"
                          : "bg-transparent text-ink",
                      )}
                    >
                      {t}
                    </button>
                  );
                })}
                {secondaryActive && (
                  <>
                    <div className="my-1 h-px bg-line" />
                    <button
                      type="button"
                      onClick={() => {
                        setTypeFilter("all");
                        setMoreOpen(false);
                      }}
                      className="cursor-pointer border-none bg-transparent px-2 py-1 text-left font-mono text-[10px] uppercase tracking-[0.08em] text-ink-3"
                    >
                      Clear filter
                    </button>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* table */}
      <div className="border border-line bg-paper-2">
        <table className="w-full border-collapse">
          <thead>
            <tr className="border-b border-ink bg-paper">
              <Th>Date · time</Th>
              <Th>Type</Th>
              <Th>Account</Th>
              <Th>Counterparty</Th>
              <Th>Tag</Th>
              <Th align="right">Sats</Th>
              <Th align="right">BTC/EUR rate</Th>
              <Th align="right">EUR</Th>
              <Th align="right">Conf</Th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((tx) => (
              <TxRow
                key={tx.id}
                tx={tx}
                hideSensitive={hideSensitive}
                currency={currency}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

interface TxFilterPillProps {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}

/** Local filter pill — small, hard-edge, paper or ink fill. */
function TxFilterPill({ active, onClick, children }: TxFilterPillProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex cursor-pointer items-center border px-2.5 py-1 font-mono text-[10px] uppercase tracking-[0.1em]",
        active
          ? "border-ink bg-ink text-paper"
          : "border-line bg-transparent text-ink-3 hover:text-ink",
      )}
    >
      {children}
    </button>
  );
}

interface ThProps {
  align?: "left" | "right";
  children: React.ReactNode;
}

function Th({ align = "left", children }: ThProps) {
  return (
    <th
      className={cn(
        "px-3.5 py-2.5 font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3",
        align === "right" ? "text-right" : "text-left",
      )}
    >
      {children}
    </th>
  );
}

interface TxRowProps {
  tx: Tx;
  hideSensitive: boolean;
  currency: Currency;
}

function TxRow({ tx, hideSensitive, currency }: TxRowProps) {
  // Type chip needs a dynamic border + text colour that varies by tx type;
  // there isn't a single Tailwind utility per arbitrary type so we set the
  // colour via a CSS variable rather than using inline styles for layout.
  const typeColor = TX_TYPE_HEX[tx.type];
  const isEur = currency === "eur";

  // The sats column is technical and reads better as the canonical ledger
  // amount; we keep it visible regardless of the toggle but shift emphasis
  // (text-ink vs text-ink-2) so the currency-matching column reads as the
  // primary value.
  const satsClass = cn(
    "px-3.5 py-2.5 text-right font-mono text-[11px]",
    tx.amountSat > 0
      ? "text-[#3fa66a]"
      : isEur
        ? "text-ink-2"
        : "text-ink",
    blurClass(hideSensitive),
  );
  const eurClass = cn(
    "px-3.5 py-2.5 text-right font-mono text-[11px]",
    isEur ? "text-ink" : "text-ink-2",
    blurClass(hideSensitive),
  );

  return (
    <tr className="border-b border-line">
      <td className="px-3.5 py-2.5 font-mono text-[11px] text-ink-2">
        {tx.date}
      </td>
      <td className="px-3.5 py-2.5">
        <span
          className="border px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-[0.1em]"
          style={{ borderColor: typeColor, color: typeColor }}
        >
          {tx.type}
        </span>
      </td>
      <td className="px-3.5 py-2.5 font-sans text-xs text-ink">
        {tx.account}
      </td>
      <td className="px-3.5 py-2.5 font-sans text-xs text-ink">
        {tx.counter}
      </td>
      <td className="px-3.5 py-2.5">
        <span className="border border-line bg-paper px-1.5 py-0.5 font-mono text-[10px] tracking-[0.04em] text-ink-2">
          {tx.tag}
        </span>
      </td>
      <td className={satsClass}>{fmtSat(tx.amountSat)}</td>
      <td className="px-3.5 py-2.5 text-right font-mono text-[11px] text-ink-3">
        {fmtRate(tx.rate)}
      </td>
      <td className={eurClass}>{fmtEurSigned(tx.eur)}</td>
      <td className="px-3.5 py-2.5 text-right font-mono text-[11px] text-ink-3">
        {tx.conf}
      </td>
    </tr>
  );
}
