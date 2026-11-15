/**
 * Unlinking a move, with what the owner needs to decide: both legs to the
 * sat with their transaction ids, the fee between them, how and when it was
 * linked, what changes in the book, and how to undo it.
 *
 * An automatic link (exchange or Bark evidence) is undone through the
 * review core: the dialog shows the core's dry run (which side is held,
 * whether reports wait, how each tax year moves) and applies exactly that
 * run, journals included. The review core cannot dry-run removing an
 * ordinary pair, so for a pair the dialog says plainly what the core does
 * today: each side counts on its own, a sale and a purchase, unless Kassiber
 * has its own proof that both are the owner's. A link the core derives from
 * one transaction seen by two wallets cannot be undone; the dialog explains
 * it instead.
 */
import * as React from "react";
import { ArrowRight, Link2, Loader2, Unlink, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDaemonMutation } from "@/daemon/client";
import { blurClass } from "@/components/transactions/model";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { lineageEvidenceLabel } from "../lineageEvidence";
import { AUTOMATIC_UNDO_DAYS, AUTOMATIC_UNDO_REASON } from "../custodyGapsModel";
import { fixedLinkKind, type LinkedMove, type TransferPairSummary } from "./linkedMovesModel";
import { DialogSection, FeeLine, PreviewOutcomeView } from "./MoveEffects";
import { dateTime, hasOpenPart, kindKey, withOpenPart } from "./linkModel";
import { MoveLeg } from "./MoveLeg";
import { appliedId, moveOutcome, useReviewPreview, type ReviewOperation } from "./reviewPreview";
import { formatMsatAsBtc } from "./satsFormat";
import { useMoveLeg } from "./useMoveLeg";
import { useOpenParts } from "./useOpenParts";

/** Pair sources that mean "linked from a suggestion in one step". */
const BULK_SOURCES = new Set(["bulk_exact", "bulk_selected"]);

/** "details" shows the move with an Unlink button; "unlink" asks to confirm it. */
export type MoveDialogIntent = "details" | "unlink";

export type UnlinkResult =
  | { kind: "pair"; pair: TransferPairSummary | null; move: LinkedMove }
  | { kind: "automatic"; dismissalId: string | null; move: LinkedMove };

function exactBigInt(value: string | number | bigint | null | undefined): bigint | null {
  if (value === null || value === undefined) return null;
  try {
    return BigInt(typeof value === "number" ? Math.trunc(value) : value);
  } catch {
    return null;
  }
}

