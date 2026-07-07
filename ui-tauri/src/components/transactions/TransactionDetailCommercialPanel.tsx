import { ExternalLink } from "lucide-react";
import type * as React from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { openExternalUrl } from "@/daemon/transport";
import { cn } from "@/lib/utils";

import {
  LedgerRow,
  type CommercialBtcpayMatch,
  type CommercialContextData,
} from "./TransactionDetailSheetParts";

function commercialOriginLabel(
  origin: CommercialBtcpayMatch["origin"],
  t: (key: string) => string,
) {
  if (!origin) return t("commercial.originLabel.unknown");
  const labelKeys: Record<string, string> = {
    pos: "commercial.originLabel.pos",
    app: "commercial.originLabel.app",
    external_order: "commercial.originLabel.externalOrder",
    payment_request: "commercial.originLabel.paymentRequest",
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
      className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-sm text-left text-foreground underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-label={ariaLabel}
      onClick={() => {
        void openExternalUrl(url).catch((error) => {
          console.error("Failed to open BTCPay URL", error);
        });
      }}
    >
      <span className="truncate">{children}</span>
      <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
    </button>
  );
}

export function CommercialProvenancePanel({
  context,
  loading,
  hidden,
}: {
  context?: CommercialContextData;
  loading?: boolean;
  hidden?: boolean;
}) {
  const { t } = useTranslation("transactions");
  const btcpay = context?.btcpay ?? [];
  const documents = context?.documents ?? [];
  if (loading) {
    return (
      <div className="overflow-hidden rounded-md border">
        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          {t("commercial.title")}
        </div>
        <div className="px-3 py-3 text-sm text-muted-foreground">{t("commercial.loading")}</div>
      </div>
    );
  }
  if (!btcpay.length && !documents.length) {
    return (
      <div className="overflow-hidden rounded-md border">
        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          {t("commercial.title")}
        </div>
        <div className="px-3 py-3 text-sm text-muted-foreground">
          {t("commercial.empty")}
        </div>
      </div>
    );
  }
  return (
    <div className="overflow-hidden rounded-md border">
      <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {t("commercial.title")}
      </div>
      {btcpay.map((match) => {
        const payment = match.payment;
        const invoice = match.invoice;
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
              label={t("commercial.btcpayPayment")}
              value={
                <span className={cn("truncate", hidden && "sensitive")}>
                  {payment?.payment_id || t("commercial.linked")}
                </span>
              }
              muted={match.link.state !== "reviewed"}
            />
            <LedgerRow
              label={payment ? t("commercial.paidInvoice") : t("commercial.invoice")}
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
            {match.origin ? (
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
                  <Badge variant="secondary" className="rounded-md">
                    {translatedCommercialToken(
                      "commercial.linkState",
                      match.link.state,
                      t as (key: string) => string,
                    )}
                  </Badge>
                  {match.link.reconciliation_state &&
                  match.link.reconciliation_state !== "unreviewed" ? (
                    <Badge variant="outline" className="rounded-md">
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
          </div>
        );
      })}
      {documents.length ? (
        <div className="border-t bg-muted/20 px-3 py-2">
          <div className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
            {t("commercial.documents")}
          </div>
          <div className="flex flex-wrap gap-1.5">
            {documents.map((document) => (
              <Badge key={document.id} variant="outline" className="rounded-md">
                <span className={cn("max-w-48 truncate", hidden && "sensitive")}>
                  {document.label}
                </span>
              </Badge>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
