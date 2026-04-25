/**
 * Connection detail page — translated from claude-design/screens/connections.jsx
 * (the ConnectionDetail component, lines ~209-305).
 *
 * Reached via /connections/$connectionId. Header shows kind + label,
 * action row stubs sync / labels / edit / remove. Body splits into
 * top stat tiles, recent transactions, connection details (with KV
 * rows including a reveal-toggle for the xpub), and derived
 * addresses.
 *
 * Outstanding before this screen is feature-complete:
 *  - Wire Sync / Edit / Remove / Import-labels / Export-labels to
 *    real daemon kinds when those land
 *  - Replace synthesized derived-addresses list with a real per-
 *    descriptor lookup
 */

import { useState } from "react";
import {
  ArrowLeft,
  Copy,
  Check,
  Eye,
  EyeOff,
  RefreshCw,
  ArrowDownToLine,
  ArrowUpToLine,
} from "lucide-react";
import { Link, useParams } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import { KbCard } from "@/components/kb/KbCard";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";
import type { Connection, OverviewSnapshot } from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtBtc = (v: number) => v.toFixed(8);
const fmtEur = (v: number) =>
  "€ " +
  v.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
const fmtSatSigned = (n: number) =>
  (n > 0 ? "+ " : "− ") + Math.abs(n).toLocaleString("en-US");
const fmtEurSigned = (n: number) =>
  (n > 0 ? "+ €" : "− €") + Math.abs(n).toFixed(2);

const SYNTHETIC_ADDRESSES = [
  "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
  "bc1q9d4ywgfnd8h43da5tpcxcn6ajv590cg6d3tg6a",
  "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
  "bc1pgw9z80zvz6jcdqfp3hjlam77t34ddln0wfqp6w",
  "bc1qm34lsc65zpw79lxes69zkqmk6ee3ewf0j77s3h",
];

const FULL_XPUB =
  "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j8mdfKB3kRvvKUC7vw3R7Y8eYS9zPNxKr1J9";
const SHORT_XPUB = "xpub6C…aQL2j";

export function ConnectionDetail() {
  const { connectionId } = useParams({ from: "/_app/connections/$connectionId" });
  const { data, isLoading } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
  );
  const hideSensitive = useUiStore((s) => s.hideSensitive);

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center font-mono text-xs text-ink-3">
        loading…
      </div>
    );
  }

  const snapshot = data.data;
  const connection = snapshot.connections.find((c) => c.id === connectionId);

  if (!connection) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-4 p-12 text-ink-2">
        <div className="kb-mono-caption">connection · not found</div>
        <p className="m-0 max-w-md text-center font-sans text-sm">
          No connection with id <code className="font-mono">{connectionId}</code>{" "}
          exists in this workspace.
        </p>
        <Button asChild variant="secondary" size="sm" className="rounded-none">
          <Link to="/connections">
            <ArrowLeft className="size-3" /> Back to connections
          </Link>
        </Button>
      </div>
    );
  }

  return (
    <ConnectionDetailView
      connection={connection}
      priceEur={snapshot.priceEur}
      txs={snapshot.txs}
      hideSensitive={hideSensitive}
    />
  );
}

interface ConnectionDetailViewProps {
  connection: Connection;
  priceEur: number;
  txs: OverviewSnapshot["txs"];
  hideSensitive: boolean;
}