export function UnlinkDialog({
  move,
  intent = "unlink",
  variant = "dialog",
  onRequestUnlink,
  onLinkRest,
  onClose,
  onUnlinked,
}: {
  move: LinkedMove | null;
  intent?: MoveDialogIntent;
  /** A dialog over the page, or a panel that stays beside the table. */
  variant?: "dialog" | "panel";
  /** From the details view, go on to unlinking this move. */
  onRequestUnlink?: () => void;
  /** Link the part of this send that no receipt covers yet. */
  onLinkRest?: (transactionId: string) => void;
  onClose: () => void;
  /** After the change; `differently` asks to choose another receipt next. */
  onUnlinked: (result: UnlinkResult, differently: boolean) => void;
}) {
  const { t } = useTranslation(["review", "custodyGaps"]);
  const { t: tGaps } = useTranslation("custodyGaps");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const parts = useOpenParts();
  const sendRead = useMoveLeg(move?.outTransactionId);
  const receiptRead = useMoveLeg(move?.inTransactionId);
  // Only a link the owner made can leave a part of its send for another link.
  const open = (leg: typeof sendRead.leg) => (leg && move?.pairId ? withOpenPart(leg, parts) : leg);
  const send = { ...sendRead, leg: open(sendRead.leg) };
  const receipt = { ...receiptRead, leg: open(receiptRead.leg) };
  const rest = send.leg && hasOpenPart(send.leg) ? send.leg : null;
  const unpair = useDaemonMutation<{ deleted?: string; pair?: TransferPairSummary }>("ui.transfers.unpair");
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => setError(null), [move?.key]);

  const mode: "automatic" | "pair" | "fixed" = !move
    ? "fixed"
    : move.pairId
      ? "pair"
      : move.undoable && move.lineage
        ? "automatic"
        : "fixed";
  const operations = React.useMemo<ReviewOperation[] | null>(
    () =>
      // Only asking to unlink runs the dry run; the details view just shows the move.
      move && mode === "automatic" && intent === "unlink"
        ? [
            {
              type: "dismiss_pair",
              out_transaction_id: move.outTransactionId,
              in_transaction_id: move.inTransactionId,
              reason: AUTOMATIC_UNDO_REASON,
              expires_in_days: AUTOMATIC_UNDO_DAYS,
            },
          ]
        : null,
    [intent, mode, move],
  );
  const preview = useReviewPreview(operations, t("review:transfersPage.effects.unavailable"));
  const outcome =
    move && preview.state.status === "ready" ? moveOutcome(preview.state.artifact, move.outTransactionId, move.inTransactionId) : null;
  const pending = unpair.isPending || preview.applying;

  if (!move) return variant === "panel" ? null : <Dialog open={false} />;

  const from = move.fromLabel;
  const to = move.toLabel;
  const sendMsat = send.leg ? send.leg.amountMsat : exactBigInt(move.amountMsat);
  const receiptMsat = receipt.leg ? receipt.leg.amountMsat : exactBigInt(move.inAmountMsat);
  const networkFee = send.leg?.feeMsat ?? 0n;
  // A send linked in part has no fee between these two to name: the rest is still open.
  const feeMsat = rest
    ? null
    : (exactBigInt(move.feeMsat) ??
      (sendMsat !== null && receiptMsat !== null && move.asset === (receipt.leg?.asset ?? move.asset) ? sendMsat + networkFee - receiptMsat : null));

  // How it was linked, in the owner's words.
  const how =
    mode === "pair"
      ? move.byRule
        ? t("review:transfersPage.unlink.howRule", { date: dateTime(move.linkedAt) })
        : move.pairSource && BULK_SOURCES.has(move.pairSource)
          ? t("review:transfersPage.unlink.howBulk", { date: dateTime(move.linkedAt) })
          : t("review:transfersPage.unlink.howYou", { date: dateTime(move.linkedAt) })
      : move.lineage
        ? t("review:transfersPage.unlink.howAutomatic", { reason: lineageEvidenceLabel(move.lineage, tGaps) })
        : t("review:transfersPage.unlink.howAutomaticPlain");
  const recordedAs =
    mode === "pair" && move.pairKind
      ? t("review:transfersPage.unlink.recordedAs", {
          kind: t(`review:transfersPage.kind.${kindKey(move.pairKind)}`),
          policy: t(`review:transfersPage.policy.${move.pairPolicy === "taxable" ? "taxable" : "carrying"}`),
        })
      : null;

  const automaticLines: string[] = [];
  if (outcome) {
    if (outcome.sendHeld || outcome.receiptHeld) {
      automaticLines.push(t("review:transfersPage.unlink.effectHeld", { from }));
    } else {
      automaticLines.push(t("review:transfersPage.unlink.effectBooked", { from, to }));
    }
  }

  const confirm = async (differently: boolean) => {
    setError(null);
    try {
      if (mode === "pair" && move.pairId) {
        const envelope = await unpair.mutateAsync({ pair_id: move.pairId });
        onUnlinked({ kind: "pair", pair: envelope.data?.pair ?? null, move }, differently);
      } else if (mode === "automatic") {
        const result = await preview.apply();
        onUnlinked({ kind: "automatic", dismissalId: appliedId(result, "dismissal_id"), move }, differently);
      }
    } catch (cause) {
      setError(cause instanceof Error && cause.message ? cause.message : t("review:transfersPage.undo.failed"));
    }
  };

  const ready = mode === "pair" || (mode === "automatic" && preview.state.status === "ready");
  const details = intent === "details";
  const amountFallback = (msat: bigint | null) => (msat === null ? null : msat);

  const title = details
    ? t("review:transfersPage.details.title")
    : mode === "fixed"
      ? t("review:transfersPage.unlink.titleFixed")
      : t("review:transfersPage.unlink.title");
  const description = t("review:transfersPage.unlink.subtitle", { from, to });
  // In a panel the two legs stack: it is narrower than the dialog.
  const legGrid =
    variant === "panel"
      ? "grid grid-cols-1 items-stretch gap-2"
      : "grid grid-cols-1 items-stretch gap-2 sm:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]";

  return (
    <MoveShell
      variant={variant}
      title={title}
      description={description}
      hideSensitive={hideSensitive}
      mode={mode}
      intent={intent}
      onClose={() => (pending ? null : onClose())}
    >
        <div className={legGrid}>
          <MoveLeg
            label={t("review:transfersPage.leg.sent")}
            leg={send.leg}
            loading={send.loading}
            fallback={{ wallet: from, rail: move.fromRail, amountMsat: amountFallback(exactBigInt(move.amountMsat)), occurredAt: move.occurredAt }}
            onOpen={onClose}
          />
          {variant === "panel" ? null : (
            <span className="hidden place-items-center text-muted-foreground sm:grid">
              <ArrowRight className="size-4" aria-hidden="true" />
            </span>
          )}
          <MoveLeg
            label={t("review:transfersPage.leg.received")}
            leg={receipt.leg}
            loading={receipt.loading}
            fallback={{ wallet: to, rail: move.toRail, amountMsat: amountFallback(exactBigInt(move.inAmountMsat)), occurredAt: move.inOccurredAt }}
            onOpen={onClose}
          />
        </div>

        <div className={cn("grid gap-4", variant === "dialog" && "sm:grid-cols-2")}>
          {rest ? (
            <DialogSection title={t("review:transfersPage.unlink.restTitle")}>
              <p className={cn("tabular-nums", blurClass(hideSensitive))} data-testid="unlink-rest">
                {t("review:transfersPage.linked.rest", { left: formatMsatAsBtc((rest.open?.unlinkedMsat ?? 0n).toString()) })}
              </p>
              {onLinkRest ? (
                <Button type="button" size="sm" className="mt-1.5 h-7 gap-1.5" disabled={pending} onClick={() => onLinkRest(rest.id)}>
                  <Link2 className="size-3.5" aria-hidden="true" />
                  {t("review:transfersPage.strip.linkRest")}
                </Button>
              ) : null}
            </DialogSection>
          ) : (
            <DialogSection title={t("review:transfersPage.unlink.feeTitle")}>
              <FeeLine sendMsat={sendMsat} feeMsat={feeMsat} />
            </DialogSection>
          )}
          <DialogSection title={t("review:transfersPage.unlink.howTitle")}>
            <p>{how}</p>
            {recordedAs ? <p className="text-muted-foreground">{recordedAs}</p> : null}
          </DialogSection>
        </div>

        {details && mode !== "fixed" ? null : (
        <DialogSection
          title={mode === "fixed" ? t("review:transfersPage.unlink.fixedTitle") : t("review:transfersPage.unlink.changesTitle")}
        >
          {mode === "automatic" ? (
            <PreviewOutcomeView state={preview.state} outcome={outcome} lines={automaticLines} />
          ) : mode === "pair" ? (
            <div className="space-y-1.5" data-testid="unlink-plain-effects">
              {/* A taxable link already counted as a sale and a purchase, so unlinking it moves no tax figure. */}
              {move.pairPolicy === "taxable" ? (
                <p>
                  {t("review:transfersPage.unlink.effectPairTaxable", {
                    kind: t(`review:transfersPage.kind.${kindKey(move.pairKind)}`),
                  })}
                </p>
              ) : (
                <>
                  <p className={blurClass(hideSensitive)}>{t("review:transfersPage.unlink.effectPair", { from, to })}</p>
                  <p className="text-muted-foreground">{t("review:transfersPage.unlink.effectPairWhy")}</p>
                </>
              )}
              <p className="text-xs text-muted-foreground">{t("review:transfersPage.unlink.noDryRun")}</p>
            </div>
          ) : (
            <p className="text-muted-foreground">
              {`${t(`review:transfersPage.linked.fixed.${fixedLinkKind(move)}`)} `}
              {t("review:transfersPage.unlink.fixedFix")}
            </p>
          )}
        </DialogSection>
        )}

        {mode !== "fixed" && !details ? (
          <DialogSection title={t("review:transfersPage.unlink.undoTitle")}>
            <p className="text-muted-foreground">
              {mode === "automatic" ? t("review:transfersPage.unlink.undoAutomatic") : t("review:transfersPage.unlink.undoPair")}
            </p>
          </DialogSection>
        ) : null}

        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
        <DialogFooter className={cn("gap-2", mode !== "fixed" && "sm:justify-between")}>
          {/* A panel closes with its own ×, so only a dialog repeats Close here. */}
          {mode === "fixed" ? (
            variant === "panel" ? null : (
              <Button type="button" variant="outline" onClick={onClose}>
                {t("review:transfersPage.unlink.close")}
              </Button>
            )
          ) : details ? (
            <>
              <Button type="button" variant={variant === "panel" ? "outline" : "ghost"} onClick={onRequestUnlink}>
                <Unlink className="size-4" aria-hidden="true" />
                {t("review:transfersPage.details.unlink")}
              </Button>
              {variant === "panel" ? null : (
                <Button type="button" variant="outline" onClick={onClose}>
                  {t("review:transfersPage.unlink.close")}
                </Button>
              )}
            </>
          ) : (
            <>
              <Button type="button" variant="ghost" disabled={pending || !ready} onClick={() => void confirm(true)}>
                {t("review:transfersPage.unlink.differently")}
              </Button>
              <div className="flex flex-col-reverse gap-2 sm:flex-row">
                <Button type="button" variant="outline" disabled={pending} onClick={onClose}>
                  {t("review:transfersPage.undo.cancel")}
                </Button>
                <Button type="button" variant="destructive" disabled={pending || !ready} onClick={() => void confirm(false)}>
                  {pending ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
                  {pending ? t("review:transfersPage.undo.undoing") : t("review:transfersPage.undo.confirm")}
                </Button>
              </div>
            </>
          )}
        </DialogFooter>
    </MoveShell>
  );
}

