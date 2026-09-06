import { useTranslation } from "react-i18next";
import type { FeatureSnapshot } from "@/lib/chainAnalysisWorkbench";

/** Structured values stay inspectable without presenting raw payload JSON. */
export function StructuredValue({ value, depth = 0 }: { value: unknown; depth?: number }) {
  const { t } = useTranslation("chainAnalysis");
  if (value == null)
  return <span className="text-muted-foreground">
    {t("workbench.unknown")}
  </span>;
  if (typeof value === "boolean")
  return <span>
    {value ? t("workbench.yes") : t("workbench.no")}
  </span>;
  if (typeof value !== "object")
  return <span className="break-all font-mono">
    {String(value)}
  </span>;
  if (depth > 3)
  return <span>
    {t("workbench.nested")}
  </span>;
  if (Array.isArray(value))
  return <div className="space-y-1">
    {value.length ? value.slice(0, 32).map((item, i) => <div key={i}>
      <StructuredValue value={item} depth={depth + 1} />
    </div>) : <span>—</span>}
    {value.length > 32 && <span>
      {t("workbench.firstRows", {
        count: 32,
        total: value.length
      })}
    </span>}
  </div>;
  return <dl className="grid gap-2">
    {Object.entries(value).map(([key, item]) => <div key={key} className="flex flex-wrap gap-x-3">
      <dt className="text-muted-foreground">
        {key.replace(/_/g, " ")}
      </dt>
      <dd>
        <StructuredValue value={item} depth={depth + 1} />
      </dd>
    </div>)}
  </dl>;
}
export function CodeList({ title, values }: { title: string; values: unknown }) {
  if (!Array.isArray(values) || !values.length)
  return null;
  return <div>
    <h4 className="mb-2 text-xs font-medium">
      {title}
    </h4>
    <ul className="space-y-1 text-[11px] text-muted-foreground">
      {values.map((item, i) => <li key={i} className="break-words">
        {String(item)}
      </li>)}
    </ul>
  </div>;
}
export function FeatureDetails({ snapshot }: { snapshot: FeatureSnapshot }) {
  const { t } = useTranslation("chainAnalysis");
  return <section className="space-y-3">
    <h3 className="text-sm font-medium">
      {t("workbench.features")}
    </h3>
    <p className="text-[10px] text-muted-foreground">
      {snapshot.extractor_version} · {snapshot.source}
    </p>
    <div className="divide-y rounded-lg border">
      {snapshot.features.map(feature => <details key={feature.code} className="p-3">
        <summary className="cursor-pointer text-xs">
          <span className="font-medium">
            {feature.code.replace(/_/g, " ")}
          </span>
          <span className="ml-3 text-muted-foreground">
            {feature.availability}
          </span>
        </summary>
        <div className="mt-3 text-xs">
          <StructuredValue value={feature.value} />
          <CodeList title={t("entropyPanel.assumptions")} values={feature.assumptions} />
        </div>
      </details>)}
    </div>
    <CodeList title={t("entropyPanel.limitations")} values={snapshot.limitations} />
  </section>;
}
