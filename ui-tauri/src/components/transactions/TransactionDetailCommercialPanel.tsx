import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

import {
  LedgerRow,
  type CommercialBtcpayMatch,
  type CommercialContextData,
} from "./TransactionDetailSheetParts";

function commercialOriginLabel(origin: CommercialBtcpayMatch["origin"]) {
  if (!origin) return "Unknown";
  const labels: Record<string, string> = {
    pos: "BTCPay POS",
    app: "BTCPay app",
    external_order: "External order",
    payment_request: "Payment request",
  };
  return labels[origin.kind] ?? origin.kind.replace(/_/g, " ");
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
  const btcpay = context?.btcpay ?? [];
  const documents = context?.documents ?? [];
  if (loading) {
    return (
      <div className="overflow-hidden rounded-md border">
        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          Commercial provenance
        </div>
        <div className="px-3 py-3 text-sm text-muted-foreground">Loading…</div>
      </div>
    );
  }
  if (!btcpay.length && !documents.length) {
    return (
      <div className="overflow-hidden rounded-md border">
        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          Commercial provenance
        </div>
        <div className="px-3 py-3 text-sm text-muted-foreground">
          No linked BTCPay or document context.
        </div>
      </div>
    );
  }
  return (
    <div className="overflow-hidden rounded-md border">
      <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Commercial provenance
      </div>
      {btcpay.map((match) => {
        const payment = match.payment;
        const invoice = match.invoice;
        return (
          <div key={match.link.id} className="border-b last:border-b-0">
            <LedgerRow
              label="BTCPay payment"
              value={
                <span className={cn("truncate", hidden && "sensitive")}>
                  {payment?.payment_id || "Linked"}
                </span>
              }
              muted={match.link.state !== "reviewed"}
            />
            <LedgerRow
              label="Invoice"
              value={
                <span className={cn("truncate", hidden && "sensitive")}>
                  {invoice?.invoice_id || payment?.invoice_id || "Unknown"}
                </span>
              }
            />
            {match.payment_request ? (
              <LedgerRow
                label="Payment request"
                value={
                  <span className={cn("truncate", hidden && "sensitive")}>
                    {match.payment_request.label || match.payment_request.id}
                  </span>
                }
              />
            ) : null}
            {match.origin ? (
              <LedgerRow
                label="Origin"
                value={
                  <span className={cn("truncate", hidden && "sensitive")}>
                    {commercialOriginLabel(match.origin)}
                    {match.origin.label ? ` · ${match.origin.label}` : ""}
                  </span>
                }
              />
            ) : null}
            <LedgerRow
              label="Review"
              value={
                <span className="inline-flex min-w-0 items-center gap-1.5">
                  <Badge variant="secondary" className="rounded-md">
                    {match.link.state}
                  </Badge>
                  {match.link.commercial_kind ? (
                    <span className={cn("truncate", hidden && "sensitive")}>
                      {match.link.commercial_kind}
                    </span>
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
            Documents
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