/**
 * The frame around a transfer's details: a dialog over the page, or a panel
 * beside the table that keeps the list in view (rotki pins its matching
 * panel next to the event list the same way:
 * `frontend/app/src/modules/history/events/use-pinned-match-panel.ts`).
 */
function MoveShell({
  variant,
  title,
  description,
  hideSensitive,
  mode,
  intent,
  onClose,
  children,
}: {
  variant: "dialog" | "panel";
  title: string;
  description: string;
  hideSensitive: boolean;
  mode: string;
  intent: MoveDialogIntent;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const { t } = useTranslation("review");
  if (variant === "panel") {
    return (
      <aside
        className="kb-surface flex max-h-[var(--kb-panel-room,calc(100vh-6rem))] flex-col gap-4 overflow-y-auto p-4 text-sm"
        aria-labelledby="transfer-panel-title"
        data-testid="unlink-dialog"
        data-variant="panel"
        data-mode={mode}
        data-intent={intent}
      >
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1 space-y-1">
            <h2 id="transfer-panel-title" className="text-base leading-tight font-semibold">
              {title}
            </h2>
            <p className={cn("text-muted-foreground", blurClass(hideSensitive))}>{description}</p>
          </div>
          <Button type="button" size="icon" variant="ghost" className="size-7 shrink-0" aria-label={t("transfersPage.unlink.close")} onClick={onClose}>
            <X className="size-4" aria-hidden="true" />
          </Button>
        </div>
        {children}
      </aside>
    );
  }
  return (
    <Dialog open onOpenChange={(open) => (!open ? onClose() : null)}>
      <DialogContent
        className="max-h-[90vh] overflow-y-auto sm:max-w-2xl"
        data-testid="unlink-dialog"
        data-variant="dialog"
        data-mode={mode}
        data-intent={intent}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription className={blurClass(hideSensitive)}>{description}</DialogDescription>
        </DialogHeader>
        {children}
      </DialogContent>
    </Dialog>
  );
}

