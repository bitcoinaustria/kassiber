/**
 * Reports — capital-gains export flow.
 *
 * Visual translation of claude-design/screens/tax.jsx. The screen is
 * named "Reports" in the kassiber app because /reports surfaces multiple
 * report types (capital gains, journal entries, balance sheet, the
 * Austrian E 1kv preview); only capital gains is wired here today.
 *
 * Inline styles from the source become Tailwind classes against the
 * theme tokens in styles/globals.css. The fixture lives in
 * mocks/reports.ts and is served via the mock daemon under
 * `ui.reports.capital_gains` so the screen exercises the same loading
 * shape as Overview.
 *
 * Deferred:
 *  - Wiring the cost-basis selection / policy toggles to a real engine
 *  - Live re-computation when From/To dates are edited (the inputs are
 *    visible but only echo the selected year)
 *  - Actual export click handlers (CSV/PDF/JSON buttons are display-only)
 *  - "Back" navigation — the AppShell header now governs routing
 */

import { useState, type ReactNode } from "react";
import { ArrowRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { LabeledInput } from "@/components/kb/LabeledInput";
import { KbCard } from "@/components/kb/KbCard";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";
import {
  JURISDICTIONS,
  type CapitalGainsReport,
  type CostBasisMethod,
  type DisposedLot,
} from "@/mocks/reports";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const COST_BASIS_METHODS: Array<{
  k: CostBasisMethod;
  name: string;
  desc: string;
}> = [
  { k: "fifo", name: "FIFO", desc: "First-in, first-out · common default" },
  { k: "lifo", name: "LIFO", desc: "Last-in, first-out" },
  { k: "hifo", name: "HIFO", desc: "Highest-in, first-out (tax optimization)" },
  { k: "spec", name: "Specific ID", desc: "Per-lot selection" },
];

const REPORTING_YEARS = [2023, 2024, 2025, 2026] as const;

export function Reports() {
  const { data, isLoading } = useDaemon<CapitalGainsReport>(
    "ui.reports.capital_gains",
  );
  const hideSensitive = useUiStore((s) => s.hideSensitive);

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center font-mono text-xs text-ink-3">
        loading…
      </div>
    );
  }

  return (
    <ReportsView report={data.data} hideSensitive={hideSensitive} />
  );
}

interface ReportsViewProps {
  report: CapitalGainsReport;
  hideSensitive: boolean;
}

