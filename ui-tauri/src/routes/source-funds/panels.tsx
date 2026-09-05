import { findingTranslationKeys } from "./findingCopy";
import { Trans, useTranslation } from "react-i18next";
// Leaf panels of the source-of-funds workstation. Presentation only:
// every mutation/query lives in useSourceFundsCase, every payload shape in
// model.ts.

import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowRight,
  ArrowUpRight,
  ChevronDown,
  Eye,
  ExternalLink,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
} from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { openExternalUrl } from "@/daemon/transport";
import { formatFiatAmount } from "@/lib/currency";
import { formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import {
  COVERAGE_BUCKET_BARS,
  COVERAGE_BUCKET_ORDER,
  COVERAGE_BUCKET_TONES,
  NO_ATTACHMENT,
  NO_RECIPIENT,
  PROVENANCE_SHORT_LABELS,
  REVEAL_MODES,
  formatBtc,
  formatDateTime,
  pretty,
  shortId,
  stringValue,
  txSignedAmount,
  txAmount,
  txDate,
  txDirection,
  txFlow,
  txLabel,
  txRef,
  txWallet,
  uniqueSorted,
  type EvidenceAttachment,
  type SourceFundsCoverage,
  type SourceFundsCoverageBuckets,
  type SourceFundsFinding,
  type SourceFundsPreview,
  type SourceFundsRecipient,
  type TransactionRow,
} from "./model";

export function ReportControlFields({
  amountLabel,
  targetAmount,
  selectedTx,
  revealMode,
  onAmountChange,
  onRevealModeChange,
}: {
  amountLabel: string;
  targetAmount: string;
  selectedTx?: TransactionRow;
  revealMode: string;
  onAmountChange: (value: string) => void;
  onRevealModeChange: (value: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  return (
    <>
      <Field label={amountLabel} htmlFor="sof-amount">
        <Input
          id="sof-amount"
          value={targetAmount}
          onChange={(event) => onAmountChange(event.target.value)}
          placeholder={selectedTx ? txAmount(selectedTx) : "0.00000000"}
        />
        <p className="mt-1 text-xs text-muted-foreground">{t("case.optionalAmount")}</p>
      </Field>
      <Field label={t("controls.revealLabel")} htmlFor="sof-reveal">
        <Select value={revealMode} onValueChange={onRevealModeChange}>
          <SelectTrigger id="sof-reveal" className="h-10 w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {REVEAL_MODES.map((mode) => (
              <SelectItem key={mode} value={mode}>
                {t(`reveal.${mode}`, { defaultValue: pretty(mode) })}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
    </>
  );
}

export function TransactionTargetRow({
  row,
  active,
  onSelect,
  onOpenDetails,
}: {
  row: TransactionRow;
  active: boolean;
  onSelect: () => void;
  onOpenDetails: () => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const flow = txFlow(row);
  const FlowIcon =
    flow === "incoming"
      ? ArrowDownRight
      : flow === "outgoing"
        ? ArrowUpRight
        : ArrowLeftRight;
  const flowClassName =
    flow === "incoming"
      ? "border-emerald-600/20 bg-emerald-50 text-emerald-700 dark:bg-emerald-900/25 dark:text-emerald-300"
      : flow === "outgoing"
        ? "border-red-600/20 bg-red-50 text-red-700 dark:bg-red-900/25 dark:text-red-300"
        : "border-zinc-500/20 bg-zinc-50 text-zinc-700 dark:bg-zinc-800/70 dark:text-zinc-300";
  const amountClassName =
    flow === "incoming"
      ? "text-emerald-700 dark:text-emerald-300"
      : flow === "outgoing"
        ? "text-red-700 dark:text-red-300"
        : "text-muted-foreground";
  const txid = row.external_id || row.externalId || row.id;
  const description = row.counter || row.description || row.note || txid || t("transactionRow.fallbackDescription");

  return (
    <div
      className={cn(
        "grid grid-cols-[minmax(0,1fr)_44px] transition-colors hover:bg-muted/35",
        active && "bg-primary/5 ring-1 ring-primary/30 ring-inset",
      )}
    >
      <button
        type="button"
        className="min-w-0 px-3 py-2 text-left"
        onClick={onSelect}
      >
        <div className="grid min-w-0 gap-3 md:grid-cols-[minmax(0,1fr)_150px_160px_130px] md:items-center">
          <div className="flex min-w-0 items-start gap-3">
            <span
              className={`mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border ${flowClassName}`}
              aria-hidden="true"
            >
              <FlowIcon className="size-4" />
            </span>
            <div className="min-w-0">
              <div className="truncate text-sm font-medium text-foreground">
                {description}
              </div>
              <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                <span>{row.asset || "BTC"}</span>
                <span className="break-all font-mono">
                  {shortId(txid)}
                </span>
                {row.direction && (
                  <span className="md:hidden">{t(txDirection(row) === "inbound" ? "flow.incoming" : "flow.outgoing")}</span>
                )}
              </div>
            </div>
          </div>
          <div className={`font-mono text-sm tabular-nums md:text-right ${amountClassName}`}>
            {txSignedAmount(row)}
          </div>
          <div className="text-sm text-muted-foreground">
            <span className="md:hidden">{t("transactionRow.walletMobile")}</span>
            {txWallet(row)}
          </div>
          <div className="flex flex-wrap items-center gap-2 md:justify-end">
            <span className={`inline-flex items-center gap-1 rounded-md border px-2 py-1 text-xs font-medium ${flowClassName}`}>
              <FlowIcon className="size-3.5" aria-hidden="true" />
              {t(`flow.${flow}`, { defaultValue: flow })}
            </span>
            <span className="text-xs text-muted-foreground">
              {txDate(row)}
            </span>
          </div>
        </div>
      </button>
      <button
        type="button"
        className="flex shrink-0 items-center justify-center border-l text-muted-foreground transition-colors hover:bg-muted/45 hover:text-foreground"
        onClick={onOpenDetails}
        aria-label={t("workstation.viewTransactionDetails")}
        title={t("workstation.viewDetails")}
      >
        <Eye className="size-4" aria-hidden="true" />
      </button>
    </div>
  );
}


export function TransactionTargetHeader() {
  const { t } = useTranslation("sourceFunds");
  return (
    <div className="hidden border-b bg-muted/50 px-3 py-2 text-xs font-medium text-muted-foreground md:grid md:grid-cols-[minmax(0,1fr)_150px_160px_130px_44px] md:gap-3">
      <span>{t("transactionRow.header.transaction")}</span>
      <span className="text-right">{t("sourceOrGap.amount")}</span>
      <span>{t("fallback.wallet")}</span>
      <span className="text-right">{t("transactionRow.header.flow")}</span>
      <span aria-hidden="true" />
    </div>
  );
}


export function OptionalSection({
  open,
  onOpenChange,
  icon,
  title,
  summary,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  icon: ReactNode;
  title: string;
  summary?: string;
  children: ReactNode;
}) {
  return (
    <Collapsible open={open} onOpenChange={onOpenChange}>
      <section className="kb-surface-inset">
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left"
          >
            <span className="flex min-w-0 items-center gap-2">
              {icon}
              <span className="min-w-0">
                <span className="block text-sm font-semibold">{title}</span>
                {summary && (
                  <span className="block truncate text-xs text-muted-foreground">
                    {summary}
                  </span>
                )}
              </span>
            </span>
            <ChevronDown
              className={[
                "size-4 shrink-0 text-muted-foreground transition-transform",
                open ? "rotate-180" : "",
              ].join(" ")}
              aria-hidden="true"
            />
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="border-t p-4">{children}</div>
        </CollapsibleContent>
      </section>
    </Collapsible>
  );
}


export function CaseBrief({
  report,
  bulkReviewable,
  manualReview,
  onOpenTransaction,
}: {
  report?: SourceFundsPreview;
  bulkReviewable: number;
  manualReview: number;
  onOpenTransaction?: (txId: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const overview = report?.overview;
  const targetAsset = overview?.target_asset || report?.target.asset || "BTC";
  const paragraphs = report?.narrative?.paragraphs ?? [];
  const sources = report?.source_mix ?? [];
  const dataSources = report?.data_sources ?? [];
  const context = report?.report_context;
  const jurisdiction = context?.jurisdiction_label;
  const fiatCurrency = context?.fiat_currency;
  return (
    <section className="space-y-4 rounded-md border bg-muted/20 p-4">
      <div className="flex flex-col gap-2 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <h2 className="text-base font-semibold">
            {overview?.target_label || report?.target.label || t("caseBrief.fallbackTarget")}
          </h2>
          <p className="text-sm text-muted-foreground">
            {formatDateTime(overview?.target_date)} ·{" "}
            {overview?.target_wallet || report?.target.wallet || t("caseBrief.fallbackWallet")} ·{" "}
            {formatBtc(overview?.target_amount ?? report?.target.required_amount, targetAsset)}
            {(jurisdiction || fiatCurrency) && (
              <>
                {" "}
                · {[jurisdiction, fiatCurrency].filter(Boolean).join(" / ")}
              </>
            )}
          </p>
        </div>
        <span className="text-xs text-muted-foreground">{t(report?.explain_gates.exportable ? "case.exportable" : "case.needsEvidence")}</span>
      </div>
      <div className="grid gap-3 md:grid-cols-5">
        <Metric label={t("caseBrief.metric.transactions")} value={overview?.transaction_count ?? 0} />
        <Metric label={t("caseBrief.metric.reviewedLinks")} value={overview?.link_count ?? 0} />
        <Metric label={t("caseBrief.metric.sources")} value={overview?.source_category_count ?? 0} />
        <Metric label={t("caseBrief.metric.blockers")} value={overview?.blocker_count ?? 0} />
        <Metric label={t("caseBrief.metric.batchable")} value={bulkReviewable} />
      </div>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_340px]">
        <div className="space-y-2">
          {paragraphs.length > 0 ? (
            paragraphs.slice(0, 3).map((paragraph) => (
              <p key={paragraph} className="text-sm text-muted-foreground">
                {paragraph}
              </p>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">
              {t("workstation.localPreviewDataIsNot")}</p>
          )}
          {manualReview > 0 && (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              {t("caseBrief.manualReview", { count: manualReview })}
            </p>
          )}
        </div>
        <div className="space-y-2">
          <div className="rounded-md border bg-background">
            {(sources.length > 0 ? sources : [{ source_type: "unresolved", amount: 0, count: 0 }]).map(
              (source) => (
                <div
                  key={source.source_type}
                  className="flex items-center justify-between gap-3 border-b px-3 py-2 text-sm last:border-b-0"
                >
                  <span className="truncate">{pretty(source.source_type)}</span>
                  <span className="font-mono text-xs tabular-nums">
                    {formatBtc(source.amount, targetAsset)}
                  </span>
                </div>
              ),
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            {dataSources.slice(0, 4).map((source) => (
              <span
                key={`${source.kind}-${source.label}`}
                className="rounded-full border bg-background px-2 py-1 text-xs text-muted-foreground"
              >
                {source.label} · {source.transaction_count + source.source_count}
              </span>
            ))}
            {dataSources.length > 4 && (
              <span className="rounded-full border bg-background px-2 py-1 text-xs text-muted-foreground">
                +{dataSources.length - 4}
              </span>
            )}
          </div>
        </div>
      </div>
      <FlowPathPreview
        flow={report?.simplified_flow}
        onOpenTransaction={onOpenTransaction}
      />
    </section>
  );
}

// Maps the on-device (light, print-matching) diagram palette to a dark-mode
// palette. The frozen SVG stays light so it matches the exported PDF; the app
// recolours it for the dark theme on screen only.


const DARK_SVG_SUBS: ReadonlyArray<readonly [RegExp, string]> = [
  [/#222222/gi, "#e5e7eb"], // ink / text
  [/#666666/gi, "#9ca3af"], // muted text
  [/#d9d9d9/gi, "#3f3f46"], // hairlines
  [/#ffffff/gi, "#09090b"], // surfaces / donut hole / neutral fills
  [/#f7f7f7/gi, "#18181b"], // soft surface
  [/#ecfdf5/gi, "#06281f"], // root-source fill
  [/#16a34a/gi, "#34d399"], // root-source / income
  [/#fffbeb/gi, "#2a1d07"], // attestation fill
  [/#d97706/gi, "#fbbf24"], // attestation / manual
  [/#fff7ed/gi, "#2a1607"], // privacy fill
  [/#ea580c/gi, "#fb923c"], // privacy stroke / edge
  [/#e3000f/gi, "#f87171"], // target / accent
  [/#2563eb/gi, "#60a5fa"], // swap edge / fiat purchase / wallet
  [/#dbeafe/gi, "#1e3a5f"], // swap legend chip
  [/#0ea5e9/gi, "#38bdf8"], // exchange
  [/#65a30d/gi, "#a3e635"], // mining
  [/#a855f7/gi, "#c084fc"], // gift
  [/#0891b2/gi, "#22d3ee"], // blockchain
  [/#6b7280/gi, "#9ca3af"], // unknown
  [/#dc2626/gi, "#f87171"], // fallback red
];


function toDarkSvg(svg: string): string {
  return DARK_SVG_SUBS.reduce((acc, [pattern, color]) => acc.replace(pattern, color), svg);
}


function useIsDark(): boolean {
  const [dark, setDark] = useState(
    () =>
      typeof document !== "undefined" &&
      document.documentElement.classList.contains("dark"),
  );
  useEffect(() => {
    const root = document.documentElement;
    const sync = () => setDark(root.classList.contains("dark"));
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);
  return dark;
}


export function ReportDiagram({ svg, label }: { svg?: string; label: string }) {
  const dark = useIsDark();
  if (!svg) {
    return null;
  }
  // Rendered on-device; embedded as a sandboxed <img> so any user-supplied
  // label text in the SVG can never execute as markup. Recoloured for dark mode.
  const themed = dark ? toDarkSvg(svg) : svg;
  const src = `data:image/svg+xml;utf8,${encodeURIComponent(themed)}`;
  return (
    <figure className="space-y-1">
      <img
        src={src}
        alt={label}
        className="w-full rounded-md border bg-white dark:bg-zinc-950"
      />
      <figcaption className="text-xs text-muted-foreground">{label}</figcaption>
    </figure>
  );
}


export function FlowPathPreview({
  flow,
  onOpenTransaction,
}: {
  flow?: SourceFundsPreview["simplified_flow"];
  onOpenTransaction?: (txId: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const levels = flow?.levels ?? [];
  if (levels.length === 0) {
    return null;
  }
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">{t("flowPath.title")}</h3>
        {flow?.deferred_privacy_hops?.length ? (
          <span className="rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200">
            {t("flowPath.privacyHopDeferred")}</span>
        ) : null}
      </div>
      {flow?.note && (
        <p className="text-xs text-muted-foreground">{flow.note}</p>
      )}
      <div className="overflow-x-auto pb-1">
        <div className="flex min-w-max items-stretch gap-2">
          {levels.map((level, levelIndex) => {
            const nodes = level.nodes.slice(0, 3);
            const hidden = Math.max(0, level.nodes.length - nodes.length);
            return (
              <div
                key={`${level.role ?? "level"}-${levelIndex}`}
                className="flex items-center gap-2"
              >
                <div className="w-44 rounded-md border bg-background p-2">
                  <div className="mb-2 text-2xs font-semibold uppercase text-muted-foreground">
                    {pretty(level.role || "flow")}
                  </div>
                  <div className="space-y-1">
                    {nodes.map((node) => {
                      const transactionId = stringValue(node.transaction_id);
                      const clickable =
                        node.node_type === "transaction" &&
                        Boolean(transactionId) &&
                        Boolean(onOpenTransaction);
                      const nodeClassName = [
                        "block w-full rounded border px-2 py-1 text-left",
                        node.deferred_privacy_hop
                          ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100"
                          : level.role === "target"
                            ? "border-primary/35 bg-primary/5"
                            : "bg-muted/25",
                        clickable
                          ? "cursor-pointer transition-colors hover:border-primary/50"
                          : "",
                      ].join(" ");
                      const nodeContent = (
                        <>
                          <div className="truncate text-xs font-medium">
                            {node.label || node.id}
                          </div>
                          <div className="mt-0.5 truncate text-xs text-muted-foreground">
                            {pretty(node.kind || node.node_type || "")}
                            {node.amount != null
                              ? ` · ${formatBtc(node.amount, node.asset || "BTC")}`
                              : ""}
                          </div>
                        </>
                      );
                      return clickable ? (
                        <button
                          key={node.id}
                          type="button"
                          className={nodeClassName}
                          onClick={() => onOpenTransaction?.(transactionId)}
                          title={t("workstation.openTransactionDetails")}
                        >
                          {nodeContent}
                        </button>
                      ) : (
                        <div key={node.id} className={nodeClassName}>
                          {nodeContent}
                        </div>
                      );
                    })}
                    {hidden > 0 && (
                      <div className="rounded border border-dashed px-2 py-1 text-xs text-muted-foreground">
                        +{hidden} more
                      </div>
                    )}
                  </div>
                </div>
                {levelIndex < levels.length - 1 && (
                  <ArrowRight
                    className="size-4 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}


export function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 font-mono text-lg font-semibold tabular-nums">
        {formatCount(value)}
      </div>
    </div>
  );
}


export function PurposeButton({
  active,
  title,
  body,
  onClick,
}: {
  active: boolean;
  title: string;
  body: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={[
        "rounded-md border px-3 py-3 text-left transition-colors",
        active ? "border-primary bg-primary/5" : "hover:bg-muted/60",
      ].join(" ")}
      onClick={onClick}
    >
      <span className="block text-sm font-semibold">{title}</span>
      <span className="mt-1 block text-xs text-muted-foreground">{body}</span>
    </button>
  );
}


export function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
    </div>
  );
}


export function SelectField({
  id,
  label,
  value,
  options,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <Field label={label} htmlFor={id}>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} className="h-10 w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option} value={option}>
              {pretty(option)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );
}


export function TransactionSelect({
  id,
  label,
  rows,
  value,
  onChange,
  defaultLabel,
}: {
  id: string;
  label: string;
  rows: TransactionRow[];
  value: string;
  onChange: (value: string) => void;
  defaultLabel?: string;
}) {
  const { t } = useTranslation("sourceFunds");
  const defaultValue = "__source_funds_selected_target__";
  return (
    <Field label={label} htmlFor={id}>
      <Select
        value={defaultLabel && !value ? defaultValue : value || undefined}
        onValueChange={(next) => onChange(next === defaultValue ? "" : next)}
      >
        <SelectTrigger id={id} className="h-10 w-full">
          <SelectValue placeholder={t("select.transaction")} />
        </SelectTrigger>
        <SelectContent>
          {defaultLabel ? (
            <SelectItem value={defaultValue}>{defaultLabel}</SelectItem>
          ) : null}
          {rows.map((row) => (
            <SelectItem key={txRef(row)} value={txRef(row)}>
              {txLabel(row)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );
}


export function EvidenceSelect({
  id,
  value,
  evidence,
  onChange,
}: {
  id: string;
  value: string;
  evidence: EvidenceAttachment[];
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  return (
    <Field label={t("evidence.label")} htmlFor={id}>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} className="h-10 w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NO_ATTACHMENT}>{t("select.noAttachment")}</SelectItem>
          {evidence.map((item) => (
            <SelectItem key={item.id} value={item.id}>
              {[item.label, item.wallet, item.external_id].filter(Boolean).join(" · ")}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );
}


export function StatusPill({ state }: { state: string }) {
  const { t } = useTranslation("sourceFunds");
  const className =
    state === "reviewed"
      ? "border-emerald-200 bg-emerald-50 text-emerald-800 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200"
      : state === "rejected"
        ? "border-muted bg-muted text-muted-foreground"
        : "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200";
  return (
    <span className={`rounded-full border px-2 py-0.5 text-xs ${className}`}>
      {t(`linkState.${state}`, { defaultValue: pretty(state) })}
    </span>
  );
}


export function TracedCoverageHero({ coverage }: { coverage?: SourceFundsCoverage }) {
  const { t } = useTranslation("sourceFunds");
  const totals = coverage?.totals;
  const total = totals?.amount ?? 0;
  const txCount = totals?.tx_count ?? 0;
  const buckets = totals?.buckets;
  if (!coverage || txCount === 0) {
    return null;
  }
  const pct = (name: keyof SourceFundsCoverageBuckets) =>
    total > 0 ? ((buckets?.[name]?.amount ?? 0) / total) * 100 : 0;
  return (
    <Card>
      <CardContent className="p-4">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <div className="text-xs uppercase tracking-wide text-muted-foreground">
              {t("workstation.inboundHistoryTraced")}</div>
            <div className="mt-0.5 flex items-baseline gap-2">
              <span className="font-mono text-3xl font-semibold tabular-nums text-emerald-700 dark:text-emerald-300">
                {pct("fully_traced").toFixed(1)}%
              </span>
              <span className="text-sm text-muted-foreground">
                {t("case.fullyTraced", { count: txCount })}
              </span>
            </div>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
            {COVERAGE_BUCKET_ORDER.filter(
              (name) => name === "fully_traced" || (buckets?.[name]?.amount ?? 0) > 0,
            ).map((name) => (
              <span key={name} className="inline-flex items-center gap-1.5">
                <span className={`size-2.5 rounded-sm ${COVERAGE_BUCKET_BARS[name]}`} />
                <span className="text-muted-foreground">
                  {t(`coverageBucket.${name}`)}
                </span>
                <span className={`font-medium ${COVERAGE_BUCKET_TONES[name]}`}>
                  {pct(name).toFixed(1)}%
                </span>
              </span>
            ))}
          </div>
        </div>
        <div className="mt-3 flex h-2.5 w-full overflow-hidden rounded-full bg-muted">
          {COVERAGE_BUCKET_ORDER.map((name) => {
            const percent = pct(name);
            return percent > 0 ? (
              <div
                key={name}
                className={COVERAGE_BUCKET_BARS[name]}
                style={{ width: `${percent}%` }}
                title={`${t(`coverageBucket.${name}`)}: ${percent.toFixed(1)}%`}
              />
            ) : null;
          })}
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          {t("case.attested", { percent: pct("attested").toFixed(1) })}
        </p>
        {coverage.truncation?.truncated && (
          <p className="mt-1 text-xs text-amber-700 dark:text-amber-300">
            {t("case.partialCoverage", { shown: txCount, total: coverage.truncation.inbound_total_count })}
          </p>
        )}
      </CardContent>
    </Card>
  );
}


export function CoveragePanel({
  coverage,
  loading,
}: {
  coverage?: SourceFundsCoverage;
  loading?: boolean;
}) {
  const { t } = useTranslation("sourceFunds");
  const totals = coverage?.totals;
  const totalAmount = totals?.amount ?? 0;
  const totalTxCount = totals?.tx_count ?? 0;
  const buckets = totals?.buckets;
  const denominator = totalAmount > 0 ? totalAmount : 0;
  return (
    <section>
        {loading && !coverage ? (
          <EmptyState text={t("workstation.computingCoverage")} />
        ) : !coverage || totalTxCount === 0 ? (
          <EmptyState text={t("coverage.noInbound")} />
        ) : (
          <>
            {coverage.truncation?.truncated && (
              <div className="mb-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100">
                <Trans t={t} i18nKey="coverage.truncated" values={{ shown: totalTxCount, total: coverage.truncation.inbound_total_count, notClassified: coverage.truncation.not_classified_count }} components={[<code className="text-2xs" />]} />
              </div>
            )}
            <div className="grid gap-4 md:grid-cols-5">
              {COVERAGE_BUCKET_ORDER.map((name) => {
                const bucket = buckets?.[name];
                const amount = bucket?.amount ?? 0;
                const txCount = bucket?.tx_count ?? 0;
                const percent = denominator > 0 ? (amount / denominator) * 100 : 0;
                return (
                  <div key={name} className="space-y-1">
                    <div className="text-xs uppercase tracking-wide opacity-70">
                      {t(`coverageBucket.${name}`)}
                    </div>
                    <div className={`text-lg font-semibold ${COVERAGE_BUCKET_TONES[name]}`}>
                      {amount.toFixed(8)}
                    </div>
                    <div className="text-xs opacity-80">
                      {txCount} tx · {percent.toFixed(1)}%
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}
    </section>
  );
}


export function RecipientPicker({
  recipients,
  selectedRecipientId,
  onSelectRecipient,
}: {
  recipients: SourceFundsRecipient[];
  selectedRecipientId: string;
  onSelectRecipient: (recipient: SourceFundsRecipient | null) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  if (recipients.length === 0) {
    return (
      <div className="rounded-md border bg-muted/30 px-3 py-3 text-sm text-muted-foreground">
        {t("case.optionalRecipient")}
      </div>
    );
  }
  const selected = recipients.find((r) => r.id === selectedRecipientId) ?? null;
  return (
    <div className="rounded-md border px-3 py-3 text-sm">
      <div className="mb-1 font-medium">{t("recipient.ariaLabel")}</div>
      <div className="mb-2 text-xs text-muted-foreground">
        {t("recipient.hint")}</div>
      <Select
        value={selectedRecipientId || NO_RECIPIENT}
        onValueChange={(value) => {
          if (value === NO_RECIPIENT) {
            onSelectRecipient(null);
            return;
          }
          const next = recipients.find((r) => r.id === value) ?? null;
          if (next && next.active === false) return;
          onSelectRecipient(next);
        }}
      >
        <SelectTrigger className="h-9 w-full" aria-label={t("recipient.ariaLabel")}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NO_RECIPIENT}>{t("recipient.none")}</SelectItem>
          {recipients.map((recipient) => {
            const inactive = recipient.active === false;
            return (
              <SelectItem key={recipient.id} value={recipient.id} disabled={inactive}>
                {recipient.label} · {t(`reveal.${recipient.default_reveal_mode}`, { defaultValue: recipient.default_reveal_mode })}
                {inactive ? t("recipient.inactiveSuffix") : ""}
              </SelectItem>
            );
          })}
        </SelectContent>
      </Select>
      {selected && selected.notes && (
        <div className="mt-2 text-xs opacity-80">{selected.notes}</div>
      )}
    </div>
  );
}


export function RecipientPreferenceAdvisory({
  recipient,
  currentRevealMode,
  onApply,
}: {
  recipient: SourceFundsRecipient | null;
  currentRevealMode: string;
  onApply: (mode: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  if (!recipient) return null;
  const preferred = recipient.default_reveal_mode;
  if (!preferred || preferred === currentRevealMode) return null;
  return (
    <div className="flex items-center justify-between gap-3 rounded-md border bg-muted/30 px-3 py-2 text-xs">
      <span className="text-muted-foreground">
        <Trans t={t} i18nKey="recipient.advisory" values={{ label: recipient.label, preferred: t(`reveal.${preferred}`, { defaultValue: preferred }), current: t(`reveal.${currentRevealMode}`, { defaultValue: currentRevealMode }) }} components={[<span className="font-medium text-foreground" />, <span className="font-medium text-foreground" />]} />
      </span>
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={() => onApply(preferred)}
      >
        {t("recipient.applyPreference")}</Button>
    </div>
  );
}


export function GateRow({
  finding,
  onOpenTransaction,
  onAction,
}: {
  finding: SourceFundsFinding;
  onOpenTransaction?: () => void;
  /** Dispatches the finding's next_step.action; gap cards become one-click fixes. */
  onAction?: (action: string, finding: SourceFundsFinding) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const copy = findingTranslationKeys(finding);
  const blocker = finding.severity === "blocker";
  const rawHeadline = finding.next_step?.headline?.trim();
  const headline = copy.nextStep ? t(copy.nextStep, { defaultValue: rawHeadline ?? "" }) : rawHeadline;
  const docAnchor = finding.next_step?.doc_anchor?.trim();
  const action = finding.next_step?.action?.trim();
  const actionLabel = action && ["open_source_creator", "open_link_review", "open_review_queue", "open_source", "open_transaction"].includes(action) ? t(`gapAction.${action}`, { defaultValue: action }) : undefined;
  return (
    <div
      className={[
        "rounded-md border px-3 py-2 text-sm",
        blocker
          ? "border-amber-200 bg-amber-50 text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100"
          : "",
      ].join(" ")}
    >
      <div className="font-medium">{copy.title ? t(copy.title, { defaultValue: pretty(finding.code) }) : pretty(finding.code)}</div>
      <div className="mt-1 text-xs opacity-80">{copy.message ? t(copy.message, { defaultValue: finding.message }) : finding.message}</div>
      {headline && (
        <div className="mt-2 text-xs font-medium opacity-90">
          {t("gates.nextStep", { headline })}
          {docAnchor && (
            <span className="ml-1 opacity-70">{t("gates.seeDocs", { anchor: docAnchor })}</span>
          )}
        </div>
      )}
      <div className="flex flex-wrap gap-3">
        {onAction && action && actionLabel && (
          <button
            type="button"
            className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-[var(--kb-accent)] hover:underline"
            onClick={() => onAction(action, finding)}
          >
            <ArrowRight className="size-3.5" aria-hidden="true" />
            {actionLabel}
          </button>
        )}
        {onOpenTransaction && (
          <button
            type="button"
            className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-[var(--kb-accent)] hover:underline"
            onClick={onOpenTransaction}
          >
            <Eye className="size-3.5" aria-hidden="true" />
            {t("workstation.openTransactionToFix")}</button>
        )}
      </div>
    </div>
  );
}


export function EmptyState({ text }: { text: string }) {
  return (
    <div className="rounded-md border px-3 py-6 text-center text-sm text-muted-foreground">
      {text}
    </div>
  );
}


export function DisclosureNodeOverrides({
  report,
  overrides,
  onChange,
}: {
  report?: SourceFundsPreview;
  overrides: Record<string, "show" | "hide">;
  onChange: (id: string, decision: "show" | "hide" | undefined) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const nodes = (report?.graph.nodes ?? []).filter(
    (node) => stringValue(node.node_type) === "transaction",
  );
  if (nodes.length === 0) {
    return null;
  }
  const buttonClass = (active: boolean, tone: "show" | "hide") =>
    [
      "rounded border px-2 py-0.5 text-xs transition-colors",
      active
        ? tone === "show"
          ? "border-emerald-500 bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300"
          : "border-rose-500 bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300"
        : "text-muted-foreground hover:bg-muted/50",
    ].join(" ");
  return (
    <div className="space-y-1">
      <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {t("workstation.pertransactionDisclosure")}</div>
      <p className="text-xs text-muted-foreground">
        {t("workstation.overrideTheRevealModeFor")}</p>
      <div className="space-y-1">
        {nodes.map((node) => {
          const id = stringValue(node.transaction_id);
          if (!id) {
            return null;
          }
          const external = stringValue(node.external_id);
          const decision = overrides[id];
          return (
            <div
              key={id}
              className="flex items-center justify-between gap-2 rounded-md border px-2 py-1"
            >
              <div className="min-w-0">
                <div className="truncate text-xs">
                  {stringValue(node.label) || id}
                </div>
                <div className="truncate font-mono text-2xs text-muted-foreground">
                  {external ? shortId(external) : "(redacted)"}
                </div>
              </div>
              <div className="flex shrink-0 gap-1">
                <button
                  type="button"
                  className={buttonClass(decision === "show", "show")}
                  onClick={() =>
                    onChange(id, decision === "show" ? undefined : "show")
                  }
                >
                  {t("workstation.show")}</button>
                <button
                  type="button"
                  className={buttonClass(decision === "hide", "hide")}
                  onClick={() =>
                    onChange(id, decision === "hide" ? undefined : "hide")
                  }
                >
                  {t("workstation.hide")}</button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}


export function FlowLevelDetailPreview({
  report,
  omitted,
}: {
  report?: SourceFundsPreview;
  omitted: boolean;
}) {
  const { t } = useTranslation("sourceFunds");
  const levels = report?.flow_levels ?? [];
  if (levels.length === 0) return null;
  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{t("workstation.transactionDetailsByLevel")}</h3>
        {omitted && (
          <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
            {t("case.omitted")}
          </span>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("workstation.level1IsTheReport")}</p>
      <div className={omitted ? "space-y-3 opacity-50" : "space-y-3"}>
        {levels.map((level) => (
          <div key={level.level} className="overflow-hidden rounded-md border">
            <div className="flex items-center justify-between border-b bg-muted/40 px-3 py-1.5 text-xs font-medium">
              <span>
                {t("case.levelSummary", { level: level.level, transactions: level.transaction_count, sources: level.source_count })}
              </span>
              {level.fiat_value_total != null && (
                <span className="font-mono tabular-nums">
                  {formatFiatAmount(
                    level.fiat_value_total,
                    level.fiat_currency || "EUR",
                  )}
                </span>
              )}
            </div>
            {level.nodes.map((node) => {
              const isSource = node.node_type === "source";
              const amount = node.required_amount ?? node.amount ?? null;
              const inbound = isSource || node.direction === "inbound";
              const provenance = isSource
                ? "attested"
                : PROVENANCE_SHORT_LABELS[node.data_provenance ?? ""] ?? "";
              return (
                <div
                  key={node.id}
                  className="flex flex-wrap items-center gap-x-3 gap-y-0.5 border-b px-3 py-1.5 text-xs last:border-b-0 sm:grid sm:grid-cols-[110px_1fr_90px_120px_70px]"
                >
                  <span className="text-muted-foreground">
                    {formatDateTime(node.occurred_at || node.acquired_at)}
                  </span>
                  <span className="min-w-0 truncate font-medium">
                    {isSource ? node.label : node.wallet || node.label}
                  </span>
                  <span className="text-muted-foreground">
                    {isSource
                      ? pretty(node.source_type ?? "source")
                      : pretty(node.direction ?? "")}
                  </span>
                  <span className="text-right font-mono tabular-nums">
                    {amount == null
                      ? "—"
                      : `${inbound ? "+" : "−"}${formatBtc(amount, node.asset || "BTC")}`}
                  </span>
                  <span className="text-right text-muted-foreground">
                    {provenance}
                  </span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </section>
  );
}


export function DisclosureNarrative({ report }: { report?: SourceFundsPreview }) {
  const { t } = useTranslation("sourceFunds");
  const txidCount = report?.disclosure_preview.txids.length ?? 0;
  const evidenceCount = report?.disclosure_preview.attachments.length ?? 0;
  const hiddenCount = report?.disclosure_preview.excluded.length ?? 0;
  const sourceCount = report?.source_mix.length ?? 0;
    const reviewedLinkCount = report?.graph.edges.length ?? 0;
  const walletLabels =
    report?.disclosure_preview.wallets_named ??
    uniqueSorted(
      (report?.graph.nodes ?? [])
        .map((node) => stringValue(node.wallet))
        .filter(Boolean),
    );
  const targetLabel = report?.target.label || t("disclosure.narrativeTarget");
  const purposeLabel = report?.purpose?.label || t("disclosure.narrativePurpose");
  const revealMode = t(`reveal.${report?.reveal_mode || "standard"}`, { defaultValue: report?.reveal_mode || "standard" });

  return (
    <section className="space-y-3 rounded-md border bg-muted/20 p-4">
      <div className="space-y-1">
        <h2 className="text-base font-semibold">{t("disclosure.summaryTitle")}</h2>
        <p className="text-sm text-muted-foreground">
          {t("disclosure.narrative", { purpose: purposeLabel, target: targetLabel, txids: t("disclosure.txidCount", { count: txidCount }), evidence: t("disclosure.evidenceCount", { count: evidenceCount }), links: t("disclosure.linkCount", { count: reviewedLinkCount }), sources: t("disclosure.sourceCount", { count: sourceCount }) })}
        </p>
      </div>

      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
        <DisclosureMetric label={t("disclosure.txids")} value={txidCount} />
        <DisclosureMetric label={t("evidence.label")} value={evidenceCount} />
        <DisclosureMetric label={t("caseBrief.metric.reviewedLinks")} value={reviewedLinkCount} />
        <DisclosureMetric label={t("caseBrief.metric.sources")} value={sourceCount} />
        <DisclosureMetric label={t("disclosure.metric.hidden")} value={hiddenCount} />
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="text-xs font-medium text-muted-foreground">
            {t("disclosure.revealMode")}</div>
          <div className="mt-1 font-medium">{revealMode}</div>
          <p className="mt-1 text-xs text-muted-foreground">
            {report?.disclosure_preview.privacy_note ||
              t("disclosure.noPreview")}
          </p>
        </div>
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="text-xs font-medium text-muted-foreground">
            {t("disclosure.walletLabels")}</div>
          <div className="mt-1 text-sm">
            {walletLabels.length > 0 ? walletLabels.join(", ") : t("disclosure.none")}
          </div>
          {report?.disclosure_preview.ownership_note && (
            <p className="mt-1 text-xs text-amber-600 dark:text-amber-500">
              {report.disclosure_preview.ownership_note}
            </p>
          )}
          <p className="mt-1 text-xs text-muted-foreground">
            {t("disclosure.walletPrivacyNote")}</p>
        </div>
      </div>
    </section>
  );
}


export function DisclosureMetric({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border bg-background px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 font-mono text-lg font-semibold tabular-nums">
        {formatCount(value)}
      </div>
    </div>
  );
}


export function DisclosureTxidList({ report }: { report?: SourceFundsPreview }) {
  const { t } = useTranslation("sourceFunds");
  const [openingTxid, setOpeningTxid] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const txids = report?.disclosure_preview.txids ?? [];
  const links = useMemo(
    () =>
      new Map(
        (report?.disclosure_preview.explorer_links ?? []).map((link) => [
          link.txid,
          link,
        ]),
      ),
    [report?.disclosure_preview.explorer_links],
  );
  const onOpen = async (txid: string, url: string) => {
    setOpenError(null);
    setOpeningTxid(txid);
    try {
      await openExternalUrl(url);
    } catch (error) {
      setOpenError(
        error instanceof Error && error.message
          ? error.message
          : t("disclosure.openError"),
      );
    } finally {
      setOpeningTxid(null);
    }
  };
  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold">{t("disclosure.txids")}</h2>
      <div className="space-y-1">
        {txids.length === 0 ? (
          <div className="rounded-md border px-3 py-2 text-muted-foreground">
            {t("disclosure.none")}</div>
        ) : (
          txids.map((txid) => {
            const link = links.get(txid);
            return (
              <div
                key={txid}
                className="flex flex-col gap-2 rounded-md border px-3 py-2 text-xs sm:flex-row sm:items-center sm:justify-between"
              >
                <span className="break-all font-mono">{txid}</span>
                {link ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 shrink-0"
                    disabled={openingTxid === txid}
                    onClick={() => void onOpen(txid, link.url)}
                    title={t("disclosure.openTitle", { txid, label: link.label })}
                  >
                    <ExternalLink className="mr-2 size-3.5" aria-hidden="true" />
                    {openingTxid === txid ? t("disclosure.opening") : link.label}
                  </Button>
                ) : (
                  <span className="text-muted-foreground">
                    {t("disclosure.noExplorerLink")}</span>
                )}
              </div>
            );
          })
        )}
      </div>
      {openError && (
        <p
          role="alert"
          className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          {openError}
        </p>
      )}
    </section>
  );
}


export function DisclosureList({ label, values }: { label: string; values: string[] }) {
  const { t } = useTranslation("sourceFunds");
  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold">{label}</h2>
      <div className="space-y-1">
        {values.length === 0 ? (
          <div className="rounded-md border px-3 py-2 text-muted-foreground">
            {t("disclosure.none")}</div>
        ) : (
          values.map((value) => (
            <div
              key={value}
              className="break-all rounded-md border px-3 py-2 font-mono text-xs"
            >
              {value}
            </div>
          ))
        )}
      </div>
    </section>
  );
}
