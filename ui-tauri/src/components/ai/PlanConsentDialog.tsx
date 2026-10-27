import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { AiToolConsentDecision, AiToolConsentRequest } from "@/daemon/stream";
import { formatCustodyMsat } from "@/routes/custodyGapsModel";
import {
  planApprovalAvailable,
  planConsentPreview,
  type ImportPlanPreview,
  type PlanBlocker,
  type PlanConsentPreview,
  type RollbackPlanPreview,
} from "./planConsent";

function Fact({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-3">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="font-medium tabular-nums">{value}</dd>
    </div>
  );
}

function Blockers({ blockers }: { blockers: PlanBlocker[] }) {
  const { t } = useTranslation("assistant");
  if (!blockers.length) return null;
  return (
    <section role="alert" className="space-y-1 rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
      <h4 className="text-xs font-medium">{t("consent.plan.blockers")}</h4>
      <ul className="space-y-1">
        {blockers.map((blocker) => (
          <li key={blocker.code}>
            {t(`consent.plan.blocker.${blocker.code}`, { defaultValue: blocker.message || blocker.code })}
            {blocker.desktopOnly ? (
              <span className="block text-xs text-muted-foreground">{t("consent.plan.desktopOnly")}</span>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

function ImportPlanBody({ plan }: { plan: ImportPlanPreview }) {
  const { t } = useTranslation("assistant");
  return (
    <div className="space-y-4 text-sm">
      <dl className="space-y-2">
        <Fact
          label={t("consent.plan.target")}
          value={t(plan.createsWallet ? "consent.plan.newWallet" : "consent.plan.existingWallet", {
            label: plan.targetLabel,
          })}
        />
        <Fact label={t("consent.plan.rowsToImport")} value={plan.rows.toImport} />
        <Fact label={t("consent.plan.rowsPresent")} value={plan.rows.alreadyPresent} />
        {plan.rows.updatesExisting ? (
          <Fact label={t("consent.plan.rowsUpdated")} value={plan.rows.updatesExisting} />
        ) : null}
        <Fact
          label={t("consent.plan.dateRange")}
          value={
            plan.dateRange
              ? `${plan.dateRange.first ?? "—"} → ${plan.dateRange.last ?? "—"}`
              : t("consent.plan.withheld")
          }
        />
        {plan.overlap.inDateRange ? (
          <Fact label={t("consent.plan.overlapRange")} value={plan.overlap.inDateRange} />
        ) : null}
        {plan.overlap.inOtherWallets ? (
          <Fact label={t("consent.plan.overlapOther")} value={plan.overlap.inOtherWallets} />
        ) : null}
      </dl>
      {plan.totals.length ? (
        <section className="space-y-1">
          <h4 className="text-xs font-medium text-muted-foreground">{t("consent.plan.totals")}</h4>
          <ul className="space-y-1 font-mono text-xs">
            {plan.totals.map((total) => (
              <li key={`${total.asset}-${total.direction}`}>
                {t(`consent.plan.direction.${total.direction}`, { defaultValue: total.direction })}
                {" · "}
                {t("consent.plan.rowCount", { count: total.rows })}
                {total.amountMsat !== null ? ` · ${formatCustodyMsat(total.amountMsat, total.asset)}` : ""}
                {total.feeMsat ? ` · ${t("consent.plan.fee")} ${formatCustodyMsat(total.feeMsat, total.asset)}` : ""}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {plan.warnings.length ? (
        <section className="space-y-1">
          <h4 className="text-xs font-medium text-muted-foreground">{t("consent.plan.warnings")}</h4>
          <ul className="list-disc space-y-1 pl-4 text-xs">
            {plan.warnings.map((code) => (
              <li key={code}>{t(`consent.plan.warning.${code}`, { defaultValue: code })}</li>
            ))}
          </ul>
        </section>
      ) : null}
      <Blockers blockers={plan.blockers} />
    </div>
  );
}

function RollbackPlanBody({ plan }: { plan: RollbackPlanPreview }) {
  const { t } = useTranslation("assistant");
  return (
    <div className="space-y-4 text-sm">
      <dl className="space-y-2">
        <Fact label={t("consent.plan.rowsToDelete")} value={plan.transactionsToDelete} />
        {plan.importedAt ? <Fact label={t("consent.plan.importedAt")} value={plan.importedAt} /> : null}
        {plan.sourceFormat ? <Fact label={t("consent.plan.sourceFormat")} value={plan.sourceFormat} /> : null}
      </dl>
      {plan.alsoRemoved.length ? (
        <section className="space-y-1">
          <h4 className="text-xs font-medium text-muted-foreground">{t("consent.plan.alsoRemoved")}</h4>
          <ul className="font-mono text-xs">
            {plan.alsoRemoved.map((item) => (
              <li key={item.table}>{item.table}: {item.count}</li>
            ))}
          </ul>
        </section>
      ) : null}
      <Blockers blockers={plan.blockers} />
    </div>
  );
}

export function PlanConsentBody({ preview }: { preview: PlanConsentPreview | null }) {
  const { t } = useTranslation("assistant");
  if (!preview || preview.status === "unavailable" || preview.status === "stale") {
    return (
      <p role="alert" className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
        {t(preview?.status === "stale" ? "consent.plan.stale" : "consent.plan.unavailable")}
        {preview?.code ? <span className="mt-2 block font-mono text-xs">{preview.code}</span> : null}
      </p>
    );
  }
  return preview.kind === "import" ? <ImportPlanBody plan={preview} /> : <RollbackPlanBody plan={preview} />;
}

export function PlanConsentDialog({
  request,
  onDecision,
}: {
  request: AiToolConsentRequest;
  onDecision: (decision: AiToolConsentDecision) => Promise<void> | void;
}) {
  const { t } = useTranslation("assistant");
  const descriptionId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [pending, setPending] = useState(false);
  const preview = planConsentPreview(request.name, request.argumentsPreview);
  const canApprove = planApprovalAvailable(preview);
  const rollback = request.name === "ui.imports.rollback";
  const decide = async (decision: AiToolConsentDecision) => {
    if (pending || (decision !== "deny" && !canApprove)) return;
    setPending(true);
    try {
      await onDecision(decision);
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog open>
      <DialogContent
        role="alertdialog"
        aria-describedby={descriptionId}
        showCloseButton={false}
        className="grid grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-lg"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          cancelRef.current?.focus();
        }}
        onEscapeKeyDown={(event) => {
          event.preventDefault();
          void decide("deny");
        }}
      >
        <DialogHeader className="border-b px-5 py-4 text-left">
          <DialogTitle>{t(rollback ? "consent.plan.titleRollback" : "consent.plan.titleImport")}</DialogTitle>
          <DialogDescription id={descriptionId}>
            <span className="block font-medium text-foreground">{request.summary}</span>
            {t("consent.plan.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 overflow-y-auto overscroll-none p-5">
          <PlanConsentBody preview={preview} />
        </div>
        <div className="flex flex-wrap justify-end gap-2 border-t bg-background px-5 py-4">
          <Button ref={cancelRef} variant="outline" className="min-h-11" disabled={pending} onClick={() => void decide("deny")}>
            {t("consent.apply.cancel")}
          </Button>
          <Button className="min-h-11" disabled={pending || !canApprove} onClick={() => void decide("allow_once")}>
            {t(rollback ? "consent.plan.confirmRollback" : "consent.plan.confirmImport")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
