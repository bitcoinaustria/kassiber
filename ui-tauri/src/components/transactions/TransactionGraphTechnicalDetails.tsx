import { useTranslation } from "react-i18next";
import { LedgerRow } from "./TransactionDetailSheetParts";
import type { TransactionGraphPayload } from "./TransactionGraphModel";
import { formatBtcAmount, SATS_PER_BTC } from "./model";

/** Only observed facts belong here; an absent graph has no zero-input count. */
export function TransactionGraphTechnicalDetails({ graph, hideSensitive }: {
  graph?: TransactionGraphPayload;
  hideSensitive: boolean;
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
  if (!fields.length) return null;
  return (
    <div className="overflow-hidden rounded-md border">
      <div className="border-b bg-muted px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        {t("details.technical")}
      </div>
      <div className="grid sm:grid-cols-2">
        {fields.map(field => <LedgerRow key={field.label} label={field.label}
          value={field.sensitive && hideSensitive ? t("graph.hidden")
            : field.format ? field.format(field.value as number)
            : field.unit ? `${field.value} ${field.unit}` : field.value} />)}
      </div>
    </div>
  );
}
