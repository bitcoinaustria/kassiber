import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

type Evidence = { status: "observed" | "unavailable" | "retracted"; confirmed?: boolean; block_height?: number | null; block_hash?: string | null };
type Gains = { fiat_currency?: string; gain_loss_exact?: string };
export type ReportImpact = {
  snapshot_id: string;
  report_state: "saved" | "filed";
  period_start_year: number;
  period_end_year: number;
  content_sha256: string;
  transaction_id: string;
  transaction_available: boolean;
  before_gain_summary: Gains;
  resolution: { amendment_status: "no_change" | "saved_report_changed" | "review_required"; after_gain_summary: Gains } | null;
};

/** Historical report identity and exact arithmetic stay visible after rebuild. */
export function ReportImpactDetails({ impact, observation }: {
  impact: ReportImpact;
  observation: { before: Evidence; after: Evidence };
}) {
  const { t } = useTranslation("chainAnalysis");
  const evidence = (value: Evidence) => value.status === "observed"
    ? t(value.confirmed ? "watch.report.confirmed" : "watch.report.unconfirmed", { height: value.block_height ?? "?" })
    : t(`watch.report.${value.status}`);
  return <details className="w-full rounded border p-3">
    <summary className="cursor-pointer font-medium">{t("watch.report.title", { start: impact.period_start_year, end: impact.period_end_year })} · {t(`watch.report.${impact.report_state}`)}</summary>
    <div className="mt-2 space-y-2">
      <p>{t("watch.report.evidence", { before: evidence(observation.before), after: evidence(observation.after) })}</p>
      {(observation.before.block_hash || observation.after.block_hash) && <p className="break-all">{t("watch.report.blocks")} <code>{observation.before.block_hash ?? "?"} → {observation.after.block_hash ?? "?"}</code></p>}
      <p>{t("watch.report.identity")} <code className="break-all">{impact.snapshot_id}</code></p>
      <p className="break-all font-mono">SHA-256: {impact.content_sha256}</p>
      <p>{t("watch.report.gainBefore", { value: impact.before_gain_summary.gain_loss_exact ?? "?", currency: impact.before_gain_summary.fiat_currency ?? "" })}</p>
      {impact.resolution ? <>
        <p>{t("watch.report.gainAfter", { value: impact.resolution.after_gain_summary.gain_loss_exact ?? "?", currency: impact.resolution.after_gain_summary.fiat_currency ?? "" })}</p>
        <p>{t(`watch.report.${impact.resolution.amendment_status}`)}</p>
      </> : <p>{t("watch.report.pending")}</p>}
      <p>{t("watch.report.filingAuthority")}</p>
      <div className="flex flex-wrap gap-3">
        {impact.transaction_available && <Link to="/transactions" search={{ tx: impact.transaction_id }} className="underline">{t("watch.report.transaction")}</Link>}
        <Link to="/journals" className="underline">{t("watch.report.journals")}</Link>
        <Link to="/reports" className="underline">{t("watch.report.reports")}</Link>
      </div>
    </div>
  </details>;
}
