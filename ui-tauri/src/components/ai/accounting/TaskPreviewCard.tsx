import { useTranslation } from "react-i18next";
import { useUiStore } from "@/store/ui";
import { formatMinor, type Book } from "./model";
import { isBitcoinTaskProposal, isTaskEntry, isTaskPreview, isTaskReports, type TaskEntry, type TaskPreview } from "./taskModel";
import { FinancialReports } from "./FinancialReports";

type MoneyBook = Pick<Book, "currency" | "minor_unit_exponent">;
function object(value: unknown): Record<string, unknown> { return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {}; }
function scalar(value: unknown) { return typeof value === "string" || typeof value === "number" || typeof value === "boolean" ? String(value) : "—"; }

export function TaskPreviewCard({ value, book }: { value: TaskPreview; book: MoneyBook }) {
  const { t } = useTranslation("accountingReview");
  const hidden = useUiStore((state) => state.hideSensitive);
  if (hidden) return <p>{t("hidden")}</p>;
  if (!isTaskPreview(value)) return <p role="alert">{t("error")}</p>;
  const detail = object(value.detail);
  const entries = Array.isArray(detail.entries) ? detail.entries.filter(isTaskEntry) : [];
  const projections = value.step === "post" && Array.isArray(detail.projections)
    ? detail.projections.filter((row: unknown) => isBitcoinTaskProposal(row, value.period_id, true)) : [];
  const forms = Array.isArray(detail.forms) ? detail.forms : [];
  return <section className="space-y-3 rounded-lg border bg-muted/20 p-4" aria-label={t("preview")}>
    <h4 className="font-medium">{t(`steps.${value.step}`)}</h4><p className="text-sm">{t(value.ready ? "ready" : "blocked")}</p><p className="text-xs text-muted-foreground">{value.period_id} · {t("previewRevision", { revision: value.expected_revision })}</p><code className="block break-all text-xs text-muted-foreground">{value.expected_digest}</code>
    {value.step === "prepare" && <p className="text-xs text-muted-foreground">{t("partial")}</p>}
    <ul className="space-y-1">{value.blockers?.map((row, index) => <li key={index} className="text-sm"><code>{row.kind}{row.code ? ` · ${row.code}` : ""}</code></li>)}</ul>
    {value.proposals?.map((proposal) => proposal.source_kind === "bitcoin"
      ? <ProjectionPreview key={`bitcoin:${proposal.source_id}`} value={proposal} book={book} />
      : isTaskEntry(proposal.payload) && <EntryPreview key={`${proposal.source_kind}:${proposal.source_id}`} entry={proposal.payload} book={book} source={proposal.source_id} />)}
    {projections.map((projection, index) => <ProjectionPreview key={index} value={projection} book={book} />)}
    {entries.map((entry, index) => <EntryPreview key={index} entry={entry} book={book} />)}
    {value.step === "close" && <><p className="text-sm leading-relaxed">{t("closeConsequence")}</p>{isTaskReports(detail) && <FinancialReports reports={detail} money={(amount) => formatMinor(amount, book)} />}</>}
    {value.step === "tax_finalize" && <><p className="text-sm">{t("taxConsequence")}</p>{forms.map((value, index) => {
      const form = object(value);
      return <section key={index} className="space-y-2 rounded border p-3"><h5 className="font-medium">{scalar(form.label)} · {scalar(form.form_id)}</h5><dl className="space-y-2 text-xs">{Object.entries(object(form.fields)).map(([key, value]) => {
        const field = object(value);
        const amount = typeof field.value_minor === "string" && /^-?\d+$/.test(field.value_minor) ? formatMinor(field.value_minor, book) : scalar(field.value);
        return <div key={key} className="border-t pt-2"><dt>{scalar(field.label)} · {key}</dt><dd className="mt-1 break-words font-mono">{amount} · {scalar(field.state)}</dd>{Boolean(field.reason) && <dd className="mt-1 text-muted-foreground">{scalar(field.reason)}</dd>}</div>;
      })}</dl></section>;
    })}</>}
    {(value.step === "export_close" || value.step === "export_tax") && <div className="space-y-2"><p className="text-sm font-medium">{t("exportConsequence")}</p><dl className="space-y-2 text-xs">{Object.entries(detail).filter(([key]) => ["id", "final_id", "period_id", "revision", "snapshot_digest", "report_digest", "input_digest"].includes(key)).map(([key, value]) => <div key={key}><dt className="text-muted-foreground">{key}</dt><dd className="break-all font-mono">{scalar(value)}</dd></div>)}</dl></div>}
  </section>;
}

function ProjectionPreview({ value, book }: { value: unknown; book: MoneyBook }) {
  // Exact strings and every effect, including zero-fiat custody/rounding. Text
  // only: escape non-ASCII controls too, so evidence cannot spoof review order.
  const text = JSON.stringify({ book, effect: value }, null, 2).replace(/[\u007f-\uffff]/g, (character) => `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`);
  return <article className="rounded border p-3"><pre className="whitespace-pre-wrap break-all font-mono text-xs">{text}</pre></article>;
}

function EntryPreview({ entry, book, source }: { entry: TaskEntry; book: MoneyBook; source?: string }) {
  const { t } = useTranslation("accountingReview");
  return <article className="space-y-2 rounded border p-3 text-sm"><p className="font-medium">{entry.description}</p><p className="break-all text-xs text-muted-foreground">{entry.entry_date}{source ? ` · ${source}` : ""}</p><div className="overflow-x-auto"><table className="w-full min-w-80 text-xs"><thead><tr><th className="text-left">{t("account")}</th><th className="text-right">{t("debit")}</th><th className="text-right">{t("credit")}</th></tr></thead><tbody>{entry.lines.map((line, index) => <tr key={index}><td>{line.account_code}</td><td className="text-right font-mono">{formatMinor(line.debit_minor, book)}</td><td className="text-right font-mono">{formatMinor(line.credit_minor, book)}</td></tr>)}</tbody></table></div></article>;
}
