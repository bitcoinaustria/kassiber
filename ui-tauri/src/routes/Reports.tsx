/**
 * Reports — capital-gains export flow.
 *
 * The screen still uses fixture data through the mock daemon, but the
 * layout now follows the shared shadcn dashboard language used by Overview
 * and Transactions.
 */

import { useState, type ReactNode } from "react";
import { ArrowRight, FileSpreadsheet, FileText } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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
  { k: "fifo", name: "FIFO", desc: "First-in, first-out" },
  { k: "lifo", name: "LIFO", desc: "Last-in, first-out" },
  { k: "hifo", name: "HIFO", desc: "Highest-in, first-out" },
  { k: "spec", name: "Specific ID", desc: "Per-lot selection" },
];

const REPORTING_YEARS = [2023, 2024, 2025, 2026] as const;

export function Reports() {
  const { data, isLoading } = useDaemon<CapitalGainsReport>(
    "ui.reports.capital_gains",
  );
  const hideSensitive = useUiStore((s) => s.hideSensitive);

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading reports...
      </div>
    );
  }

  if (data?.error || !data?.data) {
    return (
      <div className="w-full bg-background p-3 sm:p-4 md:p-6">
        <Card>
          <CardHeader>
            <CardTitle>Reports unavailable</CardTitle>
            <CardDescription>
              {data?.error?.message ?? "The daemon did not return report data."}
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    );
  }

  return <ReportsView report={data.data} hideSensitive={hideSensitive} />;
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
    <div className="w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6">
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,360px)_minmax(0,1fr)]">
        <ReportControls
          year={year}
          setYear={setYear}
          jurCode={jurCode}
          setJurCode={(code) => {
            const next = JURISDICTIONS[code] ?? JURISDICTIONS.AT;
            setJurCode(code);
            setMethod(next.defaultMethod);
          }}
          method={method}
          setMethod={setMethod}
          rateLabel={j.rateLabel}
          policy={j.policy}
        />

        <div className="min-w-0 space-y-4">
          {report.status?.needsJournals && (
            <Card className="border-amber-200 bg-amber-50 text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-100">
              <CardHeader>
                <CardTitle>Journals need processing</CardTitle>
                <CardDescription className="text-amber-800 dark:text-amber-200">
                  Recent transaction changes are not reflected in trusted report
                  totals yet.
                </CardDescription>
              </CardHeader>
            </Card>
          )}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
            <ReportMetricCard
              label="Proceeds"
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(totals.proceeds)}
                </span>
              }
              sub={`${lots.length} disposals`}
            />
            <ReportMetricCard
              label="Cost Basis"
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(totals.cost)}
                </span>
              }
              sub={method.toUpperCase()}
            />
            <ReportMetricCard
              label="Net Gain"
              value={
                <span
                  className={cn(
                    totals.gain >= 0 ? "text-emerald-600" : "text-red-600",
                    blurClass(hideSensitive),
                  )}
                >
                  {signedMoney(j.ccy, totals.gain, fmt)}
                </span>
              }
              sub={`${year} tax year`}
            />
            <ReportMetricCard
              label={j.rateLabel}
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(kest)}
                </span>
              }
              sub="Estimated liability"
            />
          </div>

          <Card>
            <CardHeader className="border-b">
              <CardTitle>Disposed lots · {year}</CardTitle>
              <CardDescription>
                Previewed disposal lots for the selected jurisdiction and
                method.
              </CardDescription>
            </CardHeader>
            <CardContent className="overflow-x-auto p-0">
              <Table>
                <TableHeader>
                  <TableRow className="bg-muted/50 hover:bg-muted/50">
                    <TableHead className="min-w-[112px]">Acquired</TableHead>
                    <TableHead className="min-w-[112px]">Disposed</TableHead>
                    <TableHead>Holding</TableHead>
                    <TableHead className="text-right">Sats</TableHead>
                    <TableHead className="text-right">Cost {j.ccy}</TableHead>
                    <TableHead className="text-right">
                      Proceeds {j.ccy}
                    </TableHead>
                    <TableHead className="text-right">Gain {j.ccy}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {lots.map((lot, index) => (
                    <ReportLotRow
                      key={`${lot.acquired}-${lot.disposed}-${index}`}
                      lot={lot}
                      hideSensitive={hideSensitive}
                    />
                  ))}
                  <TableRow className="bg-muted/30 font-medium">
                    <TableCell colSpan={3}>Total</TableCell>
                    <TableCell
                      className={cn(
                        "text-right tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {totals.sats.toLocaleString("en-US")}
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-right tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {totals.cost.toFixed(2)}
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-right tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {totals.proceeds.toFixed(2)}
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-right tabular-nums",
                        totals.gain >= 0 ? "text-emerald-600" : "text-red-600",
                        blurClass(hideSensitive),
                      )}
                    >
                      {signedNumber(totals.gain)}
                    </TableCell>
                  </TableRow>
                </TableBody>
              </Table>
            </CardContent>
          </Card>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            <ReportExportFormat
              icon={FileSpreadsheet}
              name="CSV"
              sub="Spreadsheet"
              detail="17 columns · UTF-8"
            />
            <ReportExportFormat
              icon={FileText}
              name="PDF"
              sub="Human-readable"
              detail={`4 pages · ${j.name} format`}
              primary
            />
            <ReportExportFormat
              icon={FileSpreadsheet}
              name="XLSX"
              sub="Spreadsheet"
              detail="Multi-sheet workbook"
            />
          </div>
        </div>
      </div>
    </div>
  );
}

