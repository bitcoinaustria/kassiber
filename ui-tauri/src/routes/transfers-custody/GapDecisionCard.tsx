/**
 * Decision card for one custody-gap question ("is this the same Bitcoin?")
 * or its residual follow-up ("where did the remainder go?").
 *
 * The daemon authors the whole resolution — the card only asks the minimum
 * decision and renders the plan/apply previews. Uses the same
 * ``ui.custody.review.plan`` / ``ui.custody.review.apply`` contract as the
 * previous Custody Gaps screen.
 */

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, ChevronDown, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import {
  bridgeCreateArgs,
  bridgePreviewArgs,
  formatCustodyMsat,
  residualConfirmArgs,
  residualPreviewArgs,
  shouldOfferResidualClassification,
  type BridgePreview,
  type CustodyGap,
  type CustodyGapReviewHistory,
  type CustodyResidualClassification,
  type FiledReportImpactPreview,
  type ResidualClassificationPreview,
} from "../custodyGapsModel";
import { ReviewHistoryPanel } from "../CustodyGaps";
import { ConfirmSection } from "./ConfirmSection";
import { FlowDiagram } from "./FlowDiagram";
import { topReasonCodes } from "./inboxModel";

const RESIDUAL_GROUPS = [
  {
    labelKey: "swap.inbox.residual.groupExternal",
    options: [
      "external_payment",
      "external_disposal",
      "external_gift",
      "external_loss",
    ],
  },
  { labelKey: "swap.inbox.residual.groupMine", options: ["retained_custody"] },
  {
    labelKey: "swap.inbox.residual.groupOpen",
    options: ["suspense_continuation"],
  },
] as const satisfies ReadonlyArray<{
  labelKey: string;
  options: readonly CustodyResidualClassification[];
}>;

function msatIsPositive(value: string | number): boolean {
  try {
    return BigInt(value) > 0n;
  } catch {
    return false;
  }
}

function formatDateRange(gap: CustodyGap, language: string): string {
  const format = (value: string | null) => {
    if (!value) return "—";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat(language, {
      year: "numeric",
      month: "short",
      day: "numeric",
    }).format(date);
  };
  return `${format(gap.started_at)} – ${format(gap.ended_at)}`;
}

/** Fetched lazily — only mounts once the evidence disclosure opens. */
function GapEvidenceDetails({ gap }: { gap: CustodyGap }) {
  const { t, i18n } = useTranslation("custodyGaps");
  const historyQuery = useDaemon<CustodyGapReviewHistory>(
    "ui.custody.gaps.history",
    { gap_id: gap.gap_id, limit: 100 },
  );
  return (
    <div className="space-y-3 border-l-2 pl-3 text-sm">
      <p className="text-xs text-muted-foreground">
        {formatDateRange(gap, i18n.language)}
      </p>
      <ul className="space-y-1 text-muted-foreground">
        {gap.reason_codes.map((reason) => (
          <li key={reason}>
            {t(`reasons.${reason}`, {
              defaultValue: reason.replaceAll("_", " "),
            })}
          </li>
        ))}
      </ul>
      {historyQuery.data?.data ? (
        <ReviewHistoryPanel
          history={historyQuery.data.data.history}
          asset={gap.asset}
        />
      ) : historyQuery.isLoading ? (
        <p className="text-muted-foreground">{t("reviewHistory.loading")}</p>
      ) : null}
    </div>
  );
}

type PendingGapPlan =
  | { action: "create"; preview: BridgePreview }
  | { action: "dismiss"; preview: BridgePreview }
  | {
      action: "classify_residual";
      preview: ResidualClassificationPreview;
      reason: string;
    };

