import { useTranslation } from "react-i18next";
import { ArrowRight, ArrowUpRight, CheckCircle2, HelpCircle } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { formatCustodyMsat } from "@/routes/custodyGapsModel";
import { reviewRecord, reviewText, type ReviewArtifact, type ReviewEffects, type ReviewReceipt, type ReviewRecord } from "./reviewWorkflow";

function RawReview({ value }: { value: unknown }) {
  const { t } = useTranslation("assistant");
  return <details className="text-xs text-muted-foreground">
    <summary className="cursor-pointer py-2">{t("review.rawJson")}</summary>
    <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted p-3">{JSON.stringify(value, null, 2)}</pre>
  </details>;
}

export function ReviewEffectsView({ before, after }: { before?: ReviewEffects; after: ReviewEffects }) {
  const { t } = useTranslation("assistant");
  const rows = [
    { label: t("review.effects.quarantine"), before: before?.quarantine_count, after: after.quarantine_count },
    { label: t("review.effects.readiness"), before: before && t(before.report_ready ? "review.effects.ready" : "review.effects.blocked"), after: t(after.report_ready ? "review.effects.ready" : "review.effects.blocked") },
  ];
  return <section className="space-y-2" aria-label={t("review.computed")}>
    <h4 className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground"><CheckCircle2 className="size-3.5" aria-hidden="true" />{t("review.computed")}</h4>
    <dl className="space-y-2">{rows.map((row) => <div key={row.label} className="flex flex-wrap items-baseline justify-between gap-3">
      <dt className="text-xs text-muted-foreground">{row.label}</dt>
      <dd className="flex items-center gap-2 font-medium tabular-nums">{before ? <><span>{row.before}</span><ArrowRight className="size-3.5 text-muted-foreground" aria-hidden="true" /><span className="sr-only">{t("review.effects.becomes")}</span></> : null}<span>{row.after}</span></dd>
    </div>)}</dl>
  </section>;
}

function CustodyRequest({ request }: { request: ReviewRecord | null }) {
  const { t } = useTranslation("assistant");
  if (!request) return null;
  const components = Array.isArray(request.components) ? request.components : request.spec ? [request.spec] : [];
  return <div className="space-y-2 text-xs">
    <p>{reviewText(request.action)}{typeof request.activate === "boolean" ? ` · ${t(request.activate ? "review.op.activate" : "review.op.draft")}` : ""}</p>
    {components.map((value, index) => {
      const component = reviewRecord(value);
      const legs = Array.isArray(component?.legs) ? component.legs : [];
      return <div key={index} className="space-y-1 border-l pl-2">
        <p>{reviewText(component?.component_type)}{component?.notes ? ` · ${reviewText(component.notes)}` : ""}</p>
        {legs.map((value, legIndex) => {
          const leg = reviewRecord(value);
          if (!leg) return null;
          const asset = reviewText(leg.asset);
          const unresolvedAsset = t("review.op.assetFromReference");
          const amount = leg.amount_btc !== undefined ? `${reviewText(leg.amount_btc)} ${asset || `(${unresolvedAsset})`}`
            : leg.amount_msat !== undefined ? asset ? formatCustodyMsat(reviewText(leg.amount_msat), asset)
              : `${reviewText(leg.amount_msat)} msat (${unresolvedAsset})` : "—";
          return <p key={legIndex} className="break-words"><span>{reviewText(leg.role)}</span>{" · "}<span className="font-mono">{amount}</span>{" · "}{reviewText(leg.transaction ?? leg.transaction_id ?? leg.wallet ?? leg.wallet_id ?? leg.untracked_wallet)}{leg.occurred_at ? ` · ${reviewText(leg.occurred_at)}` : ""}</p>;
        })}
      </div>;
    })}
  </div>;
}

function ReviewOperation({ operation }: { operation: ReviewRecord }) {
  const { t } = useTranslation("assistant");
  const title = operation.type === "price_override" ? t("review.op.price_override")
    : operation.type === "exclude" ? t("review.op.exclude") : t("review.op.custody_component");
  const request = reviewRecord(operation.request);
  const reason = reviewText(operation.reason ?? request?.reason);
  return <li className="space-y-1 border-b py-2 last:border-0">
    <p className="text-sm font-medium">{title}</p>
    {operation.transaction_id ? <p className="break-all font-mono text-xs">{reviewText(operation.transaction_id)}</p> : null}
    {operation.fiat_rate !== undefined ? <p className="text-xs">{t("review.op.rate")}: <span className="font-mono">{reviewText(operation.fiat_rate)}</span></p> : null}
    {operation.fiat_value !== undefined ? <p className="text-xs">{t("review.op.value")}: <span className="font-mono">{reviewText(operation.fiat_value)}</span></p> : null}
    {operation.type === "custody_component" ? <CustodyRequest request={request} /> : null}
    {reason ? <p className="whitespace-pre-wrap text-xs text-muted-foreground">{t("review.proposedInterpretation")}: {reason}</p> : null}
  </li>;
}

