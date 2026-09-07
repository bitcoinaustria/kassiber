import { useTranslation } from "react-i18next";
import { FeatureDetails, StructuredValue } from "./FeatureDetails";
import type { FeatureSnapshot } from "@/lib/chainAnalysisWorkbench";
import { Fact } from "./EvidenceDetails";

function FeatureChanges({ rows }: { rows: unknown[] }) {
  const { t } = useTranslation("chainAnalysis");
  return <div className="space-y-4">{rows.slice(0, 32).map((value, index) => {
    if (!value || typeof value !== "object") return null;
    const row = value as Record<string, unknown>;
    const snapshots = row.before || row.after ? [row.before, row.after] : [row];
    return <div key={index} className="space-y-3"><Fact label={t("id")} value={row.id || row.subject} /><div className="grid gap-3 sm:grid-cols-2">{snapshots.map((item, side) => {
      const features = item && typeof item === "object" ? (item as { features?: FeatureSnapshot }).features : undefined;
      return <div key={side}>{snapshots.length > 1 && <h4 className="mb-2 text-xs text-muted-foreground">{t(side === 0 ? "workbench.before" : "workbench.after")}</h4>}{features && Array.isArray(features.features) ? <FeatureDetails snapshot={features} /> : <StructuredValue value={item} />}</div>;
    })}</div></div>;
  })}{rows.length > 32 && <p className="text-xs text-muted-foreground">{t("workbench.firstRows", { count: 32, total: rows.length })}</p>}</div>;
}

export function CaseComparison({ comparison }: { comparison: Record<string, unknown> }) {
  const { t } = useTranslation("chainAnalysis");
  const kinds = ["nodes", "edges", "findings", "clusters", "patterns", "exposure", "transaction_features"] as const;
  return <section className="mt-3 space-y-3 rounded-lg border bg-muted/15 p-3">
    <p className="text-sm">
      {comparison.changed ? t("case.changed") : t("case.unchanged")}
    </p>
    <div className="space-y-2">
      {kinds.map(kind => {
      const groups = (["added", "removed", "changed"] as const).map(change => ({
          change,
          rows: Array.isArray(comparison[`${change}_${kind}`]) ? comparison[`${change}_${kind}`] as unknown[] : []
        }));
      if (!groups.some(g => g.rows.length))
        return null;
      return <details key={kind} className="rounded border p-3">
          <summary className="cursor-pointer text-xs font-medium">
            {kind === "transaction_features" ? t("workbench.features") : t(kind)} · {groups.map(g => t(`workbench.${g.change}`, { count: g.rows.length })).join(" · ")}
          </summary>
          <div className="mt-3 space-y-3 text-xs">
            {groups.filter(g => g.rows.length).map(group => <div key={group.change}>
              <h4 className="mb-2 font-medium">
                {t(`workbench.${group.change}`, { count: group.rows.length })}
              </h4>
              {kind === "transaction_features" ? <FeatureChanges rows={group.rows} /> : <StructuredValue value={group.rows} />}
            </div>)}
          </div>
        </details>;
    })}
    </div>
    {!!comparison.coverage_changed && <details className="rounded border p-3">
      <summary className="cursor-pointer text-xs font-medium">
        {t("coverage")}
      </summary>
      <div className="mt-3 text-xs">
        <StructuredValue value={comparison.coverage_changed} />
      </div>
    </details>}
  </section>;
}