export function GapDecisionCard({
  gap,
  onSettled,
}: {
  gap: CustodyGap;
  onSettled?: () => void;
}) {
  const { t } = useTranslation("review");
  const { t: tGaps } = useTranslation("custodyGaps");
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const addNotification = useUiStore((state) => state.addNotification);
  const [pendingPlan, setPendingPlan] = useState<PendingGapPlan | null>(null);
  const [residualChoice, setResidualChoice] =
    useState<CustodyResidualClassification | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [evidenceOpen, setEvidenceOpen] = useState(false);

  const reviewPlan = useDaemonMutation<
    BridgePreview | ResidualClassificationPreview
  >("ui.custody.review.plan", { invalidateQueries: false });
  const reviewApply = useDaemonMutation("ui.custody.review.apply");

  const residualMode = shouldOfferResidualClassification(gap);
  const destinations = gap.destination_wallet_labels.length
    ? gap.destination_wallet_labels.join(", ")
    : "—";
  const residualAmount = formatCustodyMsat(gap.residual_msat, gap.asset);

  const fail = (cause: unknown) => {
    setActionError(
      cause instanceof Error ? cause.message : tGaps("actions.failed"),
    );
  };

  // Filed-report impacts as plain caution sentences (no nested panels).
  const impactNotes = (impacts: FiledReportImpactPreview[]) =>
    impacts.flatMap((impact) => [
      tGaps("filedImpact.report", {
        kind: impact.report_kind.replaceAll("_", " "),
        state: tGaps(`filedImpact.state.${impact.report_state}`),
        start: impact.affected_period_start_year,
        end: impact.affected_period_end_year,
      }),
      impact.amendment_warning,
    ]);

  const planBridge = async () => {
    setActionError(null);
    setPendingPlan(null);
    try {
      const result = await reviewPlan.mutateAsync(bridgePreviewArgs(gap.gap_id));
      if (result.data) {
        setPendingPlan({ action: "create", preview: result.data as BridgePreview });
      }
    } catch (cause) {
      fail(cause);
    }
  };

  const confirmBridge = async () => {
    if (pendingPlan?.action !== "create") return;
    setActionError(null);
    try {
      await reviewApply.mutateAsync(bridgeCreateArgs(pendingPlan.preview));
      setPendingPlan(null);
      addNotification({
        title: t("swap.inbox.gap.connectedTitle"),
        body: t("swap.inbox.gap.connectedBody"),
        tone: "success",
      });
      onSettled?.();
    } catch (cause) {
      fail(cause);
    }
  };

  const planDismiss = async () => {
    setActionError(null);
    setPendingPlan(null);
    try {
      const result = await reviewPlan.mutateAsync({
        action: "dismiss",
        gap_id: gap.gap_id,
      });
      if (result.data) {
        setPendingPlan({
          action: "dismiss",
          preview: result.data as BridgePreview,
        });
      }
    } catch (cause) {
      fail(cause);
    }
  };

  const confirmDismiss = async () => {
    if (pendingPlan?.action !== "dismiss") return;
    setActionError(null);
    try {
      await reviewApply.mutateAsync({
        action: "dismiss",
        gap_id: gap.gap_id,
        expected_input_version: pendingPlan.preview.input_version,
      });
      setPendingPlan(null);
      addNotification({
        title: t("swap.inbox.gap.dismissedTitle"),
        body: t("swap.inbox.gap.dismissedBody"),
        tone: "info",
      });
      onSettled?.();
    } catch (cause) {
      fail(cause);
    }
  };

  const planResidual = async () => {
    if (!residualChoice) return;
    setActionError(null);
    setPendingPlan(null);
    try {
      const result = await reviewPlan.mutateAsync(
        residualPreviewArgs(gap.gap_id, residualChoice, ""),
      );
      if (result.data) {
        setPendingPlan({
          action: "classify_residual",
          preview: result.data as ResidualClassificationPreview,
          reason: "",
        });
      }
    } catch (cause) {
      fail(cause);
    }
  };

  const confirmResidual = async () => {
    if (pendingPlan?.action !== "classify_residual") return;
    setActionError(null);
    try {
      await reviewApply.mutateAsync(
        residualConfirmArgs(pendingPlan.preview, pendingPlan.reason),
      );
      setPendingPlan(null);
      addNotification({
        title: t("swap.inbox.residual.savedTitle"),
        body: t("swap.inbox.residual.savedBody"),
        tone: "success",
      });
      onSettled?.();
    } catch (cause) {
      fail(cause);
    }
  };

  const busy = reviewPlan.isPending || reviewApply.isPending;
  const impactParts: string[] = [];
  if (gap.downstream.affected_disposals > 0) {
    impactParts.push(
      t("swap.inbox.impactDisposals", {
        count: gap.downstream.affected_disposals,
      }),
    );
  }
  if (gap.downstream.affected_years.length > 0) {
    impactParts.push(
      t("swap.inbox.impactYears", {
        years: gap.downstream.affected_years.join(", "),
      }),
    );
  }

  return (
    <Card className="gap-4 py-5">
      <CardContent className="space-y-4 px-5">
        <div>
          <p className="text-[10px] font-medium uppercase tracking-[0.18em] text-muted-foreground">
            {[
              t("swap.inbox.type.gap"),
              tGaps(`confidence.${gap.confidence}`),
              gap.promotion_eligible ? t("swap.inbox.suggestedBadge") : null,
              gap.status === "conflicting"
                ? t("swap.inbox.competingBadge")
                : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
          <h2 className="mt-1.5 text-xl font-semibold">
            {residualMode
              ? t("swap.inbox.residual.question", { amount: residualAmount })
              : t("swap.inbox.gap.question")}
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {residualMode
              ? t("swap.inbox.residual.subtitle", {
                  out: formatCustodyMsat(gap.source_total_msat, gap.asset),
                  back: formatCustodyMsat(gap.return_total_msat, gap.asset),
                  fee: formatCustodyMsat(gap.source_fee_msat, gap.asset),
                })
              : t("swap.inbox.gap.subtitle")}
          </p>
        </div>

        <FlowDiagram
          from={{ label: gap.source_wallet_label }}
          via={{ label: t("swap.inbox.gap.unknownHop"), unknown: true }}
          to={{ label: destinations }}
          outAmount={formatCustodyMsat(gap.source_total_msat, gap.asset)}
          backAmount={formatCustodyMsat(gap.return_total_msat, gap.asset)}
          fee={
            msatIsPositive(gap.source_fee_msat)
              ? t("swap.inbox.gap.feeNote", {
                  amount: formatCustodyMsat(gap.source_fee_msat, gap.asset),
                })
              : null
          }
          residual={
            msatIsPositive(gap.residual_msat)
              ? t("swap.inbox.gap.residualNote", { amount: residualAmount })
              : null
          }
          hideSensitive={hideSensitive}
        />

        {residualMode ? (
          <div className="space-y-3">
            {RESIDUAL_GROUPS.map((group) => (
              <div key={group.labelKey} className="space-y-1.5">
                <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  {t(group.labelKey)}
                </p>
                <div className="grid gap-1.5">
                  {group.options.map((option) => (
                    <button
                      key={option}
                      type="button"
                      onClick={() => {
                        setResidualChoice(option);
                        setPendingPlan(null);
                      }}
                      aria-pressed={residualChoice === option}
                      className={cn(
                        "flex items-start gap-2.5 rounded-md border p-3 text-left text-sm transition-colors hover:border-foreground/40",
                        residualChoice === option &&
                          "border-primary ring-1 ring-primary",
                      )}
                    >
                      <CheckCircle2
                        className={cn(
                          "mt-0.5 size-4 shrink-0",
                          residualChoice === option
                            ? "text-primary"
                            : "text-muted-foreground/40",
                        )}
                        aria-hidden="true"
                      />
                      <span>
                        <span className="font-medium">
                          {tGaps(`residual.options.${option}.label`)}
                        </span>
                        <span className="mt-0.5 block text-xs text-muted-foreground">
                          {tGaps(`residual.options.${option}.description`)}
                        </span>
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            ))}
            <p className="text-xs text-muted-foreground">
              {tGaps("residual.taxBoundary")}
            </p>
          </div>
        ) : (
          <div className="space-y-1.5">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("swap.inbox.whyTitle")}
            </p>
            <ul className="space-y-1 text-sm text-muted-foreground">
              {topReasonCodes(gap).map((reason) => (
                <li key={reason} className="flex gap-2">
                  <span aria-hidden="true">–</span>
                  <span>
                    {tGaps(`reasons.${reason}`, {
                      defaultValue: reason.replaceAll("_", " "),
                    })}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <Collapsible open={evidenceOpen} onOpenChange={setEvidenceOpen}>
          <CollapsibleTrigger className="flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:underline">
            {evidenceOpen ? (
              <ChevronDown className="size-3.5" aria-hidden="true" />
            ) : (
              <ChevronRight className="size-3.5" aria-hidden="true" />
            )}
            {t("swap.inbox.evidenceToggle")}
          </CollapsibleTrigger>
          <CollapsibleContent className="mt-2">
            {evidenceOpen ? <GapEvidenceDetails gap={gap} /> : null}
          </CollapsibleContent>
        </Collapsible>

        {!residualMode && impactParts.length > 0 ? (
          <p className="text-sm text-amber-700 dark:text-amber-300">
            {impactParts.join(" ")}
          </p>
        ) : null}

        {gap.status === "conflicting" ? (
          <p className="text-sm text-rose-700 dark:text-rose-300">
            {t("swap.inbox.gap.competingNote")}
          </p>
        ) : null}

        {actionError ? (
          <p className="text-sm text-destructive">{actionError}</p>
        ) : null}

        {residualMode ? (
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              onClick={planResidual}
              disabled={busy || !residualChoice}
            >
              {reviewPlan.isPending
                ? tGaps("residual.previewing")
                : t("swap.inbox.residual.saveAnswer")}
            </Button>
          </div>
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button type="button" onClick={planBridge} disabled={busy}>
              {reviewPlan.isPending && !pendingPlan
                ? tGaps("actions.previewing")
                : t("swap.inbox.gap.yes")}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={planDismiss}
              disabled={busy}
            >
              {t("swap.inbox.gap.no")}
            </Button>
          </div>
        )}

        {pendingPlan?.action === "create" ? (
          <ConfirmSection
            heading={t("swap.inbox.confirmTitle")}
            lines={[
              tGaps("actions.previewAmounts", {
                retained: formatCustodyMsat(
                  pendingPlan.preview.retained_msat,
                  gap.asset,
                ),
                residual: formatCustodyMsat(
                  pendingPlan.preview.residual_msat,
                  gap.asset,
                ),
                fee: formatCustodyMsat(pendingPlan.preview.fee_msat, gap.asset),
              }),
            ]}
            notes={[
              ...(pendingPlan.preview.warnings ?? []).map((warning) =>
                tGaps(`warnings.${warning}`, {
                  defaultValue: warning.replaceAll("_", " "),
                }),
              ),
              ...impactNotes(pendingPlan.preview.filed_report_impacts),
            ]}
            checkboxLabel={tGaps("actions.explicitOwnershipConfirmation")}
            confirmLabel={tGaps("actions.confirmBridge")}
            pendingLabel={tGaps("actions.creating")}
            isPending={reviewApply.isPending}
            disabled={!pendingPlan.preview.activatable}
            onConfirm={confirmBridge}
            onBack={() => setPendingPlan(null)}
            backLabel={t("swap.inbox.back")}
          />
        ) : null}
        {pendingPlan?.action === "dismiss" ? (
          <ConfirmSection
            heading={t("swap.inbox.gap.dismissTitle")}
            lines={[t("swap.inbox.gap.dismissBody")]}
            confirmLabel={t("swap.inbox.gap.dismissConfirm")}
            pendingLabel={t("swap.inbox.gap.dismissing")}
            isPending={reviewApply.isPending}
            onConfirm={confirmDismiss}
            onBack={() => setPendingPlan(null)}
            backLabel={t("swap.inbox.back")}
          />
        ) : null}
        {pendingPlan?.action === "classify_residual" ? (
          <ConfirmSection
            heading={t("swap.inbox.confirmTitle")}
            lines={[
              tGaps("residual.previewAmount", {
                amount: formatCustodyMsat(
                  pendingPlan.preview.residual_msat,
                  gap.asset,
                ),
                classification: tGaps(
                  `residual.options.${pendingPlan.preview.classification}.label`,
                ),
              }),
              tGaps(`residual.custodyState.${pendingPlan.preview.custody_state}`),
              tGaps("residual.taxMeaningUnassigned"),
            ]}
            notes={[
              ...(pendingPlan.preview.classification === "external_gift" ||
              pendingPlan.preview.classification === "external_loss"
                ? [tGaps("residual.giftLossTaxReview")]
                : []),
              ...impactNotes(pendingPlan.preview.filed_report_impacts),
            ]}
            checkboxLabel={tGaps("residual.confirmation")}
            confirmLabel={tGaps("residual.confirm")}
            pendingLabel={tGaps("residual.confirming")}
            isPending={reviewApply.isPending}
            disabled={pendingPlan.preview.activatable !== true}
            onConfirm={confirmResidual}
            onBack={() => setPendingPlan(null)}
            backLabel={t("swap.inbox.back")}
          />
        ) : null}
      </CardContent>
    </Card>
  );
}
