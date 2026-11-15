/**
 * Pieces the unlink and link dialogs share: a labelled section, the fee
 * between a linked move's legs, and what a previewed change does to the book.
 */
import * as React from "react";
import { AlertTriangle, CheckCircle2, Info, Loader2, RotateCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { reviewTaxDelta, type ReviewArtifact } from "@/components/ai/reviewWorkflow";
import { blurClass } from "@/components/transactions/model";
import { Button } from "@/components/ui/button";
import { formatFiatAmount } from "@/lib/currency";
import { maskSensitive, useSensitive } from "@/lib/sensitive";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { bpsText, checkFee, feeShareBps, type FeeCheck } from "./linkModel";
import type { MoveOutcome, PreviewState } from "./reviewPreview";
import { formatMsatAsSats } from "./satsFormat";

/** A labelled block in a dialog, set like the cause panels' sections. */
export function DialogSection({ title, children, className }: { title: string; children: React.ReactNode; className?: string }) {
  return (
    <section className={cn("space-y-1.5", className)}>
      <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
      <div className="text-sm">{children}</div>
    </section>
  );
}

/** The fee between the two legs of a linked move, with a note when it does not look like one. */
export function FeeLine({ sendMsat, feeMsat }: { sendMsat: bigint | null; feeMsat: bigint | null }) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  if (feeMsat === null) return <p className="text-muted-foreground">{t("transfersPage.fee.unknown")}</p>;
  const check: FeeCheck = sendMsat === null ? "plausible" : checkFee(sendMsat, feeMsat);
  const share = sendMsat === null ? null : feeShareBps(sendMsat, feeMsat);
  const amount = formatMsatAsSats((feeMsat < 0n ? -feeMsat : feeMsat).toString());
  return (
    <div className="space-y-1">
      <p className={cn("tabular-nums", blurClass(hideSensitive))}>
        {check === "negative"
          ? t("transfersPage.fee.more", { amount })
          : check === "none"
            ? t("transfersPage.fee.none")
            : check === "mismatch"
              ? t("transfersPage.fee.mismatch", { amount, share: share === null ? "—" : bpsText(share) })
              : t("transfersPage.fee.amount", { amount, share: share === null ? "—" : share === 0n ? "<0.01" : bpsText(share) })}
      </p>
      {check === "negative" || check === "high" || check === "mismatch" ? (
        <p className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300" data-testid="fee-flag">
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          {t(`transfersPage.fee.${check}LinkedFlag`)}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The dry run recalculates the whole book, which can take a while: say so,
 * with the time it has taken, so a long run does not read as a hang.
 */
function PreviewChecking({ startedAt }: { startedAt: number | null }) {
  const { t } = useTranslation("review");
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const seconds = startedAt === null ? 0 : Math.max(0, Math.floor((now - startedAt) / 1000));
  return (
    <div className="space-y-1" role="status" data-testid="preview-checking">
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 shrink-0 animate-spin" aria-hidden="true" />
        <span>{t("transfersPage.effects.checking")}</span>
        {/* Not read out every second: the time is for the eye. */}
        {seconds >= 3 ? (
          <span className="tabular-nums" aria-hidden="true">
            {t("transfersPage.effects.elapsed", { count: seconds })}
          </span>
        ) : null}
      </p>
      {seconds >= 10 ? <p className="pl-6 text-xs text-muted-foreground">{t("transfersPage.effects.checkingLong")}</p> : null}
    </div>
  );
}

/**
 * How each tax year moves, one short sentence per figure that changes
 * ("2025: € 806.03 less gain."). The figures are the core's, after minus
 * before; nothing is computed here.
 */
function TaxSentences({ artifact }: { artifact: ReviewArtifact }) {
  const { t } = useTranslation("review");
  const hidden = useSensitive();
  const delta = reviewTaxDelta(artifact);
  if (!delta) return null;
  const { tax, years } = delta;
  const money = (value: number) => maskSensitive(formatFiatAmount(Math.abs(value), tax.fiat_currency), hidden);
  const sentences: string[] = [];
  for (const year of years) {
    const gain = Number(year.realized_gain_loss);
    const income = Number(year.income);
    const cents = typeof year.estimated_tax_eur_cents === "number" ? year.estimated_tax_eur_cents : 0;
    if (Number.isFinite(gain) && gain !== 0) {
      sentences.push(t(gain > 0 ? "transfersPage.effects.tax.gainUp" : "transfersPage.effects.tax.gainDown", { year: year.year, amount: money(gain) }));
    }
    if (Number.isFinite(income) && income !== 0) {
      sentences.push(t(income > 0 ? "transfersPage.effects.tax.incomeUp" : "transfersPage.effects.tax.incomeDown", { year: year.year, amount: money(income) }));
    }
    if (cents !== 0) {
      sentences.push(t(cents > 0 ? "transfersPage.effects.tax.taxUp" : "transfersPage.effects.tax.taxDown", { year: year.year, amount: money(cents / 100) }));
    }
  }
  if (sentences.length === 0) sentences.push(t("transfersPage.effects.tax.none"));
  return (
    <>
      {sentences.map((sentence) => (
        <li key={sentence} className="flex items-start gap-2" data-testid="preview-tax">
          <Info className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="tabular-nums">{sentence}</span>
        </li>
      ))}
    </>
  );
}

/** One plain sentence per effect: what the change does, whether reports wait, how the tax years move. */
export function PreviewOutcomeView({
  state,
  outcome,
  lines,
  reportsSaid = false,
  waiting,
  refusal,
  onRetry,
}: {
  state: PreviewState;
  outcome: MoveOutcome | null;
  /** The sentences that say what this change does, already worded for it. */
  lines: string[];
  /** One of `lines` already says that reports wait, so it is not said again. */
  reportsSaid?: boolean;
  /** Shown instead of the run while something else has to finish first. */
  waiting?: string | null;
  /** The plain reason behind a refusal the caller recognises; the core's own words follow it. */
  refusal?: string | null;
  /** Runs the preview again after it failed. */
  onRetry?: () => void;
}) {
  const { t } = useTranslation("review");
  if (waiting) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status" data-testid="preview-waiting">
        <Loader2 className="size-4 shrink-0 animate-spin" aria-hidden="true" />
        {waiting}
      </p>
    );
  }
  if (state.status === "checking" || state.status === "idle") {
    return <PreviewChecking startedAt={state.status === "checking" ? state.startedAt : null} />;
  }
  if (state.status === "failed") {
    return (
      <div className="space-y-1.5" role="alert" data-testid="preview-failed">
        {refusal ? (
          <>
            <p className="text-sm text-destructive">{refusal}</p>
            <p className="text-xs break-words text-muted-foreground">{state.message}</p>
          </>
        ) : (
          <p className="text-sm break-words text-destructive">{t("transfersPage.effects.failed", { message: state.message })}</p>
        )}
        {onRetry ? (
          <Button type="button" size="sm" variant="outline" className="h-7 gap-1.5" onClick={onRetry}>
            <RotateCw className="size-3.5" aria-hidden="true" />
            {t("transfersPage.effects.retry")}
          </Button>
        ) : null}
      </div>
    );
  }
  return (
    <ul className="space-y-1.5" data-testid="preview-outcome">
      {lines.map((line) => (
        <li key={line} className="flex items-start gap-2">
          <Info className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span>{line}</span>
        </li>
      ))}
      {outcome && !(reportsSaid && !outcome.reportsReadyAfter) ? (
        <li className="flex items-start gap-2">
          {outcome.reportsReadyAfter ? (
            <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
          ) : (
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
          )}
          <span>
            {outcome.reportsReadyAfter
              ? t("transfersPage.effects.reportsReady")
              : t("transfersPage.effects.reportsBlocked", { count: outcome.heldAfter })}
          </span>
        </li>
      ) : null}
      <TaxSentences artifact={state.artifact} />
    </ul>
  );
}