function ConnectionDetailView({
  connection,
  priceEur,
  txs,
  hideSensitive,
}: ConnectionDetailViewProps) {
  const txsForConnection = txs
    .filter((t) =>
      t.account
        .toLowerCase()
        .includes(connection.label.toLowerCase().split(" ")[0].toLowerCase()),
    )
    .slice(0, 5);
  const displayTxs = txsForConnection.length > 0 ? txsForConnection : txs.slice(0, 5);

  const isXpubLike =
    connection.kind === "xpub" || connection.kind === "descriptor";

  return (
    <div className="flex-1 overflow-auto bg-paper p-[18px]">
      <div className="mb-5 flex items-center gap-3.5">
        <Button
          asChild
          variant="secondary"
          size="icon-sm"
          className="rounded-none"
        >
          <Link to="/connections" aria-label="Back to connections">
            <ArrowLeft className="size-3" />
          </Link>
        </Button>
        <div className="flex size-10 flex-shrink-0 items-center justify-center border border-ink font-mono text-[14px] font-semibold text-ink">
          {connection.kind === "xpub" || connection.kind === "descriptor"
            ? "₿"
            : connection.kind === "core-ln" || connection.kind === "lnd"
              ? "⚡"
              : connection.kind === "cashu"
                ? "ₑ"
                : connection.kind === "nwc"
                  ? "N"
                  : "·"}
        </div>
        <div className="flex-1">
          <div className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
            {connection.kind} · Connection
          </div>
          <h2 className="m-0 font-sans text-[30px] font-semibold tracking-[-0.01em] text-ink">
            {connection.label}
          </h2>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" size="sm" className="rounded-none">
            <RefreshCw className="size-3" /> Sync
          </Button>
          <Button variant="secondary" size="sm" className="rounded-none">
            <ArrowDownToLine className="size-3" /> Import labels
          </Button>
          <Button variant="secondary" size="sm" className="rounded-none">
            <ArrowUpToLine className="size-3" /> Export labels
          </Button>
          <div className="mx-0.5 h-[22px] w-px self-center bg-line" />
          <Button variant="ghost" size="sm" className="rounded-none">
            Edit
          </Button>
          <Button variant="destructive" size="sm" className="rounded-none">
            Remove
          </Button>
        </div>
      </div>

      <div className="mb-4.5 grid grid-cols-4 gap-2.5">
        <StatTile
          label="Balance"
          value={
            <span className={blurClass(hideSensitive)}>
              {fmtBtc(connection.balance)} ₿
            </span>
          }
          sub={fmtEur(connection.balance * priceEur)}
        />
        <StatTile
          label="Addresses"
          value={connection.addresses ?? connection.channels ?? "—"}
          sub={connection.kind === "core-ln" ? "channels" : "derived"}
        />
        <StatTile label="Last sync" value={connection.last} sub={connection.status} />
        <StatTile
          label="Gap limit"
          value={connection.gap ?? "—"}
          sub="unused window"
        />
      </div>

      <div className="grid grid-cols-[1.4fr_1fr] gap-2.5">
        <KbCard title="Recent transactions" pad={false}>
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead>
              <tr className="border-b border-line">
                <th className="px-3.5 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                  Date
                </th>
                <th className="px-3.5 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                  Type
                </th>
                <th className="px-3.5 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                  sats
                </th>
                <th className="px-3.5 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                  €
                </th>
                <th className="px-3.5 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                  conf
                </th>
              </tr>
            </thead>
            <tbody>
              {displayTxs.map((tx) => (
                <tr key={tx.id} className="border-b border-line">
                  <td className="px-3.5 py-2.5 text-ink-2">
                    {tx.date.slice(5)}
                  </td>
                  <td className="px-3.5 py-2.5 text-ink-2">{tx.type}</td>
                  <td
                    className={cn(
                      "px-3.5 py-2.5 text-right tabular-nums",
                      tx.amountSat > 0 ? "text-[#3fa66a]" : "text-ink",
                      blurClass(hideSensitive),
                    )}
                  >
                    {fmtSatSigned(tx.amountSat)}
                  </td>
                  <td
                    className={cn(
                      "px-3.5 py-2.5 text-right tabular-nums",
                      blurClass(hideSensitive),
                    )}
                  >
                    {fmtEurSigned(tx.eur)}
                  </td>
                  <td className="px-3.5 py-2.5 text-right text-ink-3 tabular-nums">
                    {tx.conf}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </KbCard>

        <div className="flex flex-col gap-2.5">
          <KbCard title="Connection details">
            <div className="flex flex-col gap-2.5">
              <KvRow k="Label" v={connection.label} />
              <KvRow k="Type" v={connection.kind.toUpperCase()} mono />
              <KvRow
                k="Derivation path"
                v={connection.kind === "xpub" ? "m / 84' / 0' / 0'" : "—"}
                mono
              />
              {isXpubLike && (
                <>
                  <KvRow k="Fingerprint" v="5f3a8c0e" mono copy />
                  <KvReveal
                    k="Account xpub"
                    full={FULL_XPUB}
                    short={SHORT_XPUB}
                    hideSensitive={hideSensitive}
                  />
                </>
              )}
              <KvRow k="Backend" v="mempool.space" />
              <KvRow k="Created" v="2026-03-02 10:14" mono />
              <KvRow k="Kassiber ID" v={`conn_${connection.id}`} mono />
            </div>
          </KbCard>

          {isXpubLike && (
            <KbCard title="Derived addresses" pad={false}>
              <div className="max-h-[180px] overflow-auto">
                {SYNTHETIC_ADDRESSES.map((a, i) => (
                  <div
                    key={a}
                    className={cn(
                      "flex justify-between px-3.5 py-1.5 font-mono text-[10px]",
                      i > 0 && "border-t border-line",
                    )}
                  >
                    <span
                      className={cn("text-ink", blurClass(hideSensitive))}
                    >
                      {a.slice(0, 28)}…
                    </span>
                    <span className="text-ink-3">m/84&apos;/0&apos;/0&apos;/0/{i}</span>
                  </div>
                ))}
              </div>
            </KbCard>
          )}
        </div>
      </div>
    </div>
  );
}

interface StatTileProps {
  label: string;
  value: React.ReactNode;
  sub?: string;
}

function StatTile({ label, value, sub }: StatTileProps) {
  return (
    <div className="border border-line bg-paper-2 p-3.5">
      <div className="font-sans text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-3">
        {label}
      </div>
      <div className="mt-1.5 font-mono text-lg tracking-[-0.01em] text-ink">
        {value}
      </div>
      {sub && (
        <div className="mt-1 font-mono text-[10px] tracking-[0.05em] text-ink-3">
          {sub}
        </div>
      )}
    </div>
  );
}

interface KvRowProps {
  k: string;
  v: React.ReactNode;
  mono?: boolean;
  copy?: boolean;
}

function KvRow({ k, v, mono, copy }: KvRowProps) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    if (typeof v !== "string") return;
    try {
      await navigator.clipboard.writeText(v);
      setCopied(true);
      setTimeout(() => setCopied(false), 1100);
    } catch {
      // clipboard not available
    }
  };
  return (
    <div className="flex flex-col gap-0.5">
      <span className="font-sans text-[10px] font-medium uppercase tracking-[0.12em] text-ink-3">
        {k}
      </span>
      <div className="flex min-w-0 items-center gap-1.5">
        <span
          className={cn(
            "min-w-0 flex-1 truncate text-ink",
            mono ? "font-mono text-xs tracking-[-0.01em]" : "font-sans text-sm",
          )}
        >
          {v}
        </span>
        {copy && typeof v === "string" && (
          <button
            onClick={onCopy}
            title={copied ? "Copied" : "Copy"}
            className="flex size-5 shrink-0 cursor-pointer items-center justify-center border border-line bg-transparent"
          >
            {copied ? (
              <Check className="size-2.5 text-[#3fa66a]" />
            ) : (
              <Copy className="size-2.5 text-ink-2" />
            )}
          </button>
        )}
      </div>
    </div>
  );
}

interface KvRevealProps {
  k: string;
  full: string;
  short: string;
  hideSensitive: boolean;
}

function KvReveal({ k, full, short, hideSensitive }: KvRevealProps) {
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);
  const masked = !revealed || hideSensitive;
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(full);
      setCopied(true);
      setTimeout(() => setCopied(false), 1100);
    } catch {
      // clipboard not available
    }
  };
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="font-sans text-[10px] font-medium uppercase tracking-[0.12em] text-ink-3">
        {k}
      </span>
      <div className="flex min-w-0 items-center gap-1.5">
        <span
          className={cn(
            "min-w-0 flex-1 truncate font-mono text-xs tracking-[-0.01em] text-ink",
            masked && "sensitive",
          )}
        >
          {revealed && !hideSensitive ? full : short}
        </span>
        <button
          onClick={() => setRevealed((r) => !r)}
          title={revealed ? "Hide" : "Reveal"}
          className="flex size-5 shrink-0 cursor-pointer items-center justify-center border border-line bg-transparent"
        >
          {revealed ? (
            <EyeOff className="size-2.5 text-ink-2" />
          ) : (
            <Eye className="size-2.5 text-ink-2" />
          )}
        </button>
        <button
          onClick={onCopy}
          title={copied ? "Copied" : "Copy"}
          className="flex size-5 shrink-0 cursor-pointer items-center justify-center border border-line bg-transparent"
        >
          {copied ? (
            <Check className="size-2.5 text-[#3fa66a]" />
          ) : (
            <Copy className="size-2.5 text-ink-2" />
          )}
        </button>
      </div>
    </div>
  );
}
