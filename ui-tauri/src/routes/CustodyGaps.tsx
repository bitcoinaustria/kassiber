import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  ArrowRight,
  CheckCircle2,
  Clock3,
  Route,
  ShieldCheck,
  TriangleAlert,
  WalletCards,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  useDaemon,
  useDaemonInfinite,
  useDaemonMutation,
} from "@/daemon/client";
import {
  pageHeaderClassName,
  screenPanelClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import {
  type CustodyGap,
  type CustodyGapReviewHistory,
  type CustodyGapReviewHistoryEntry,
  type CustodyGapSnapshot,
  type CustodyGapStatus,
  type CustodyResidualClassification,
  type CustodyCoverageSnapshot,
  type CustodyLineageItem,
  type CustodyLineageSnapshot,
  type BridgePreview,
  type FiledReportImpactPreview,
  type GuidedCorrectionPreview,
  type ResidualClassificationPreview,
  bridgeCreateArgs,
  bridgePreviewArgs,
  canShowNoKnownCustodyGaps,
  collectCustodyGapPages,
  collectCustodyLineagePages,
  custodyGapActionMode,
  formatCustodyMsat,
  reopenConfirmArgs,
  reopenPreviewArgs,
  residualConfirmArgs,
  residualPreviewArgs,
  reviseConfirmArgs,
  revisePreviewArgs,
  shouldOfferResidualClassification,
} from "./custodyGapsModel";

function formatDate(value: string | null, language: string): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(language, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
}

function gapDestinations(gap: CustodyGap): string {
  return gap.destination_wallet_labels.length
    ? gap.destination_wallet_labels.join(", ")
    : "—";
}

function statusVariant(status: CustodyGapStatus) {
  if (status === "conflicting") return "destructive" as const;
  if (status === "needs_review") return "secondary" as const;
  return "outline" as const;
}

export function CustodyCoverageTimeline({
  snapshot,
}: {
  snapshot: CustodyCoverageSnapshot;
}) {
  const { t, i18n } = useTranslation("custodyGaps");

  return (
    <section className="space-y-3" aria-labelledby="custody-coverage-title">
      <div>
        <div className="flex flex-wrap items-center gap-2">
          <h2 id="custody-coverage-title" className="font-semibold">
            {t("coverage.title")}
          </h2>
          <Badge variant="destructive">{t("coverage.ownershipUnknown")}</Badge>
        </div>
        <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
          {t("coverage.description")}
        </p>
      </div>

      <Card className="gap-2 border-amber-500/30 bg-amber-500/5 py-4">
        <CardContent className="flex items-start gap-3 px-4 text-sm">
          <TriangleAlert className="mt-0.5 size-5 shrink-0 text-amber-600" />
          <p>{t("coverage.technicalOnly")}</p>
        </CardContent>
      </Card>

      {snapshot.wallets.length ? (
        <div className="space-y-4">
          {snapshot.wallets.map((wallet, walletIndex) => (
            <Card
              className="gap-4 py-5"
              key={`${wallet.wallet_label}-${walletIndex}`}
            >
              <CardHeader className="px-5">
                <CardTitle className="flex items-center gap-2 text-base">
                  <WalletCards className="size-4" />
                  {wallet.wallet_label}
                </CardTitle>
                <CardDescription>
                  {t("coverage.epochCount", { count: wallet.epochs.length })}
                </CardDescription>
              </CardHeader>
              <CardContent className="px-5">
                <ol className="space-y-4 border-l pl-5">
                  {wallet.epochs.map((epoch) => (
                    <li className="relative space-y-3" key={epoch.epoch_id}>
                      <span className="bg-background absolute -left-[1.55rem] top-1 size-3 rounded-full border" />
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge
                          variant={epoch.status === "active" ? "default" : "outline"}
                        >
                          {t(`coverage.${epoch.status}`)}
                        </Badge>
                        <span className="text-xs text-muted-foreground">
                          {epoch.chain} · {epoch.network}
                        </span>
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {t("coverage.started", {
                          date: formatDate(epoch.created_at, i18n.language),
                        })}
                        {epoch.retired_at
                          ? ` · ${t("coverage.retiredAt", {
                              date: formatDate(epoch.retired_at, i18n.language),
                            })}`
                          : ""}
                      </p>
                      {epoch.sources.length ? (
                        <div className="grid gap-3 lg:grid-cols-2">
                          {epoch.sources.map((source, sourceIndex) => (
                            <div
                              className="space-y-2 rounded-lg border p-3"
                              key={`${source.source}-${source.observer_kind}-${sourceIndex}`}
                            >
                              <p className="text-sm font-medium">
                                {source.observer_kind} · {source.source}
                              </p>
                              {source.branches.length ? (
                                <div className="space-y-2">
                                  {source.branches.map((branch) => (
                                    <div
                                      className="rounded-md bg-muted/40 p-2 text-xs"
                                      key={branch.branch}
                                    >
                                      <p className="font-medium">
                                        {t(`coverage.branch.${branch.branch}`)}
                                      </p>
                                      {branch.scanned_to_exclusive === null ? (
                                        <p className="text-muted-foreground">
                                          {t("coverage.notObserved")}
                                        </p>
                                      ) : (
                                        <>
                                          <p>
                                            {t("coverage.exclusiveBound", {
                                              index: branch.scanned_to_exclusive,
                                            })}
                                          </p>
                                          <p>
                                            {branch.highest_used === null
                                              ? t("coverage.unused")
                                              : t("coverage.highestUsed", {
                                                  index: branch.highest_used,
                                                })}
                                          </p>
                                          <p className="text-muted-foreground">
                                            {t("coverage.observed", {
                                              date: formatDate(
                                                branch.observed_at,
                                                i18n.language,
                                              ),
                                            })}
                                          </p>
                                        </>
                                      )}
                                    </div>
                                  ))}
                                </div>
                              ) : (
                                <p className="text-xs text-muted-foreground">
                                  {t("coverage.noBranches")}
                                </p>
                              )}
                            </div>
                          ))}
                        </div>
                      ) : (
                        <p className="text-sm text-muted-foreground">
                          {t("coverage.noSources")}
                        </p>
                      )}
                    </li>
                  ))}
                </ol>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : (
        <Card className="gap-2 py-5">
          <CardContent className="px-5 text-sm text-muted-foreground">
            {t("coverage.empty")}
          </CardContent>
        </Card>
      )}
    </section>
  );
}

type LineageEvidenceKey =
  | "lineage.evidence.recordedFanout"
  | "lineage.evidence.rowMatched"
  | "lineage.evidence.nativeTransfer"
  | "lineage.evidence.reviewedTransfer"
  | "lineage.evidence.reviewedGapBridge"
  | "lineage.evidence.channelLifecycle";

const LINEAGE_EVIDENCE_KEYS: Record<string, LineageEvidenceKey> = {
  recorded_fanout: "lineage.evidence.recordedFanout",
  row_matched: "lineage.evidence.rowMatched",
  verified_native_transfer: "lineage.evidence.nativeTransfer",
  reviewed_transfer_pair: "lineage.evidence.reviewedTransfer",
  reviewed_gap_bridge: "lineage.evidence.reviewedGapBridge",
  manual: "lineage.evidence.reviewedTransfer",
  bulk_exact: "lineage.evidence.reviewedTransfer",
  bulk_selected: "lineage.evidence.reviewedTransfer",
  rule_auto: "lineage.evidence.reviewedTransfer",
  channel_lifecycle: "lineage.evidence.channelLifecycle",
};

function lineageEvidenceLabel(
  item: CustodyLineageItem,
  t: TFunction<"custodyGaps">,
): string {
  const key = LINEAGE_EVIDENCE_KEYS[item.evidence_reason];
  if (key) return t(key);
  return item.custody_state === "internal_reviewed"
    ? t("lineage.evidence.reviewedDefault")
    : t("lineage.evidence.verifiedDefault");
}

export function CustodyLineageTimeline({
  snapshot,
}: {
  snapshot: CustodyLineageSnapshot;
}) {
  const { t, i18n } = useTranslation("custodyGaps");
  const hideSensitive = useUiStore((state) => state.hideSensitive);

  return (
    <section className="space-y-3" aria-labelledby="custody-lineage-title">
      <div>
        <div className="flex flex-wrap items-center gap-2">
          <h2 id="custody-lineage-title" className="font-semibold">
            {t("lineage.title")}
          </h2>
          <Badge variant="outline">
            {t("lineage.edgeCount", { count: snapshot.summary.total_count })}
          </Badge>
        </div>
        <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
          {t("lineage.description")}
        </p>
      </div>

      {snapshot.items.length ? (
        <div className="space-y-3">
          {snapshot.items.map((item, index) => (
            <Card
              className="gap-3 py-4"
              key={`${item.occurred_at ?? "unknown"}-${item.from_wallet_label}-${item.to_wallet_label}-${item.amount_msat}-${index}`}
            >
              <CardContent className="space-y-3 px-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="text-xs text-muted-foreground">
                    {formatDate(item.occurred_at, i18n.language)}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <Badge
                      variant={
                        item.custody_state === "internal_verified"
                          ? "default"
                          : "outline"
                      }
                    >
                      {t(`lineage.custodyState.${item.custody_state}`)}
                    </Badge>
                    <Badge
                      variant="outline"
                      className={cn(
                        item.basis_state === "eligible"
                          ? "border-emerald-500/40 text-emerald-700 dark:text-emerald-400"
                          : "border-amber-500/50 bg-amber-500/5 text-amber-700 dark:text-amber-400",
                      )}
                    >
                      {t(`lineage.basisState.${item.basis_state}`)}
                    </Badge>
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
                  <span>{item.from_wallet_label}</span>
                  <ArrowRight
                    className="size-4 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <span>{item.to_wallet_label}</span>
                  <span className={cn("ml-auto", hideSensitive && "sensitive")}>
                    {formatCustodyMsat(item.amount_msat, item.asset)}
                  </span>
                </div>

                <p className="text-xs text-muted-foreground">
                  {t("lineage.evidenceLabel", {
                    evidence: lineageEvidenceLabel(item, t),
                  })}
                </p>
                {item.basis_state === "blocked_by_prior_custody_basis" ? (
                  <p className="text-xs text-amber-700 dark:text-amber-400">
                    {t("lineage.basisBlocked")}
                  </p>
                ) : null}
              </CardContent>
            </Card>
          ))}
        </div>
      ) : (
        <Card className="gap-2 py-5">
          <CardContent className="px-5 text-sm text-muted-foreground">
            {t("lineage.empty")}
          </CardContent>
        </Card>
      )}
    </section>
  );
}

function FiledReportImpactList({
  impacts,
}: {
  impacts: FiledReportImpactPreview[];
}) {
  const { t } = useTranslation("custodyGaps");
  if (!impacts.length) return null;
  return (
    <div
      className="space-y-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3"
      data-testid="custody-filed-report-impacts"
    >
      <p className="font-medium text-amber-900 dark:text-amber-200">
        {t("filedImpact.title")}
      </p>
      {impacts.map((impact) => (
        <div key={impact.filed_report_snapshot_id} className="space-y-1">
          <p>
            {t("filedImpact.report", {
              kind: impact.report_kind.replaceAll("_", " "),
              state: t(`filedImpact.state.${impact.report_state}`),
              start: impact.affected_period_start_year,
              end: impact.affected_period_end_year,
            })}
          </p>
          <p className="text-muted-foreground">{impact.amendment_warning}</p>
          {impact.after_gain_summary.status === "pending_journal_rebuild" ? (
            <p className="font-medium text-amber-900 dark:text-amber-200">
              {t("filedImpact.pendingRebuild")}
            </p>
          ) : null}
        </div>
      ))}
    </div>
  );
}

export function BridgePreviewPanel({
  preview,
  asset,
  isCreating,
  onConfirm,
}: {
  preview: BridgePreview;
  asset: string;
  isCreating: boolean;
  onConfirm: () => void;
}) {
  const { t } = useTranslation("custodyGaps");
  const [confirmed, setConfirmed] = useState(false);
  const warnings = preview.warnings ?? [];
  const manualReview = preview.review_mode === "manual_weak_hint";

  return (
    <div
      className={cn(
        "space-y-3 rounded-md border p-3 text-sm",
        manualReview
          ? "border-amber-500/40 bg-amber-500/10"
          : "border-emerald-500/30 bg-emerald-500/5",
      )}
    >
      <p className="font-medium">{t("actions.previewReady")}</p>
      <p className="text-muted-foreground">
        {t("actions.previewAmounts", {
          retained: formatCustodyMsat(preview.retained_msat, asset),
          residual: formatCustodyMsat(preview.residual_msat, asset),
          fee: formatCustodyMsat(preview.fee_msat, asset),
        })}
      </p>
      {warnings.length ? (
        <div className="space-y-2 rounded-md border border-amber-500/40 bg-background/60 p-3">
          <p className="font-medium text-amber-900 dark:text-amber-200">
            {t("actions.reviewWarnings")}
          </p>
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            {warnings.map((warning) => (
              <li key={warning}>
                {t(`warnings.${warning}`, {
                  defaultValue: warning.replaceAll("_", " "),
                })}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <FiledReportImpactList impacts={preview.filed_report_impacts} />
      <label className="flex items-start gap-2 rounded-md border bg-background/60 p-3">
        <input
          type="checkbox"
          className="mt-0.5 size-4"
          checked={confirmed}
          onChange={(event) => setConfirmed(event.currentTarget.checked)}
        />
        <span>{t("actions.explicitOwnershipConfirmation")}</span>
      </label>
      <Button
        type="button"
        size="sm"
        onClick={onConfirm}
        disabled={isCreating || !confirmed || !preview.activatable}
      >
        {isCreating ? t("actions.creating") : t("actions.confirmBridge")}
      </Button>
    </div>
  );
}

function ExplicitPreviewConfirmation({
  children,
  confirmationLabel,
  confirmLabel,
  pendingLabel,
  isPending,
  disabled,
  onConfirm,
}: {
  children: ReactNode;
  confirmationLabel: string;
  confirmLabel: string;
  pendingLabel: string;
  isPending: boolean;
  disabled?: boolean;
  onConfirm: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  return (
    <div className="space-y-3 rounded-md border border-emerald-500/30 bg-emerald-500/5 p-3 text-sm">
      {children}
      <label className="flex items-start gap-2 rounded-md border bg-background/60 p-3">
        <input
          type="checkbox"
          className="mt-0.5 size-4"
          checked={confirmed}
          onChange={(event) => setConfirmed(event.currentTarget.checked)}
        />
        <span>{confirmationLabel}</span>
      </label>
      <Button
        type="button"
        size="sm"
        onClick={onConfirm}
        disabled={isPending || disabled || !confirmed}
      >
        {isPending ? pendingLabel : confirmLabel}
      </Button>
    </div>
  );
}

export function CorrectionPreviewPanel({
  preview,
  mode,
  asset,
  isPending,
  onConfirm,
}: {
  preview: GuidedCorrectionPreview;
  mode: "reopen" | "revise";
  asset: string;
  isPending: boolean;
  onConfirm: () => void;
}) {
  const { t } = useTranslation("custodyGaps");
  return (
    <ExplicitPreviewConfirmation
      confirmationLabel={t(`correction.${mode}.confirmation`)}
      confirmLabel={t(`correction.${mode}.confirm`)}
      pendingLabel={t(`correction.${mode}.confirming`)}
      isPending={isPending}
      disabled={mode === "revise" && preview.activatable !== true}
      onConfirm={onConfirm}
    >
      <p className="font-medium">{t(`correction.${mode}.previewReady`)}</p>
      <p className="text-muted-foreground">
        {t(`correction.${mode}.revision`, {
          current: preview.current_component_revision,
          next: preview.new_component_revision ?? "—",
        })}
      </p>
      {preview.retained_msat !== undefined && preview.residual_msat !== undefined ? (
        <p className="text-muted-foreground">
          {t("correction.revise.amounts", {
            retained: formatCustodyMsat(preview.retained_msat, asset),
            residual: formatCustodyMsat(preview.residual_msat, asset),
          })}
        </p>
      ) : null}
      <FiledReportImpactList impacts={preview.filed_report_impacts} />
    </ExplicitPreviewConfirmation>
  );
}

export function ResidualPreviewPanel({
  preview,
  asset,
  isPending,
  onConfirm,
}: {
  preview: ResidualClassificationPreview;
  asset: string;
  isPending: boolean;
  onConfirm: () => void;
}) {
  const { t } = useTranslation("custodyGaps");
  const needsTaxReview =
    preview.classification === "external_gift" ||
    preview.classification === "external_loss";
  return (
    <ExplicitPreviewConfirmation
      confirmationLabel={t("residual.confirmation")}
      confirmLabel={t("residual.confirm")}
      pendingLabel={t("residual.confirming")}
      isPending={isPending}
      disabled={preview.activatable !== true}
      onConfirm={onConfirm}
    >
      <p className="font-medium">{t("residual.previewReady")}</p>
      <p>
        {t("residual.previewAmount", {
          amount: formatCustodyMsat(preview.residual_msat, asset),
          classification: t(`residual.options.${preview.classification}.label`),
        })}
      </p>
      <div className="rounded-md border bg-background/60 p-3">
        <p className="font-medium">{t("residual.custodyResultTitle")}</p>
        <p className="text-muted-foreground">
          {t(`residual.custodyState.${preview.custody_state}`)}
        </p>
        <p className="mt-2 font-medium">{t("residual.taxMeaningTitle")}</p>
        <p className="text-muted-foreground">{t("residual.taxMeaningUnassigned")}</p>
        {needsTaxReview ? (
          <p className="mt-2 text-amber-800 dark:text-amber-200">
            {t("residual.giftLossTaxReview")}
          </p>
        ) : null}
      </div>
      <FiledReportImpactList impacts={preview.filed_report_impacts} />
    </ExplicitPreviewConfirmation>
  );
}

export function ReviewHistoryPanel({
  history,
  asset,
}: {
  history: CustodyGapReviewHistoryEntry[];
  asset: string;
}) {
  const { t, i18n } = useTranslation("custodyGaps");
  if (!history.length) {
    return <p className="text-sm text-muted-foreground">{t("reviewHistory.empty")}</p>;
  }
  return (
    <div className="space-y-2">
      <h3 className="text-sm font-medium">{t("reviewHistory.title")}</h3>
      <ol className="space-y-2 border-l pl-4">
        {history.map((entry) => (
          <li className="space-y-1 text-sm" key={`${entry.revision}-${entry.event_kind}`}>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline">
                {t("reviewHistory.revision", { revision: entry.revision })}
              </Badge>
              <span className="font-medium">
                {t(`reviewHistory.events.${entry.event_kind}`)}
              </span>
              <span className="text-xs text-muted-foreground">
                {formatDate(entry.created_at, i18n.language)}
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("reviewHistory.amounts", {
                retained: formatCustodyMsat(entry.retained_msat, asset),
                residual: formatCustodyMsat(entry.residual_msat, asset),
              })}
            </p>
            {entry.residual_classification ? (
              <p className="text-xs text-muted-foreground">
                {t("reviewHistory.residual", {
                  classification: t(
                    `residual.options.${entry.residual_classification}.label`,
                  ),
                })}
              </p>
            ) : null}
            {entry.reason ? (
              <p className="text-xs text-muted-foreground">
                {t("reviewHistory.reason", { reason: entry.reason })}
              </p>
            ) : null}
            {entry.filed_report_impact_count ? (
              <p className="text-xs text-amber-800 dark:text-amber-200">
                {t("reviewHistory.reportImpacts", {
                  count: entry.filed_report_impact_count,
                })}
              </p>
            ) : null}
          </li>
        ))}
      </ol>
    </div>
  );
}

const RESIDUAL_CLASSIFICATIONS: readonly CustodyResidualClassification[] = [
  "external_payment",
  "external_disposal",
  "external_gift",
  "external_loss",
  "retained_custody",
  "suspense_continuation",
];

type PendingReviewPlan =
  | { action: "create"; preview: BridgePreview }
  | {
      action: "reopen" | "revise";
      reason: string;
      preview: GuidedCorrectionPreview;
    }
  | {
      action: "classify_residual";
      reason: string;
      preview: ResidualClassificationPreview;
    };

function GapReviewDetails({ gapId }: { gapId: string }) {
  const { t } = useTranslation("custodyGaps");
  const [pendingPlan, setPendingPlan] = useState<PendingReviewPlan | null>(null);
  const [correctionReason, setCorrectionReason] = useState("");
  const [residualClassification, setResidualClassification] =
    useState<CustodyResidualClassification>("external_payment");
  const [residualReason, setResidualReason] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const reviewPlan = useDaemonMutation<BridgePreview | GuidedCorrectionPreview | ResidualClassificationPreview>(
    "ui.custody.review.plan",
    { invalidateQueries: false },
  );
  const reviewApply = useDaemonMutation("ui.custody.review.apply");
  const { data, isLoading, isError, error } = useDaemon<CustodyGapSnapshot>(
    "ui.custody.gaps.review_context",
    { gap_id: gapId },
  );
  const historyQuery = useDaemon<CustodyGapReviewHistory>(
    "ui.custody.gaps.history",
    { gap_id: gapId, limit: 100 },
  );

  if (isLoading) {
    return <p className="text-sm text-muted-foreground">{t("review.loading")}</p>;
  }
  if (isError || data?.error || !data?.data) {
    return (
      <p className="text-sm text-destructive">
        {error instanceof Error
          ? error.message
          : data?.error?.message ?? t("review.unavailable")}
      </p>
    );
  }

  const gap = data.data.gaps[0];
  if (!gap) {
    return (
      <p className="text-sm text-muted-foreground">{t("review.missing")}</p>
    );
  }

  const runPreview = async () => {
    setActionError(null);
    setPendingPlan(null);
    try {
      const result = await reviewPlan.mutateAsync(
        bridgePreviewArgs(gap.gap_id),
      );
      if (result.data) {
        setPendingPlan({
          action: "create",
          preview: result.data as BridgePreview,
        });
      }
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
    }
  };
  const confirmBridge = async () => {
    if (pendingPlan?.action !== "create") return;
    setActionError(null);
    try {
      await reviewApply.mutateAsync(bridgeCreateArgs(pendingPlan.preview));
      setPendingPlan(null);
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
    }
  };
  const dismiss = async () => {
    setActionError(null);
    try {
      const planned = await reviewPlan.mutateAsync({
        action: "dismiss",
        gap_id: gap.gap_id,
      });
      if (!planned.data || !window.confirm(t("actions.dismissConfirm"))) return;
      await reviewApply.mutateAsync({
        action: "dismiss",
        gap_id: gap.gap_id,
        expected_input_version: (planned.data as BridgePreview).input_version,
      });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
    }
  };

  const runCorrectionPreview = async (mode: "reopen" | "revise") => {
    setActionError(null);
    setPendingPlan(null);
    const reason = correctionReason.trim();
    try {
      const result = await reviewPlan.mutateAsync(
        mode === "reopen"
          ? reopenPreviewArgs(gap.gap_id, reason)
          : revisePreviewArgs(gap.gap_id, reason),
      );
      if (result.data) {
        setPendingPlan({
          action: mode,
          reason,
          preview: result.data as GuidedCorrectionPreview,
        });
      }
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
    }
  };

  const confirmCorrection = async () => {
    if (
      pendingPlan?.action !== "reopen" &&
      pendingPlan?.action !== "revise"
    ) {
      return;
    }
    setActionError(null);
    try {
      if (pendingPlan.action === "reopen") {
        await reviewApply.mutateAsync(
          reopenConfirmArgs(
            pendingPlan.preview,
            pendingPlan.reason,
          ),
        );
      } else {
        await reviewApply.mutateAsync(
          reviseConfirmArgs(
            pendingPlan.preview,
            pendingPlan.reason,
          ),
        );
      }
      setPendingPlan(null);
      setCorrectionReason("");
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
    }
  };

  const runResidualPreview = async () => {
    setActionError(null);
    setPendingPlan(null);
    const reason = residualReason.trim();
    try {
      const result = await reviewPlan.mutateAsync(
        residualPreviewArgs(gap.gap_id, residualClassification, reason),
      );
      if (result.data) {
        setPendingPlan({
          action: "classify_residual",
          reason,
          preview: result.data as ResidualClassificationPreview,
        });
      }
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
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
      setResidualReason("");
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : t("actions.failed"));
    }
  };

  const actionMode = custodyGapActionMode(gap);
  const reopened = actionMode === "revise";
  const canCreateBridge = actionMode === "create";
  const correctionPending = reviewApply.isPending;

  return (
    <div className="space-y-3 rounded-lg border bg-muted/25 p-4">
      <div>
        <p className="text-sm font-medium">{t("review.whyTitle")}</p>
        <ul className="mt-2 space-y-1 text-sm text-muted-foreground">
          {gap.reason_codes.map((reason) => (
            <li key={reason} className="flex gap-2">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-600" />
              <span>
                {t(`reasons.${reason}`, {
                  defaultValue: reason.replaceAll("_", " "),
                })}
              </span>
            </li>
          ))}
        </ul>
      </div>
      <div className="grid gap-2 text-sm sm:grid-cols-2">
        <p>
          <span className="text-muted-foreground">
            {t("review.affectedDisposals")}: {" "}
          </span>
          {gap.downstream.affected_disposals}
        </p>
        <p>
          <span className="text-muted-foreground">
            {t("review.affectedYears")}: {" "}
          </span>
          {gap.downstream.affected_years.join(", ") || "—"}
        </p>
      </div>
      {historyQuery.data?.data ? (
        <ReviewHistoryPanel
          history={historyQuery.data.data.history}
          asset={gap.asset}
        />
      ) : historyQuery.isLoading ? (
        <p className="text-sm text-muted-foreground">
          {t("reviewHistory.loading")}
        </p>
      ) : (
        <p className="text-sm text-muted-foreground">
          {t("reviewHistory.unavailable")}
        </p>
      )}
      {pendingPlan?.action === "create" ? (
        <BridgePreviewPanel
          preview={pendingPlan.preview}
          asset={gap.asset}
          isCreating={reviewApply.isPending}
          onConfirm={confirmBridge}
        />
      ) : null}
      {actionError ? <p className="text-sm text-destructive">{actionError}</p> : null}
      {canCreateBridge ? (
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            onClick={runPreview}
            disabled={reviewPlan.isPending}
          >
            {reviewPlan.isPending
              ? t("actions.previewing")
              : t("actions.previewBridge")}
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={dismiss} disabled={reviewPlan.isPending || reviewApply.isPending}>
            {t("actions.dismiss")}
          </Button>
        </div>
      ) : null}
      {actionMode === "reopen" || actionMode === "revise" ? (
        <div className="space-y-3 rounded-md border bg-background/60 p-3">
          <div>
            <p className="text-sm font-medium">
              {t(`correction.${reopened ? "revise" : "reopen"}.title`)}
            </p>
            <p className="text-xs text-muted-foreground">
              {t(`correction.${reopened ? "revise" : "reopen"}.description`)}
            </p>
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-medium" htmlFor={`correction-reason-${gap.gap_id}`}>
              {t("correction.reasonLabel")}
            </label>
            <Textarea
              id={`correction-reason-${gap.gap_id}`}
              maxLength={500}
              value={correctionReason}
              placeholder={t("correction.reasonPlaceholder")}
              onChange={(event) => {
                setCorrectionReason(event.currentTarget.value);
                setPendingPlan(null);
              }}
            />
          </div>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void runCorrectionPreview(reopened ? "revise" : "reopen")}
            disabled={reviewPlan.isPending}
          >
            {reviewPlan.isPending
              ? t("correction.previewing")
              : t(`correction.${reopened ? "revise" : "reopen"}.preview`)}
          </Button>
          {pendingPlan?.action === "reopen" ||
          pendingPlan?.action === "revise" ? (
            <CorrectionPreviewPanel
              key={`${pendingPlan.preview.gap_id}:${pendingPlan.preview.input_version}`}
              preview={pendingPlan.preview}
              mode={pendingPlan.action}
              asset={gap.asset}
              isPending={correctionPending}
              onConfirm={confirmCorrection}
            />
          ) : null}
        </div>
      ) : null}
      {gap.residual_classification ? (
        <div className="rounded-md border bg-background/60 p-3 text-sm">
          <p className="font-medium">{t("residual.currentTitle")}</p>
          <p className="text-muted-foreground">
            {t("residual.current", {
              amount: formatCustodyMsat(
                gap.residual_classification.amount_msat,
                gap.asset,
              ),
              classification: t(
                `residual.options.${gap.residual_classification.classification}.label`,
              ),
            })}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("residual.taxMeaningUnassigned")}
          </p>
        </div>
      ) : null}
      {shouldOfferResidualClassification(gap) ? (
        <div className="space-y-3 rounded-md border bg-background/60 p-3">
          <div>
            <p className="text-sm font-medium">{t("residual.title")}</p>
            <p className="text-xs text-muted-foreground">
              {t("residual.description", {
                amount: formatCustodyMsat(gap.residual_msat, gap.asset),
              })}
            </p>
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-medium" htmlFor={`residual-classification-${gap.gap_id}`}>
              {t("residual.classificationLabel")}
            </label>
            <select
              id={`residual-classification-${gap.gap_id}`}
              className="border-input bg-background h-9 w-full rounded-md border px-3 text-sm"
              value={residualClassification}
              onChange={(event) => {
                setResidualClassification(
                  event.currentTarget.value as CustodyResidualClassification,
                );
                setPendingPlan(null);
              }}
            >
              {RESIDUAL_CLASSIFICATIONS.map((classification) => (
                <option value={classification} key={classification}>
                  {t(`residual.options.${classification}.label`)}
                </option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground">
              {t(`residual.options.${residualClassification}.description`)}
            </p>
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-medium" htmlFor={`residual-reason-${gap.gap_id}`}>
              {t("residual.reasonLabel")}
            </label>
            <Textarea
              id={`residual-reason-${gap.gap_id}`}
              maxLength={500}
              value={residualReason}
              placeholder={t("residual.reasonPlaceholder")}
              onChange={(event) => {
                setResidualReason(event.currentTarget.value);
                setPendingPlan(null);
              }}
            />
          </div>
          <div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-muted-foreground">
            {t("residual.taxBoundary")}
          </div>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void runResidualPreview()}
            disabled={reviewPlan.isPending}
          >
            {reviewPlan.isPending
              ? t("residual.previewing")
              : t("residual.preview")}
          </Button>
          {pendingPlan?.action === "classify_residual" ? (
            <ResidualPreviewPanel
              key={`${pendingPlan.preview.gap_id}:${pendingPlan.preview.input_version}`}
              preview={pendingPlan.preview}
              asset={gap.asset}
              isPending={reviewApply.isPending}
              onConfirm={confirmResidual}
            />
          ) : null}
        </div>
      ) : null}
      <p className="text-xs text-muted-foreground">{t("review.explicitReview")}</p>
    </div>
  );
}

function CustodyGapCard({ gap }: { gap: CustodyGap }) {
  const { t, i18n } = useTranslation("custodyGaps");
  const [expanded, setExpanded] = useState(false);
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const destinations = gapDestinations(gap);

  return (
    <Card className="gap-4 py-5">
      <CardHeader className="gap-3 px-5 sm:grid-cols-[1fr_auto]">
        <div className="space-y-1.5">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle className={cn("text-base", hideSensitive && "sensitive")}>
              {gap.source_wallet_label}{" "}
              <ArrowRight className="mx-1 inline size-4" /> {destinations}
            </CardTitle>
            <Badge variant={statusVariant(gap.status)}>
              {t(`status.${gap.status}`)}
            </Badge>
            <Badge variant="outline">{t(`confidence.${gap.confidence}`)}</Badge>
            <Badge variant={gap.promotion_eligible ? "default" : "outline"}>
              {gap.promotion_eligible
                ? t("candidate.promotionEligible")
                : t("candidate.searchHint")}
            </Badge>
          </div>
          <CardDescription>
            {formatDate(gap.started_at, i18n.language)} –{" "}
            {formatDate(gap.ended_at, i18n.language)}
          </CardDescription>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => setExpanded((value) => !value)}
          aria-expanded={expanded}
        >
          {expanded ? t("card.hideReview") : t("card.review")}
        </Button>
      </CardHeader>
      <CardContent className="space-y-4 px-5">
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <div className="rounded-lg border p-3">
            <p className="text-xs text-muted-foreground">{t("card.principal")}</p>
            <p className={cn("mt-1 font-medium", hideSensitive && "sensitive")}>
              {formatCustodyMsat(gap.source_total_msat, gap.asset)}
            </p>
          </div>
          <div className="rounded-lg border p-3">
            <p className="text-xs text-muted-foreground">{t("card.knownFee")}</p>
            <p className={cn("mt-1 font-medium", hideSensitive && "sensitive")}>
              {formatCustodyMsat(gap.source_fee_msat, gap.asset)}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("card.walletDebit", {
                amount: formatCustodyMsat(gap.source_debit_msat, gap.asset),
              })}
            </p>
          </div>
          <div className="rounded-lg border p-3">
            <p className="text-xs text-muted-foreground">
              {t("card.mayHaveReturned")}
            </p>
            <p className={cn("mt-1 font-medium", hideSensitive && "sensitive")}>
              {formatCustodyMsat(gap.return_total_msat, gap.asset)}
            </p>
          </div>
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
            <p className="text-xs text-muted-foreground">{t("card.residual")}</p>
            <p className={cn("mt-1 font-medium", hideSensitive && "sensitive")}>
              {formatCustodyMsat(gap.residual_msat, gap.asset)}
            </p>
          </div>
        </div>
        {expanded ? <GapReviewDetails gapId={gap.gap_id} /> : null}
      </CardContent>
    </Card>
  );
}

export function CustodyGaps() {
  const { t } = useTranslation("custodyGaps");
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const coverageQuery = useDaemon<CustodyCoverageSnapshot>(
    "ui.custody.coverage.snapshot",
  );
  const lineageQuery = useDaemonInfinite<CustodyLineageSnapshot>(
    "ui.custody.lineage.snapshot",
    { limit: 100 },
    (lastPage) => lastPage.data?.next_cursor ?? undefined,
  );
  const gapsQuery = useDaemonInfinite<CustodyGapSnapshot>(
    "ui.custody.gaps.list",
    { limit: 100 },
    (lastPage) => lastPage.data?.next_cursor ?? undefined,
  );
  const pageEnvelopes = gapsQuery.data?.pages ?? [];
  const pages = pageEnvelopes
    .map((page) => page.data)
    .filter((page): page is CustodyGapSnapshot => Boolean(page));
  const snapshot = pages[0];
  const gaps = collectCustodyGapPages(pages);
  const lineagePages = (lineageQuery.data?.pages ?? [])
    .map((page) => page.data)
    .filter((page): page is CustodyLineageSnapshot => Boolean(page));
  const lineageSnapshot = collectCustodyLineagePages(lineagePages);

  if (gapsQuery.isLoading) return <ScreenSkeleton titleWidth="w-48" />;
  if (gapsQuery.isError || !snapshot) {
    const firstPage = pageEnvelopes[0];
    return (
      <div className={screenPanelClassName}>
        <Card className="gap-2 py-5">
          <CardHeader>
            <CardTitle>{t("unavailable.title")}</CardTitle>
            <CardDescription>
              {gapsQuery.error instanceof Error
                ? gapsQuery.error.message
                : firstPage?.error?.message ?? t("unavailable.body")}
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    );
  }

  const reviewGaps = gaps.filter(
    (gap) => gap.status === "needs_review" || gap.status === "conflicting",
  );
  const canonicalIssueCount = snapshot.summary.canonical_issue_count ?? 0;
  const canonicalAmounts = snapshot.summary.canonical_unresolved_by_asset ?? [];
  const canonicalUnquantified =
    snapshot.summary.canonical_unquantified_issue_count ?? 0;
  const candidateAmounts = snapshot.summary.candidate_residual_by_asset ?? [];
  const derivedStateCurrent = snapshot.summary.derived_state_current === true;
  const searchComplete = snapshot.summary.search_complete !== false;
  const canShowClear = canShowNoKnownCustodyGaps(snapshot, reviewGaps.length);

  return (
    <div className={screenShellClassName}>
      <div className={pageHeaderClassName}>
        <div>
          <div className="flex items-center gap-2">
            <Route className="size-5" aria-hidden="true" />
            <h1 className="text-xl font-semibold">{t("title")}</h1>
          </div>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
            {t("description")}
          </p>
        </div>
        <Badge variant="outline" className="w-fit gap-1.5">
          <ShieldCheck className="size-3.5" /> {t("localOnly")}
        </Badge>
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        <Card className="gap-2 py-4">
          <CardContent className="flex items-center gap-3 px-4">
            <TriangleAlert className="size-5 text-amber-600" />
            <div>
              <p className="text-2xl font-semibold">
                {snapshot.summary.needs_review}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("summary.needsReview")}
              </p>
            </div>
          </CardContent>
        </Card>
        <Card className="gap-2 py-4">
          <CardContent className="flex items-center gap-3 px-4">
            <WalletCards className="size-5 text-muted-foreground" />
            <div>
              <p className="text-2xl font-semibold">{snapshot.summary.total}</p>
              <p className="text-xs text-muted-foreground">
                {t("summary.detected")}
              </p>
            </div>
          </CardContent>
        </Card>
        <Card className="gap-2 py-4">
          <CardContent className="flex items-center gap-3 px-4">
            <Clock3 className="size-5 text-muted-foreground" />
            <div>
              <p
                className={cn(
                  "text-base font-semibold",
                  hideSensitive && "sensitive",
                )}
              >
                {canonicalIssueCount > 0 ? (
                  canonicalAmounts.length > 0 ? (
                    canonicalAmounts.map((item) => (
                      <span className="block" key={item.asset}>
                        {formatCustodyMsat(item.amount_msat, item.asset)}
                      </span>
                    ))
                  ) : (
                    t("summary.blockingIssueCount", { count: canonicalIssueCount })
                  )
                ) : candidateAmounts.length > 0 ? (
                  candidateAmounts.map((item) => (
                    <span className="block" key={item.asset}>
                      {formatCustodyMsat(item.amount_msat, item.asset)}
                    </span>
                  ))
                ) : (
                  formatCustodyMsat(0, "BTC")
                )}
              </p>
              <p className="text-xs text-muted-foreground">
                {canonicalIssueCount > 0
                  ? t("summary.canonicalUnresolved")
                  : t("summary.candidateResidual")}
              </p>
              {canonicalUnquantified > 0 ? (
                <p className="text-xs text-amber-700">
                  {t("summary.unquantified", { count: canonicalUnquantified })}
                </p>
              ) : null}
            </div>
          </CardContent>
        </Card>
      </div>

      {!derivedStateCurrent ? (
        <Card className="gap-3 border-amber-500/30 bg-amber-500/5 py-6">
          <CardHeader className="px-5">
            <CardTitle className="flex items-center gap-2">
              <Clock3 className="size-5 text-amber-600" />
              {t("processing.title")}
            </CardTitle>
            <CardDescription>{t("processing.body")}</CardDescription>
          </CardHeader>
        </Card>
      ) : null}

      {!searchComplete ? (
        <Card className="gap-3 border-amber-500/30 bg-amber-500/5 py-6">
          <CardHeader className="px-5">
            <CardTitle className="flex items-center gap-2">
              <TriangleAlert className="size-5 text-amber-600" />
              {t("searchIncomplete.title")}
            </CardTitle>
            <CardDescription>{t("searchIncomplete.body")}</CardDescription>
          </CardHeader>
        </Card>
      ) : null}

      {coverageQuery.data?.data ? (
        <CustodyCoverageTimeline snapshot={coverageQuery.data.data} />
      ) : coverageQuery.isLoading ? (
        <Card className="gap-2 py-5">
          <CardHeader className="px-5">
            <CardTitle className="text-base">{t("coverage.title")}</CardTitle>
            <CardDescription>{t("coverage.loading")}</CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <Card className="gap-2 py-5">
          <CardHeader className="px-5">
            <CardTitle className="text-base">{t("coverage.title")}</CardTitle>
            <CardDescription>{t("coverage.unavailable")}</CardDescription>
          </CardHeader>
        </Card>
      )}

      {lineageSnapshot ? (
        <>
          <CustodyLineageTimeline snapshot={lineageSnapshot} />
          {lineageQuery.hasNextPage ? (
            <div className="flex flex-col items-center gap-2">
              <p className="text-xs text-muted-foreground">
                {t("lineage.pagination.loaded", {
                  loaded: lineageSnapshot.items.length,
                  total: lineageSnapshot.summary.total_count,
                })}
              </p>
              <Button
                type="button"
                variant="outline"
                disabled={lineageQuery.isFetchingNextPage}
                onClick={() => void lineageQuery.fetchNextPage()}
              >
                {lineageQuery.isFetchingNextPage
                  ? t("lineage.pagination.loading")
                  : t("lineage.pagination.loadMore")}
              </Button>
            </div>
          ) : null}
        </>
      ) : lineageQuery.isLoading ? (
        <Card className="gap-2 py-5">
          <CardHeader className="px-5">
            <CardTitle className="text-base">{t("lineage.title")}</CardTitle>
            <CardDescription>{t("lineage.loading")}</CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <Card className="gap-2 py-5">
          <CardHeader className="px-5">
            <CardTitle className="text-base">{t("lineage.title")}</CardTitle>
            <CardDescription>{t("lineage.unavailable")}</CardDescription>
          </CardHeader>
        </Card>
      )}

      {reviewGaps.length ? (
        <section
          className="space-y-3"
          aria-labelledby="custody-gaps-review-title"
        >
          <div>
            <h2 id="custody-gaps-review-title" className="font-semibold">
              {t("reviewQueue.title")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {t("reviewQueue.description")}
            </p>
          </div>
          {reviewGaps.map((gap) => (
            <CustodyGapCard key={gap.gap_id} gap={gap} />
          ))}
        </section>
      ) : derivedStateCurrent && canonicalIssueCount > 0 ? (
        <Card className="gap-3 border-amber-500/30 bg-amber-500/5 py-6">
          <CardHeader className="px-5">
            <CardTitle className="flex items-center gap-2">
              <TriangleAlert className="size-5 text-amber-600" />
              {t("blocking.title")}
            </CardTitle>
            <CardDescription>
              {t("blocking.body", { count: canonicalIssueCount })}
            </CardDescription>
          </CardHeader>
        </Card>
      ) : canShowClear ? (
        <Card className="items-center gap-3 py-10 text-center">
          <CheckCircle2 className="size-8 text-emerald-600" />
          <CardHeader className="max-w-xl px-5">
            <CardTitle>{t("empty.title")}</CardTitle>
            <CardDescription>{t("empty.body")}</CardDescription>
          </CardHeader>
        </Card>
      ) : null}
      {gapsQuery.hasNextPage ? (
        <div className="flex flex-col items-center gap-2">
          <p className="text-xs text-muted-foreground">
            {t("pagination.loaded", {
              loaded: gaps.length,
              total: snapshot.summary.total,
            })}
          </p>
          <Button
            type="button"
            variant="outline"
            disabled={gapsQuery.isFetchingNextPage}
            onClick={() => void gapsQuery.fetchNextPage()}
          >
            {gapsQuery.isFetchingNextPage
              ? t("pagination.loading")
              : t("pagination.loadMore")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}
