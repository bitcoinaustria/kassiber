import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { LedgerRow } from "./TransactionDetailSheetParts";
import type { TransactionGraphPayload } from "./TransactionGraphModel";
import { formatBtcAmount, SATS_PER_BTC } from "./model";

/**
 * Only observed facts belong here; an absent graph has no zero-input count.
 * Folded to one summary line by default: they are for checking, not reading.
 */
export function TransactionGraphTechnicalDetails({ graph, hideSensitive, action }: {
  graph?: TransactionGraphPayload;
  hideSensitive: boolean;
  /** Shown at the end of the summary line, e.g. a link to chain analysis. */
  action?: ReactNode;
}) {
  const { t } = useTranslation("transactions");
  const tx = graph?.transaction;
  if (!tx) return null;
  const known = (value: unknown): value is number =>
    typeof value === "number" && Number.isFinite(value);
  const fee = known(graph?.fee?.valueBtc) ? graph.fee.valueBtc
    : known(graph?.fee?.valueSats) ? graph.fee.valueSats / SATS_PER_BTC : null;
  const hasGraph = graph?.supportLevel === "full" || graph?.supportLevel === "partial";
  const fields = [
    { label: t("details.inputCount"), value: hasGraph ? tx.inputCount : null },
    { label: t("details.outputCount"), value: hasGraph ? tx.outputCount : null },
    { label: t("details.networkFee"), value: fee, sensitive: true, format: formatBtcAmount },
    { label: t("details.feeRate"), value: tx.feeRateSatVb, sensitive: true, unit: "sat/vB" },
    { label: t("details.version"), value: tx.version },
    { label: t("details.locktime"), value: tx.locktime },
    { label: t("details.size"), value: tx.size, unit: "B" },
    { label: t("details.vsize"), value: tx.vsize, unit: "vB" },
    { label: t("details.weight"), value: tx.weight, unit: "WU" },
  ].filter(field => known(field.value));
  if (!fields.length) return action ? <div className="flex justify-end">{action}</div> : null;
  const shown = (field: (typeof fields)[number]) =>
    field.sensitive && hideSensitive ? t("graph.hidden")
      : field.format ? field.format(field.value as number)
      : field.unit ? `${field.value} ${field.unit}` : String(field.value);
  // The summary names what is known; the table below has every field.
  const summary = fields
    .filter((field) => !field.sensitive || !hideSensitive)
    .slice(0, 5)
    .map((field) => `${field.label} ${shown(field)}`)
    .join(" · ");
  return (
    <details className="group kb-surface-inset overflow-hidden">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs text-muted-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight className="size-3.5 shrink-0 transition-transform group-open:rotate-90" aria-hidden="true" />
        <span className="shrink-0 font-medium text-foreground">{t("details.technical")}</span>
        <span className="min-w-0 truncate">{summary}</span>
        {action ? <span className="ml-auto shrink-0" onClick={(event) => event.stopPropagation()}>{action}</span> : null}
      </summary>
      <div className="grid border-t sm:grid-cols-2">
        {fields.map(field => <LedgerRow key={field.label} label={field.label} value={shown(field)} />)}
      </div>
    </details>
  );
}