function ReviewOperations({ operations }: { operations: ReviewRecord[] }) {
  const { t } = useTranslation("assistant");
  return <section aria-label={t("review.operations", { count: operations.length })}>
    <h4 className="text-xs font-medium text-muted-foreground">{t("review.operations", { count: operations.length })}</h4>
    <ol>{operations.slice(0, 6).map((operation, index) => <ReviewOperation key={index} operation={operation} />)}</ol>
    {operations.length > 6 ? <details className="text-xs"><summary className="cursor-pointer py-2">{t("review.more", { count: operations.length - 6 })}</summary><ol start={7}>{operations.slice(6).map((operation, index) => <ReviewOperation key={index} operation={operation} />)}</ol></details> : null}
  </section>;
}

function RemainingIssues({ effects }: { effects: ReviewEffects }) {
  const { t } = useTranslation("assistant");
  if (effects.quarantine_count === 0) return null;
  return <section className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs" aria-label={t("review.openQuestions")}>
    <p className="flex items-center gap-1.5 font-medium"><HelpCircle className="size-3.5" aria-hidden="true" />{t("review.remaining", { count: effects.quarantine_count })}</p>
    <details className="mt-1"><summary className="cursor-pointer py-1">{t("review.openQuestions")}</summary><ul className="space-y-1 break-words">{effects.quarantines.slice(0, 100).map((value, index) => { const issue = reviewRecord(value); return <li key={index}>{reviewText(issue?.transaction_id)} · {reviewText(issue?.reason)}</li>; })}</ul>{effects.quarantine_count > effects.quarantines.length ? <p className="mt-2">{t("review.notListed", { count: effects.quarantine_count - effects.quarantines.length })}</p> : null}</details>
  </section>;
}

export function ReviewProposalView({ artifact }: { artifact: ReviewArtifact }) {
  const { t } = useTranslation("assistant");
  return <div className="space-y-4">
    <p className="text-xs text-muted-foreground">{t("review.proposal.revision", { revision: artifact.base_input_version })}</p>
    <div className="grid gap-5 sm:grid-cols-2"><ReviewEffectsView before={artifact.before} after={artifact.after} /><ReviewOperations operations={artifact.operations} /></div>
    <RemainingIssues effects={artifact.after} />
    <p className="break-all font-mono text-2xs text-muted-foreground">{t("review.fingerprint")}: {artifact.digest}</p>
    <RawReview value={artifact} />
  </div>;
}

export function ReviewProposalCard({ artifact, applied = false }: { artifact: ReviewArtifact; applied?: boolean }) {
  const { t } = useTranslation("assistant");
  return <section className="my-3 space-y-3 rounded-lg border border-primary/40 bg-card p-4" aria-label={t("review.proposal.title")}>
    <h3 className="font-medium">{t("review.proposal.title")} <span className="ml-2 text-xs font-normal text-muted-foreground">{t(applied ? "review.status.applied" : "review.status.proposed")}</span></h3>
    <ReviewProposalView artifact={artifact} />
  </section>;
}

export function ReviewReceiptCard({ receipt }: { receipt: ReviewReceipt }) {
  const { t } = useTranslation("assistant");
  return <section className="my-3 space-y-3 rounded-lg border bg-card p-4" aria-label={t("review.receipt.title")}>
    <h3 className="font-medium">{t("review.receipt.title")}</h3>
    <p className="text-xs text-muted-foreground">{t("review.receipt.verified", { count: receipt.operations.length })}</p>
    <p className="break-all text-xs text-muted-foreground"><time dateTime={receipt.created_at}>{receipt.created_at}</time> · {receipt.id}</p>
    <ReviewEffectsView before={receipt.before} after={receipt.verification} />
    <RemainingIssues effects={receipt.verification} />
    <Button asChild variant="ghost" size="sm"><Link to="/quarantine"><ArrowUpRight className="size-3.5" aria-hidden="true" />{t("review.receipt.openQuarantine")}</Link></Button>
    <RawReview value={receipt} />
  </section>;
}