function ReportsView({ report, hideSensitive }: ReportsViewProps) {
  const [year, setYear] = useState(report.year);
  const [jurCode, setJurCode] = useState(report.jurisdictionCode);
  const j = JURISDICTIONS[jurCode] ?? JURISDICTIONS.AT;
  const [method, setMethod] = useState<CostBasisMethod>(j.defaultMethod);
  const [step, setStep] = useState<1 | 2>(1);

  const fmt = (n: number) =>
    n.toLocaleString(j.locale, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });

  const lots = report.lots;
  const totals = lots.reduce(
    (a, l) => ({
      sats: a.sats + l.sats,
      cost: a.cost + l.costEur,
      proceeds: a.proceeds + l.proceedsEur,
      gain: a.gain + (l.proceedsEur - l.costEur),
    }),
    { sats: 0, cost: 0, proceeds: 0, gain: 0 },
  );
  const kest = totals.gain * j.rate;

  return (
    <div className="flex-1 overflow-auto bg-paper p-3 sm:p-4.5">
      <div className="mb-4 flex flex-col gap-2 sm:mb-4.5 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0">
          <div className="kb-mono-caption">
            Report · Capital gains · {j.name}
          </div>
          <h2 className="m-0 mt-1 font-sans text-[26px] font-semibold tracking-[-0.01em] text-ink sm:text-[32px]">
            Capital gains
          </h2>
        </div>
        <div className="flex items-center gap-4.5">
          <div className="font-mono text-[10px] tracking-[0.08em] text-ink-3">
            STEP {step} / 2
          </div>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-3.5 lg:grid-cols-[minmax(0,340px)_minmax(0,1fr)]">
        {/* Left: config */}
        <div className="flex flex-col gap-2.5">
          <KbCard title="Jurisdiction">
            <div className="flex flex-col gap-2.5">
              <div className="flex flex-wrap gap-1.5">
                {Object.values(JURISDICTIONS).map((x) => {
                  const active = jurCode === x.code;
                  return (
                    <button
                      key={x.code}
                      type="button"
                      onClick={() => {
                        setJurCode(x.code);
                        setMethod(x.defaultMethod);
                      }}
                      className={cn(
                        "cursor-pointer border px-3 py-1 font-mono text-[11px] font-semibold uppercase tracking-[0.08em]",
                        active
                          ? "border-ink bg-ink text-paper"
                          : "border-line bg-transparent text-ink-2",
                      )}
                    >
                      {x.code}
                    </button>
                  );
                })}
              </div>
              <div className="font-sans text-[11px] text-ink-3">
                {j.name} · {j.policy}
              </div>
            </div>
          </KbCard>

          <KbCard title="Reporting period">
            <div className="flex flex-col gap-2.5">
              <div className="flex flex-wrap gap-1.5">
                {REPORTING_YEARS.map((y) => {
                  const active = year === y;
                  return (
                    <button
                      key={y}
                      type="button"
                      onClick={() => setYear(y)}
                      className={cn(
                        "cursor-pointer border px-3 py-1 font-mono text-[11px] font-semibold uppercase tracking-[0.08em]",
                        active
                          ? "border-ink bg-ink text-paper"
                          : "border-line bg-transparent text-ink-2",
                      )}
                    >
                      {y}
                    </button>
                  );
                })}
              </div>
              <div className="grid grid-cols-2 gap-2">
                <LabeledInput
                  label="From"
                  value={`${year}-01-01`}
                  onChange={() => {}}
                  mono
                />
                <LabeledInput
                  label="To"
                  value={`${year}-12-31`}
                  onChange={() => {}}
                  mono
                />
              </div>
            </div>
          </KbCard>

          <KbCard title="Cost-basis method">
            <div className="flex flex-col gap-1.5">
              {COST_BASIS_METHODS.map(({ k, name, desc }) => {
                const active = method === k;
                return (
                  <label
                    key={k}
                    className={cn(
                      "flex cursor-pointer gap-2.5 border border-line p-2.5",
                      active ? "bg-paper" : "bg-transparent",
                    )}
                  >
                    <input
                      type="radio"
                      name="method"
                      checked={active}
                      onChange={() => setMethod(k)}
                      className="mt-0.5 accent-accent"
                    />
                    <div>
                      <div className="font-mono text-xs font-semibold text-ink">
                        {name}
                      </div>
                      <div className="mt-0.5 font-sans text-[11px] text-ink-3">
                        {desc}
                      </div>
                    </div>
                  </label>
                );
              })}
            </div>
          </KbCard>

          <KbCard title="Policy">
            <div className="flex flex-col gap-2">
              <ReportToggleRow
                key={`internal-${jurCode}`}
                label="Treat internal transfers as non-taxable"
                def={j.internalsNonTaxable}
              />
              <ReportToggleRow
                key={`rate-${jurCode}`}
                label={`Apply ${j.rateLabel} flat rate`}
                def={j.rate > 0}
              />
              <ReportToggleRow
                label="Include Lightning channel fees as cost"
                def
              />
              <ReportToggleRow label="Aggregate lots per UTXO set" />
            </div>
          </KbCard>

          <Button
            size="lg"
            onClick={() => setStep(2)}
            className="rounded-none"
          >
            Generate preview
            <ArrowRight className="size-3" />
          </Button>
        </div>

        {/* Right: preview */}
        <div className="flex min-w-0 flex-col gap-2.5">
          <div className="grid grid-cols-2 gap-2.5 md:grid-cols-4">
            <ReportStatTile
              label="Proceeds"
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(totals.proceeds)}
                </span>
              }
              sub={`${lots.length} disposals`}
            />
            <ReportStatTile
              label="Cost basis"
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(totals.cost)}
                </span>
              }
              sub={method.toUpperCase()}
            />
            <ReportStatTile
              label="Net gain"
              value={
                <span
                  className={cn("text-[#3fa66a]", blurClass(hideSensitive))}
                >
                  + {j.ccy} {fmt(totals.gain)}
                </span>
              }
              sub={`${year} tax year`}
            />
            <ReportStatTile
              label={j.rateLabel}
              value={
                <span className={cn("text-accent", blurClass(hideSensitive))}>
                  {j.ccy} {fmt(kest)}
                </span>
              }
              sub="Estimated liability"
            />
          </div>

          <KbCard title={`Disposed lots · ${year}`} pad={false}>
            <div className="overflow-x-auto">
            <table className="w-full min-w-[720px] border-collapse">
              <thead>
                <tr className="border-b border-ink">
                  <th className="px-3 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Acquired
                  </th>
                  <th className="px-3 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Disposed
                  </th>
                  <th className="px-3 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Holding
                  </th>
                  <th className="px-3 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Sats
                  </th>
                  <th className="px-3 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Cost {j.ccy}
                  </th>
                  <th className="px-3 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Proceeds {j.ccy}
                  </th>
                  <th className="px-3 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
                    Gain {j.ccy}
                  </th>
                </tr>
              </thead>
              <tbody>
                {lots.map((l, i) => (
                  <ReportLotRow
                    key={i}
                    lot={l}
                    hideSensitive={hideSensitive}
                  />
                ))}
                <tr className="bg-paper">
                  <td
                    className="px-3 py-2 font-mono text-[11px] font-semibold text-ink"
                    colSpan={3}
                  >
                    Total
                  </td>
                  <td
                    className={cn(
                      "px-3 py-2 text-right font-mono text-[11px] font-semibold tabular-nums text-ink",
                      blurClass(hideSensitive),
                    )}
                  >
                    {totals.sats.toLocaleString("en-US")}
                  </td>
                  <td
                    className={cn(
                      "px-3 py-2 text-right font-mono text-[11px] font-semibold tabular-nums text-ink",
                      blurClass(hideSensitive),
                    )}
                  >
                    {totals.cost.toFixed(2)}
                  </td>
                  <td
                    className={cn(
                      "px-3 py-2 text-right font-mono text-[11px] font-semibold tabular-nums text-ink",
                      blurClass(hideSensitive),
                    )}
                  >
                    {totals.proceeds.toFixed(2)}
                  </td>
                  <td
                    className={cn(
                      "px-3 py-2 text-right font-mono text-[11px] font-semibold tabular-nums text-[#3fa66a]",
                      blurClass(hideSensitive),
                    )}
                  >
                    + {totals.gain.toFixed(2)}
                  </td>
                </tr>
              </tbody>
            </table>
            </div>
          </KbCard>

          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-3">
            <ReportExportFormat
              name="CSV"
              sub="Spreadsheet"
              detail="17 columns · UTF-8"
            />
            <ReportExportFormat
              name="PDF"
              sub="Human-readable"
              detail={`4 pages · ${j.name} format`}
              primary
            />
            <ReportExportFormat
              name="JSON"
              sub="Envelope"
              detail="Machine-readable"
            />
          </div>
        </div>
      </div>
    </div>
  );
}

