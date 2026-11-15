/**
 * One side of a move as the Transfers dialogs show it: the wallet with its
 * rail, when, how much, and the transaction id to copy or open. The same
 * card in the unlink dialog and on both sides of "Link manually", so a leg
 * reads the same wherever the owner decides about it.
 */
import type * as React from "react";
import { ArrowUpRight } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { CopyButton } from "@/components/kb/CopyButton";
import { blurClass } from "@/components/transactions/model";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import type { LinkLeg } from "./linkModel";
import { dateTime, hasOpenPart, shortId } from "./linkModel";
import { BtcFigure, LegAmount, WalletName } from "./figures";
import { formatMsatAsBtc, formatMsatAsSats } from "./satsFormat";

export function MoveLeg({
  label,
  leg,
  loading = false,
  fallback,
  className,
  onOpen,
  action,
}: {
  /** "Sent" or "Received". */
  label: string;
  leg: LinkLeg | null;
  loading?: boolean;
  /** What is known before the leg is read: the wallet and amount from the list. */
  fallback?: { wallet: string; rail: LinkLeg["rail"]; amountMsat: string | number | bigint | null; occurredAt: string | null };
  className?: string;
  /** Called before the transaction opens, so a dialog can close first. */
  onOpen?: () => void;
  /** A control in the card's corner, such as choosing another transaction. */
  action?: React.ReactNode;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const wallet = leg?.wallet ?? fallback?.wallet ?? "—";
  const rail = leg?.rail ?? fallback?.rail ?? "onchain";
  const occurredAt = leg?.occurredAt ?? fallback?.occurredAt ?? null;
  const amount = leg ? leg.amountMsat : (fallback?.amountMsat ?? null);
  const txid = leg?.externalId ?? null;
  return (
    <div className={cn("min-w-0 rounded-lg border bg-muted/20 px-3 py-2.5", className)} data-testid="move-leg">
      <div className="flex items-center justify-between gap-2 text-2xs text-muted-foreground sm:text-xs">
        <span className="font-medium tracking-wide uppercase">{label}</span>
        <span className="flex items-center gap-2">
          <span className="tabular-nums">{dateTime(occurredAt)}</span>
          {action}
        </span>
      </div>
      <div className="mt-1.5 flex min-w-0 items-center gap-1.5">
        <RailIcon rail={rail} size="compact" />
        <WalletName name={wallet} className="text-sm font-medium" />
      </div>
      <p className="mt-1 text-base font-semibold tabular-nums">
        {leg ? (
          <LegAmount leg={leg} />
        ) : amount === null ? (
          "—"
        ) : (
          <span className={blurClass(hideSensitive)}>
            <BtcFigure msat={amount} />
          </span>
        )}
      </p>
      {/* Linked in part: what a further link can still take. */}
      {leg && hasOpenPart(leg) && leg.open ? (
        <p className={cn("text-2xs text-amber-700 tabular-nums sm:text-xs dark:text-amber-300", blurClass(hideSensitive))} data-testid="move-leg-part">
          {t("transfersPage.link.partLeft", {
            left: formatMsatAsBtc(leg.open.unlinkedMsat.toString()),
            total: formatMsatAsBtc((leg.open.linkedMsat + leg.open.unlinkedMsat).toString()),
          })}
        </p>
      ) : null}
      {leg?.feeMsat ? (
        <p className={cn("text-2xs text-muted-foreground tabular-nums sm:text-xs", blurClass(hideSensitive))}>
          {t("transfersPage.leg.networkFee", { amount: formatMsatAsSats(leg.feeMsat.toString()) })}
        </p>
      ) : null}
      <div className="mt-1.5 flex min-h-6 items-center gap-1 text-2xs text-muted-foreground sm:text-xs">
        {loading ? (
          <Skeleton className="h-4 w-32" />
        ) : txid ? (
          <>
            <span className={cn("font-mono", blurClass(hideSensitive))} title={hideSensitive ? undefined : txid}>
              {shortId(txid)}
            </span>
            <CopyButton value={txid} variant="ghost" className="size-6" ariaLabel={t("transfersPage.leg.copyTxid")} />
          </>
        ) : (
          <span>{t("transfersPage.leg.noTxid")}</span>
        )}
        {leg ? (
          <Link
            to="/transactions"
            search={{ tx: leg.id }}
            onClick={onOpen}
            className="ml-auto inline-flex items-center gap-0.5 rounded-sm font-medium text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {t("transfersPage.showInTransactions")}
            <ArrowUpRight className="size-3" aria-hidden="true" />
          </Link>
        ) : null}
      </div>
    </div>
  );
}
