import * as React from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { openExternalUrl } from "@/daemon/transport";
import { currentUiLocale } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import {
  LedgerRow,
  type CommercialBtcpayFreshness,
  type CommercialBtcpayMatch,
  type CommercialContextData,
  type CommercialPayoutBatch,
} from "./TransactionDetailSheetParts";

function commercialOriginLabel(
  origin: CommercialBtcpayMatch["origin"],
  t: (key: string) => string,
) {
  if (!origin) return t("commercial.originLabel.unknown");
  const labelKeys: Record<string, string> = {
    pos: "commercial.originLabel.pos",
    crowdfund: "commercial.originLabel.crowdfund",
    app: "commercial.originLabel.app",
    ecommerce: "commercial.originLabel.ecommerce",
    external_order: "commercial.originLabel.externalOrder",
    payment_request: "commercial.originLabel.paymentRequest",
    refund: "commercial.originLabel.refund",
    pull_payment: "commercial.originLabel.pullPayment",
    store_payout: "commercial.originLabel.storePayout",
  };
  const key = labelKeys[origin.kind];
  return key ? t(key) : origin.kind.replace(/_/g, " ");
}

function translatedCommercialToken(
  prefix: string,
  value: string,
  t: (key: string) => string,
) {
  const normalized = value.trim().toLowerCase().replace(/[-\s]+/g, "_");
  if (!normalized) return "";
  const key = `${prefix}.${normalized}`;
  const translated = t(key);
  return translated === key
    ? normalized.replace(/_/g, " ")
    : translated;
}

export type CommercialReviewDecision = {
  state: "reviewed" | "rejected" | "suggested";
  commercial_kind?: "income" | "expense" | "refund" | "transfer" | "none";
};

type ReviewKind = NonNullable<CommercialReviewDecision["commercial_kind"]>;

const PAYMENT_REVIEW_KINDS: readonly ReviewKind[] = ["income", "transfer", "none"];
const PAYOUT_REVIEW_KINDS: readonly ReviewKind[] = ["refund", "expense", "transfer", "none"];

function reviewKindsFor(match: CommercialBtcpayMatch): readonly ReviewKind[] {
  return match.payment?.record_type === "payout" ? PAYOUT_REVIEW_KINDS : PAYMENT_REVIEW_KINDS;
}

function defaultReviewKind(match: CommercialBtcpayMatch): ReviewKind {
  if (match.payment?.record_type !== "payout") return "income";
  return match.payment.origin_kind === "refund" ? "refund" : "expense";
}

