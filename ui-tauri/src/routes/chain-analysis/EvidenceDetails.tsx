import { useTranslation } from "react-i18next";
import { readableAnalysisValue } from "@/lib/chainAnalysis";

export function EvidenceDetails({
  value,
  label,
}: {
  value: unknown;
  label?: string;
}) {
  const { t } = useTranslation("chainAnalysis");
  return (
    <details className="ca-evidence">
      <summary>{label || t("showEvidence")}</summary>
      <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap break-all font-mono text-[11px]">
        {readableAnalysisValue(value)}
      </pre>
    </details>
  );
}

export function Fact({ label, value }: { label: string; value: unknown }) {
  if (
    value == null ||
    value === "" ||
    (Array.isArray(value) && value.length === 0)
  )
    return null;
  return (
    <div className="min-w-0">
      <dt className="ca-detail-key">{label}</dt>
      <dd className="mt-1 select-text break-all font-mono text-xs leading-relaxed">
        {readableAnalysisValue(value)}
      </dd>
    </div>
  );
}
