import { useRef, useState } from "react";
import type { ParseKeys } from "i18next";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useDaemonMutation } from "@/daemon/client";
import { bookIdentityKey, useUiStore } from "@/store/ui";
import { transactionKindOptions, UNCLASSIFIED_KIND } from "@/lib/transactionTypeLabel";
import { type ReviewArtifact } from "@/components/ai/reviewWorkflow";
import { ReviewEffectsView } from "@/components/ai/ReviewWorkflowCard";
import { blurClass, type Transaction } from "./model";

import { planAcquisitionReview, type AcquisitionScope } from "./acquisitionReviewModel";
type Totals = { asset: string; acquisition_basis: string; income: string; disposal_basis: string; proceeds: string; gain_loss: string };
const fields = ["acquisition_basis", "income", "disposal_basis", "proceeds", "gain_loss"] as const;

/** Classification has its own reviewed save; the ordinary draft never carries it. */
export function AcquisitionReview({ transaction, dirty, hideSensitive }: { transaction: Transaction; dirty: boolean; hideSensitive: boolean }) {
  const { t } = useTranslation("transactions");
  const session = useUiStore((state) => state.daemonSession);
  const identity = useUiStore((state) => bookIdentityKey(state.identity));
  const scopeKey = `${session}:${identity}:${transaction.id}`;
  const current = useRef(scopeKey);
  current.current = scopeKey;
  const [kind, setKind] = useState(transaction.kindOverride ?? UNCLASSIFIED_KIND);
  const [reviewed, setReviewed] = useState<{ artifact: ReviewArtifact; scope: string; key: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const cases = useDaemonMutation<AcquisitionScope>("ui.review.cases", { invalidateQueries: false });
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", { invalidateQueries: false });
  const apply = useDaemonMutation("ui.review.apply");
  const busy = cases.isPending || plan.isPending || apply.isPending;
  const artifact = reviewed?.scope === scopeKey ? reviewed.artifact : null;
  const changed = kind !== (transaction.kindOverride ?? UNCLASSIFIED_KIND);
  async function preview() {
    setError(null); setReviewed(null);
    const pinned = scopeKey;
    try {
      const validated = await planAcquisitionReview({
        readScope: async () => (await cases.mutateAsync({ limit: 1 })).data,
        createPlan: async (args) => (await plan.mutateAsync(args)).data,
        isCurrent: () => current.current === pinned && useUiStore.getState().daemonSession === session &&
          bookIdentityKey(useUiStore.getState().identity) === identity,
        transactionId: transaction.id, kind: kind === UNCLASSIFIED_KIND ? null : kind,
        invalidMessage: t("tax.acquisition.invalidPreview"),
      });
      if (!validated) return;
      setReviewed({ artifact: validated, scope: pinned, key: crypto.randomUUID() });
    } catch (cause) { if (current.current === pinned) setError(String(cause)); }
  }
  async function confirm() {
    if (!reviewed || !artifact || dirty || useUiStore.getState().daemonSession !== session ||
        bookIdentityKey(useUiStore.getState().identity) !== identity) return;
    const pinned = scopeKey;
    setError(null);
    try {
      await apply.mutateAsync({ expected_scope: { workspace_id: artifact.workspace_id, profile_id: artifact.profile_id },
        artifact, idempotency_key: reviewed.key });
      if (current.current === pinned) setReviewed(null);
    } catch (cause) { if (current.current === pinned) setError(String(cause)); }
  }
  const before = artifact?.before.accounting_totals as Totals[] | undefined;
  const after = artifact?.after.accounting_totals as Totals[] | undefined;
  const currency = (artifact?.before.acquisition_context as { profile_policy?: { fiat_currency?: string } } | undefined)?.profile_policy?.fiat_currency;
  return <section className="space-y-3 rounded-md border bg-background p-3">
    <Label htmlFor="tx-acquisition-kind">{t("tax.kind")}</Label>
    <p className="text-xs text-muted-foreground">{t("tax.acquisition.help")}</p>
    <Select value={kind} disabled={busy || dirty} onValueChange={(value) => { setKind(value); setReviewed(null); setError(null); }}>
      <SelectTrigger id="tx-acquisition-kind"><SelectValue /></SelectTrigger>
      <SelectContent><SelectItem value={UNCLASSIFIED_KIND}>{t("tax.kindUnclassified")}</SelectItem>
        {transactionKindOptions(true).map((option) => <SelectItem key={option.kind} value={option.kind}>{t(option.labelKey as ParseKeys<"transactions">)}</SelectItem>)}
      </SelectContent>
    </Select>
    {dirty ? <p className="text-xs text-muted-foreground">{t("tax.acquisition.dirty")}</p> : null}
    <Button variant="outline" disabled={busy || dirty || !changed} onClick={() => void preview()}>{t("tax.acquisition.preview")}</Button>
    {artifact ? <div className="space-y-3">
      <p className="text-xs text-muted-foreground">{t("tax.acquisition.scope", { version: artifact.base_input_version, currency })}</p>
      <div className={blurClass(hideSensitive)}><ReviewEffectsView before={artifact.before} after={artifact.after} />
      {[...new Set([...(before ?? []).map((row) => row.asset), ...(after ?? []).map((row) => row.asset)])].map((asset) => <div key={asset} className="space-y-1 text-xs">
        <p className="font-medium">{asset} · {currency}</p>
        {fields.map((field) => <div key={field} className="flex justify-between gap-2"><span>{t(`tax.acquisition.${field}`)}</span><span className="font-mono">{before?.find((row) => row.asset === asset)?.[field] ?? "0"} → {after?.find((row) => row.asset === asset)?.[field] ?? "0"}</span></div>)}
      </div>)}</div>
      {!artifact.after.report_ready ? <div className="text-xs text-amber-700"><p>{t("tax.acquisition.blocked")}</p><ul>{artifact.after.quarantines.map((value, index) => <li key={index}><AcquisitionBlocker reason={(value as { reason?: string }).reason} /></li>)}</ul></div> : null}
      <Button disabled={busy || dirty} onClick={() => void confirm()}>{t("tax.acquisition.apply")}</Button>
    </div> : null}
    {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
  </section>;
}


export function AcquisitionBlocker({ reason }: { reason?: string }) {
  const { t } = useTranslation("transactions");
  const key = reason === "acquisition_valuation_unsupported" ? "unsupportedValuation"
    : reason === "basis_provenance_incomplete" ? "incompleteBasis" : "otherBlocker";
  return <>{t(`tax.acquisition.${key}`)}</>;
}
