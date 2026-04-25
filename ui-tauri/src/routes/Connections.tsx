/**
 * Connections list view.
 *
 * Visual translation of the connections list from claude-design — a
 * richer table-style view than the Overview's ConnectionsCard, with
 * full sync metadata, address counts, and a per-row composition bar.
 *
 * Per-row click navigates to /connections/$connectionId. Add connection
 * opens the AddConnectionFlow (picker → per-kind form).
 *
 * Outstanding before this screen is feature-complete:
 *  - Per-kind add forms beyond xpub (descriptor, lightning, exchange,
 *    cashu, btcpay, csv) — each lands when its claude-design source
 *    materializes
 *  - Bulk sync / per-row sync actions wiring to the daemon
 */

import { useState } from "react";
import { Plus, RefreshCw } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { useCurrency, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import { SyncDot } from "@/components/kb/SyncDot";
import { ProtocolChip } from "@/components/kb/ProtocolChip";
import { AddConnectionFlow } from "@/components/kb/AddConnectionFlow";
import type { Connection, OverviewSnapshot } from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtBtc = (v: number) => v.toFixed(8);
const fmtEur = (v: number) =>
  "€ " +
  v.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

export function Connections() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const navigate = useNavigate();
  const [spinning, setSpinning] = useState(false);
  const [addOpen, setAddOpen] = useState(false);

  const onSyncAll = () => {
    setSpinning(true);
    setTimeout(() => setSpinning(false), 900);
  };

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center font-mono text-xs text-ink-3">
        loading…
      </div>
    );
  }

  const snapshot = data.data;
  const totalBtc = snapshot.connections.reduce((s, c) => s + c.balance, 0);

  const errorN = snapshot.connections.filter((c) => c.status === "error").length;
  const syncingN = snapshot.connections.filter((c) => c.status === "syncing").length;

  const onSelect = (id: string) =>
    void navigate({
      to: "/connections/$connectionId",
      params: { connectionId: id },
    });

  return (
    <div className="flex-1 overflow-auto bg-paper p-[18px]">
      <div className="mb-[18px] flex items-end justify-between">
        <div>
          <div className="kb-mono-caption">
            {snapshot.connections.length} connections · {errorN > 0 ? `${errorN} need attention · ` : ""}
            {syncingN > 0 ? `${syncingN} syncing` : "all synced"}
          </div>
          <h2 className="m-0 mt-1 font-sans text-[32px] font-semibold tracking-[-0.01em] text-ink">
            Connections
          </h2>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            className="rounded-none"
            onClick={onSyncAll}
          >
            <RefreshCw
              className={cn("size-3", spinning && "animate-spin")}
            />
            Sync all
          </Button>
          <Button
            size="sm"
            className="rounded-none"
            onClick={() => setAddOpen(true)}
          >
            <Plus className="size-3" />
            Add connection
          </Button>
        </div>
      </div>

      <div className="border border-line bg-paper-2">
        <div className="grid grid-cols-[20px_1.4fr_120px_120px_1fr_140px] items-center gap-x-3 border-b border-line bg-paper px-3 py-2 font-mono text-[9px] font-semibold uppercase tracking-[0.14em] text-ink-3">
          <span />
          <span>Connection</span>
          <span>Kind</span>
          <span>Last sync</span>
          <span>Composition</span>
          <span className="text-right">Balance</span>
        </div>

        {snapshot.connections.map((c, i) => (
          <ConnectionRow
            key={c.id}
            connection={c}
            totalBtc={totalBtc}
            priceEur={snapshot.priceEur}
            hideSensitive={hideSensitive}
            currency={currency}
            isLast={i === snapshot.connections.length - 1}
            onSelect={() => onSelect(c.id)}
          />
        ))}
      </div>

      <AddConnectionFlow open={addOpen} onClose={() => setAddOpen(false)} />
    </div>
  );
}

interface ConnectionRowProps {
  connection: Connection;
  totalBtc: number;
  priceEur: number;
  hideSensitive: boolean;
  currency: Currency;
  isLast: boolean;
  onSelect: () => void;
}

function ConnectionRow({
  connection: c,
  totalBtc,
  priceEur,
  hideSensitive,
  currency,
  isLast,
  onSelect,
}: ConnectionRowProps) {
  const sats = Math.round(c.balance * 1e8);
  const pct = totalBtc > 0 ? (c.balance / totalBtc) * 100 : 0;
  const isEur = currency === "eur";

  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "grid w-full cursor-pointer grid-cols-[20px_1.4fr_120px_120px_1fr_140px] items-center gap-x-3 border-0 bg-transparent px-3 py-3 text-left transition-colors hover:bg-paper",
        !isLast && "border-b border-line",
      )}
    >
      <SyncDot status={c.status} />

      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="overflow-hidden truncate font-sans text-[14px] font-semibold tracking-[-0.005em] text-ink">
          {c.label}
        </span>
        <span className="font-mono text-[10px] uppercase tracking-[0.04em] text-ink-3">
          {c.last}
          {c.addresses != null && (
            <>
              {" · "}
              <span>{c.addresses} addresses</span>
            </>
          )}
          {c.channels != null && (
            <>
              {" · "}
              <span>{c.channels} channels</span>
            </>
          )}
          {c.gap != null && (
            <>
              {" · gap "}
              <span>{c.gap}</span>
            </>
          )}
        </span>
      </div>

      <div>
        <ProtocolChip kind={c.kind} />
      </div>

      <span className="font-mono text-[11px] uppercase tracking-[0.04em] text-ink-2">
        {c.status}
      </span>

      <div className="flex items-center gap-2">
        <div className="relative h-1 flex-1 max-w-[180px] bg-line">
          <div
            className={cn(
              "absolute inset-y-0 left-0 bg-ink transition-[width] duration-200",
              blurClass(hideSensitive),
            )}
            style={{ width: `${Math.max(1.5, pct)}%` }}
          />
        </div>
        <span
          className={cn(
            "min-w-[40px] font-mono text-[11px] tabular-nums text-ink-3",
            blurClass(hideSensitive),
          )}
        >
          {pct < 0.1 ? "<0.1%" : pct.toFixed(pct < 10 ? 1 : 0) + "%"}
        </span>
      </div>

      <div className="text-right">
        <div
          className={cn(
            "font-sans text-[14px] font-medium tabular-nums text-ink",
            blurClass(hideSensitive),
          )}
        >
          {isEur
            ? fmtEur(c.balance * priceEur)
            : "₿ " + fmtBtc(c.balance)}
        </div>
        <div
          className={cn(
            "font-mono text-[10px] tabular-nums text-ink-3",
            blurClass(hideSensitive),
          )}
        >
          {isEur
            ? "₿ " + fmtBtc(c.balance) + " · " + sats.toLocaleString("en-US") + " sat"
            : sats.toLocaleString("en-US") + " sat · " + fmtEur(c.balance * priceEur)}
        </div>
      </div>
    </button>
  );
}