interface ReportControlsProps {
  year: number;
  setYear: (year: number) => void;
  jurCode: string;
  setJurCode: (code: string) => void;
  method: CostBasisMethod;
  setMethod: (method: CostBasisMethod) => void;
  rateLabel: string;
  policy: string;
}

function ReportControls({
  year,
  setYear,
  jurCode,
  setJurCode,
  method,
  setMethod,
  rateLabel,
  policy,
}: ReportControlsProps) {
  return (
    <Card className="h-fit">
      <CardHeader className="border-b">
        <CardTitle>Report setup</CardTitle>
        <CardDescription>{policy}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <section className="space-y-3">
          <Label>Jurisdiction</Label>
          <Select value={jurCode} onValueChange={setJurCode}>
            <SelectTrigger className="w-full">
              <SelectValue placeholder="Select jurisdiction" />
            </SelectTrigger>
            <SelectContent>
              {Object.values(JURISDICTIONS).map((x) => (
                <SelectItem key={x.code} value={x.code}>
                  {x.code} · {x.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </section>

        <section className="space-y-3">
          <Label>Reporting period</Label>
          <Select
            value={String(year)}
            onValueChange={(value) => setYear(Number(value))}
          >
            <SelectTrigger className="w-full">
              <SelectValue placeholder="Select tax year" />
            </SelectTrigger>
            <SelectContent>
              {REPORTING_YEARS.map((y) => (
                <SelectItem key={y} value={String(y)}>
                  {y}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-1.5">
              <Label htmlFor="report-from">From</Label>
              <Input id="report-from" value={`${year}-01-01`} readOnly />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="report-to">To</Label>
              <Input id="report-to" value={`${year}-12-31`} readOnly />
            </div>
          </div>
        </section>

        <section className="space-y-3">
          <Label>Cost-basis method</Label>
          <div className="grid gap-2">
            {COST_BASIS_METHODS.map(({ k, name, desc }) => {
              const active = method === k;
              return (
                <button
                  key={k}
                  type="button"
                  onClick={() => setMethod(k)}
                  className={cn(
                    "rounded-md border p-3 text-left transition-colors",
                    active
                      ? "border-primary bg-primary/5"
                      : "bg-background hover:bg-muted/50",
                  )}
                >
                  <span className="flex items-center gap-2">
                    <span
                      className={cn(
                        "size-2 rounded-full",
                        active ? "bg-primary" : "bg-muted-foreground/40",
                      )}
                    />
                    <span className="font-medium">{name}</span>
                  </span>
                  <span className="mt-1 block text-xs text-muted-foreground">
                    {desc}
                  </span>
                </button>
              );
            })}
          </div>
        </section>

        <section className="space-y-3">
          <Label>Policy</Label>
          <div className="grid gap-2">
            <ReportToggleRow label="Treat internal transfers as non-taxable" def />
            <ReportToggleRow label={`Apply ${rateLabel} flat rate`} def />
            <ReportToggleRow label="Include Lightning channel fees as cost" def />
            <ReportToggleRow label="Aggregate lots per UTXO set" />
          </div>
        </section>

        <Button type="button" className="w-full gap-2">
          Generate preview
          <ArrowRight className="size-4" aria-hidden="true" />
        </Button>
      </CardContent>
    </Card>
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
    <TableRow>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {lot.acquired}
      </TableCell>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {lot.disposed}
      </TableCell>
      <TableCell>
        <span
          className={cn(
            "inline-flex rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset",
            isLong
              ? "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20"
              : "bg-muted text-muted-foreground ring-border",
          )}
        >
          {isLong ? "> 1Y" : "< 1Y"}
        </span>
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {lot.sats.toLocaleString("en-US")}
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {lot.costEur.toFixed(2)}
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {lot.proceedsEur.toFixed(2)}
      </TableCell>
      <TableCell
        className={cn(
          "text-right tabular-nums",
          gain >= 0 ? "text-emerald-600" : "text-red-600",
          blurClass(hideSensitive),
        )}
      >
        {signedNumber(gain)}
      </TableCell>
    </TableRow>
  );
}

function signedNumber(value: number) {
  return `${value >= 0 ? "+" : "-"} ${Math.abs(value).toFixed(2)}`;
}

function signedMoney(
  currency: string,
  value: number,
  format: (value: number) => string,
) {
  return `${value >= 0 ? "+" : "-"} ${currency} ${format(Math.abs(value))}`;
}

interface ReportMetricCardProps {
  label: string;
  value: ReactNode;
  sub: string;
}

function ReportMetricCard({ label, value, sub }: ReportMetricCardProps) {
  return (
    <Card className="gap-3 py-5">
      <CardContent className="space-y-3">
        <p className="text-xs font-medium text-muted-foreground">{label}</p>
        <p className="text-2xl font-semibold tracking-tight">{value}</p>
        <p className="text-xs text-muted-foreground">{sub}</p>
      </CardContent>
    </Card>
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
      className="flex items-center justify-between gap-3 rounded-md border bg-background px-3 py-2 text-left text-sm transition-colors hover:bg-muted/50"
    >
      <span>{label}</span>
      <span
        className={cn(
          "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors",
          on ? "bg-primary" : "bg-muted",
        )}
      >
        <span
          className={cn(
            "size-4 rounded-full bg-background shadow transition-transform",
            on ? "translate-x-4" : "translate-x-0.5",
          )}
        />
      </span>
    </button>
  );
}

interface ReportExportFormatProps {
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  name: string;
  sub: string;
  detail: string;
  primary?: boolean;
}

function ReportExportFormat({
  icon: Icon,
  name,
  sub,
  detail,
  primary,
}: ReportExportFormatProps) {
  return (
    <Button
      type="button"
      variant={primary ? "default" : "outline"}
      className="h-auto min-h-20 min-w-0 justify-start gap-3 whitespace-normal p-4 text-left"
    >
      <Icon className="size-5 shrink-0" aria-hidden="true" />
      <span className="grid min-w-0 gap-1">
        <span className="font-medium">{name}</span>
        <span
          className={cn(
            "whitespace-normal text-xs leading-snug font-normal break-words",
            primary ? "text-primary-foreground/75" : "text-muted-foreground",
          )}
        >
          {sub} · {detail}
        </span>
      </span>
    </Button>
  );
}