interface ReportLotRowProps {
  lot: DisposedLot;
  hideSensitive: boolean;
}

function ReportLotRow({ lot, hideSensitive }: ReportLotRowProps) {
  const gain = lot.proceedsEur - lot.costEur;
  const isLong = lot.type === "LT";
  return (
    <tr className="border-b border-line">
      <td className="px-3 py-2 font-mono text-[11px] text-ink-2">
        {lot.acquired}
      </td>
      <td className="px-3 py-2 font-mono text-[11px] text-ink-2">
        {lot.disposed}
      </td>
      <td className="px-3 py-2">
        <span
          className={cn(
            "border px-1.5 py-0.5 font-mono text-[9px] tracking-[0.1em]",
            isLong
              ? "border-[#3fa66a] text-[#3fa66a]"
              : "border-ink-3 text-ink-2",
          )}
        >
          {isLong ? "> 1Y" : "< 1Y"}
        </span>
      </td>
      <td
        className={cn(
          "px-3 py-2 text-right font-mono text-[11px] tabular-nums text-ink-2",
          blurClass(hideSensitive),
        )}
      >
        {lot.sats.toLocaleString("en-US")}
      </td>
      <td
        className={cn(
          "px-3 py-2 text-right font-mono text-[11px] tabular-nums text-ink-2",
          blurClass(hideSensitive),
        )}
      >
        {lot.costEur.toFixed(2)}
      </td>
      <td
        className={cn(
          "px-3 py-2 text-right font-mono text-[11px] tabular-nums text-ink-2",
          blurClass(hideSensitive),
        )}
      >
        {lot.proceedsEur.toFixed(2)}
      </td>
      <td
        className={cn(
          "px-3 py-2 text-right font-mono text-[11px] tabular-nums text-[#3fa66a]",
          blurClass(hideSensitive),
        )}
      >
        + {gain.toFixed(2)}
      </td>
    </tr>
  );
}

interface ReportStatTileProps {
  label: string;
  value: ReactNode;
  sub: string;
}

function ReportStatTile({ label, value, sub }: ReportStatTileProps) {
  return (
    <div className="flex flex-col gap-1 border border-line bg-paper-2 p-3.5">
      <div className="kb-mono-caption">{label}</div>
      <div className="font-sans text-[20px] font-medium leading-[1.1] tracking-[-0.005em] text-ink">
        {value}
      </div>
      <div className="font-mono text-[10px] text-ink-3">{sub}</div>
    </div>
  );
}

interface ReportToggleRowProps {
  label: string;
  def?: boolean;
}

function ReportToggleRow({ label, def }: ReportToggleRowProps) {
  const [on, setOn] = useState(!!def);
  return (
    <button
      type="button"
      onClick={() => setOn((v) => !v)}
      className="flex cursor-pointer items-center gap-2.5 border-none bg-transparent p-0 text-left"
    >
      <span
        className={cn(
          "relative inline-block h-4 w-[30px] transition-colors",
          on ? "bg-ink" : "bg-line-2",
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 size-3 bg-paper-2 transition-[left]",
            on ? "left-4" : "left-0.5",
          )}
        />
      </span>
      <span className="font-sans text-xs text-ink-2">{label}</span>
    </button>
  );
}

interface ReportExportFormatProps {
  name: string;
  sub: string;
  detail: string;
  primary?: boolean;
}

function ReportExportFormat({
  name,
  sub,
  detail,
  primary,
}: ReportExportFormatProps) {
  return (
    <button
      type="button"
      className={cn(
        "flex cursor-pointer flex-col gap-0.5 border p-4 text-left",
        primary
          ? "border-ink bg-ink text-paper"
          : "border-line bg-paper-2 text-ink",
      )}
    >
      <div className="flex items-center justify-between">
        <span className="font-sans text-[22px]">{name}</span>
        <span className="font-mono text-sm">⤓</span>
      </div>
      <span className="font-sans text-[11px] opacity-70">{sub}</span>
      <span className="mt-1.5 font-mono text-[10px] opacity-55">{detail}</span>
    </button>
  );
}