function CommercialReviewActions({
  match,
  batch,
  pending,
  onReview,
}: {
  match: CommercialBtcpayMatch;
  batch?: CommercialPayoutBatch | null;
  pending?: boolean;
  onReview: (linkId: string, decision: CommercialReviewDecision) => void;
}) {
  const { t } = useTranslation("transactions");
  const [kind, setKind] = React.useState<ReviewKind>(() => defaultReviewKind(match));
  const record = match.payment ?? match.invoice;
  // One send can pay several payouts; the core reviews and prices them together.
  const inBatch = Boolean(
    batch && batch.size > 1 && batch.link_ids.includes(match.link.id),
  );
  const priced = inBatch
    ? Boolean(batch?.fiat_value_exact && batch.fiat_currency)
    : Boolean(record?.fiat_value_exact && record.fiat_currency);
  const priceCurrency = inBatch ? batch?.fiat_currency : record?.fiat_currency;
  if (match.link.state === "reviewed") {
    return (
      <div className="flex flex-wrap items-center gap-2 px-3 py-2">
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={pending}
          onClick={() => onReview(match.link.id, { state: "suggested" })}
        >
          {t("commercial.review.reopen")}
        </Button>
        <span className="text-xs text-muted-foreground">
          {inBatch
            ? `${t("commercial.review.batchReopen", { count: batch?.size ?? 0 })} `
            : ""}
          {t("commercial.review.reopenEffect")}
        </span>
      </div>
    );
  }
  if (match.link.state !== "suggested") return null;
  const effect = [
    inBatch ? t("commercial.review.batchEffect", { count: batch?.size ?? 0 }) : null,
    inBatch && batch?.mixed_currencies ? t("commercial.review.batchMixedCurrencies") : null,
    priced
      ? t("commercial.review.effectPrice", { currency: priceCurrency ?? "" })
      : null,
    kind === "income" || kind === "expense" || kind === "refund"
      ? t("commercial.review.effectKind", {
          kind: t(`commercial.commercialKind.${kind}`),
        })
      : t("commercial.review.effectKeepKind"),
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <div className="space-y-2 border-t bg-muted/20 px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <select
          aria-label={t("commercial.review.kindLabel")}
          className="h-8 rounded-md border border-input bg-background px-2 text-xs"
          value={kind}
          disabled={pending}
          onChange={(event) => setKind(event.target.value as ReviewKind)}
        >
          {reviewKindsFor(match).map((value) => (
            <option key={value} value={value}>
              {t(`commercial.commercialKind.${value}`)}
            </option>
          ))}
        </select>
        <Button
          type="button"
          size="sm"
          disabled={pending}
          onClick={() =>
            onReview(match.link.id, { state: "reviewed", commercial_kind: kind })
          }
        >
          {t("commercial.review.confirm")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={pending}
          onClick={() => onReview(match.link.id, { state: "rejected" })}
        >
          {t("commercial.review.reject")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{effect}</p>
    </div>
  );
}

function formatFreshnessDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

/** The least current store behind this transaction's BTCPay records. */
function oldestFreshness(entries: readonly CommercialBtcpayFreshness[] | undefined) {
  if (!entries?.length) return null;
  return [...entries].sort((a, b) => {
    const rank = (entry: CommercialBtcpayFreshness) =>
      entry.never_synced ? 0 : entry.last_error_code ? 1 : entry.stale ? 2 : 3;
    if (rank(a) !== rank(b)) return rank(a) - rank(b);
    return String(a.last_success_at ?? "").localeCompare(String(b.last_success_at ?? ""));
  })[0];
}

function BtcpayFreshnessNote({ entries }: { entries?: CommercialBtcpayFreshness[] }) {
  const { t } = useTranslation("transactions");
  const entry = oldestFreshness(entries);
  if (!entry) return null;
  const when = entry.last_success_at ? formatFreshnessDate(entry.last_success_at) : "";
  const text = entry.never_synced
    ? t("commercial.freshness.never")
    : entry.last_error_code
      ? t("commercial.freshness.failed", { when })
      : entry.stale
        ? t("commercial.freshness.stale", { when })
        : t("commercial.freshness.asOf", { when });
  const warn = entry.never_synced || Boolean(entry.last_error_code) || entry.stale;
  return (
    <p
      className={cn(
        "border-b px-3 py-1.5 text-xs",
        warn
          ? "bg-amber-500/10 text-amber-900 dark:text-amber-200"
          : "text-muted-foreground",
      )}
    >
      {text}
    </p>
  );
}

function compactMiddle(value: string, head = 12, tail = 10) {
  const trimmed = value.trim();
  if (trimmed.length <= head + tail + 3) return trimmed;
  return `${trimmed.slice(0, head)}...${trimmed.slice(-tail)}`;
}

function ExternalCommercialValue({
  children,
  url,
  hidden,
  ariaLabel,
}: {
  children: React.ReactNode;
  url?: string;
  hidden?: boolean;
  ariaLabel: string;
}) {
  if (!url || hidden) {
    return <span className={cn("truncate", hidden && "sensitive")}>{children}</span>;
  }
  return (
    <button
      type="button"
      className="inline-flex min-w-0 max-w-full items-center rounded-sm text-left text-foreground underline decoration-muted-foreground/70 underline-offset-2 hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-label={ariaLabel}
      onClick={() => {
        void openExternalUrl(url).catch((error) => {
          console.error("Failed to open BTCPay URL", error);
        });
      }}
    >
      <span className="truncate">{children}</span>
    </button>
  );
}

export function CommercialProvenancePanel({
  context,
  loading,
  hidden,
  onReview,
  reviewPending,
  reviewError,
}: {
  context?: CommercialContextData;
  loading?: boolean;
  hidden?: boolean;
  onReview?: (linkId: string, decision: CommercialReviewDecision) => void;
  reviewPending?: boolean;
  reviewError?: string | null;
}) {
  const { t } = useTranslation("transactions");
  const btcpay = context?.btcpay ?? [];
  if (!btcpay.length) {
    return null;
  }
  if (loading) {
    return (
      <div className="overflow-hidden rounded-md border">
        <div className="border-b bg-muted px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t("commercial.title")}
        </div>
        <div className="px-3 py-3 text-sm text-muted-foreground">{t("commercial.loading")}</div>
      </div>
    );
  }
  return (
    <div className="overflow-hidden rounded-md border">
      <div className="border-b bg-muted px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        {t("commercial.title")}
      </div>
      <BtcpayFreshnessNote entries={context?.btcpay_freshness} />
      {btcpay.map((match) => {
        const payment = match.payment;
        const invoice = match.invoice;
        const isPayout = payment?.record_type === "payout";
        const originDuplicatesPaymentRequest =
          match.origin?.kind === "payment_request" && Boolean(match.payment_request);
        const showLinkState = match.link.state !== "reviewed";
        const invoiceId =
          invoice?.invoice_id || payment?.invoice_id || t("commercial.unknown");
        const paymentRequestLabel =
          match.payment_request?.label || match.payment_request?.id || "";
        const originLabel = match.origin ? (
          <>
            {commercialOriginLabel(
              match.origin,
              t as (key: string) => string, // loose translator
            )}
            {match.origin.label ? ` · ${match.origin.label}` : ""}
          </>
        ) : null;
        return (
          <div key={match.link.id} className="border-b last:border-b-0">
            <LedgerRow
              label={isPayout ? t("commercial.btcpayPayout") : t("commercial.btcpayPayment")}
              value={
                <span
                  className={cn("truncate font-mono text-xs", hidden && "sensitive")}
                  title={payment?.payment_id || undefined}
                >
                  {payment?.payment_id
                    ? compactMiddle(payment.payment_id)
                    : t("commercial.linked")}
                </span>
              }
              muted={match.link.state !== "reviewed"}
            />
            <LedgerRow
              label={
                isPayout
                  ? t("commercial.refundedInvoice")
                  : payment
                    ? t("commercial.paidInvoice")
                    : t("commercial.invoice")
              }
              value={
                <span className={cn("truncate", hidden && "sensitive")}>
                  {invoiceId}
                </span>
              }
            />
            {match.payment_request ? (
              <LedgerRow
                label={t("commercial.paymentRequest")}
                value={
                  <ExternalCommercialValue
                    url={match.payment_request.url}
                    hidden={hidden}
                    ariaLabel={t("commercial.openPaymentRequest")}
                  >
                    {paymentRequestLabel}
                  </ExternalCommercialValue>
                }
              />
            ) : null}
            {match.origin && !originDuplicatesPaymentRequest ? (
              <LedgerRow
                label={t("commercial.origin")}
                value={
                  <ExternalCommercialValue
                    url={match.origin.url}
                    hidden={hidden}
                    ariaLabel={t("commercial.openOrigin")}
                  >
                    {originLabel}
                  </ExternalCommercialValue>
                }
              />
            ) : null}
            <LedgerRow
              label={t("commercial.reconciliation")}
              value={
                <span className="inline-flex min-w-0 flex-wrap items-center gap-1.5">
                  {showLinkState ? (
                    <Badge variant="secondary" className="rounded-md">
                      {translatedCommercialToken(
                        "commercial.linkState",
                        match.link.state,
                        t as (key: string) => string,
                      )}
                    </Badge>
                  ) : null}
                  {match.link.reconciliation_state &&
                  match.link.reconciliation_state !== "unreviewed" ? (
                    <Badge
                      variant={showLinkState ? "outline" : "secondary"}
                      className="rounded-md"
                    >
                      {translatedCommercialToken(
                        "commercial.reconciliationState",
                        match.link.reconciliation_state,
                        t as (key: string) => string,
                      )}
                    </Badge>
                  ) : null}
                  {match.link.commercial_kind ? (
                    <Badge variant="outline" className="rounded-md">
                      <span className={cn(hidden && "sensitive")}>
                        {translatedCommercialToken(
                          "commercial.commercialKind",
                          match.link.commercial_kind,
                          t as (key: string) => string,
                        )}
                      </span>
                    </Badge>
                  ) : null}
                </span>
              }
            />
            {onReview ? (
              <CommercialReviewActions
                match={match}
                batch={context?.payout_batch}
                pending={reviewPending}
                onReview={onReview}
              />
            ) : null}
          </div>
        );
      })}
      {reviewError ? (
        <p className="border-t px-3 py-2 text-xs text-destructive" role="alert">
          {reviewError}
        </p>
      ) : null}
    </div>
  );
}
