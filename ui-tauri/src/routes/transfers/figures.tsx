/**
 * How the Transfers screen writes an amount and a wallet name, wherever
 * they appear: bitcoin exact to the sat with its leading zeros dimmed, the
 * display currency on request, and a long wallet name cut in the middle so
 * the words that tell two wallets apart stay.
 */
import { useTranslation } from "react-i18next";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { isBitcoinAsset } from "@/components/review/transferModel";
import { blurClass, formatDisplayMoney } from "@/components/transactions/model";
import { useCurrency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { splitWalletName } from "./linkedMovesModel";
import type { LinkLeg } from "./linkModel";
import { formatMsatAsAsset, formatMsatAsBtc } from "./satsFormat";

/** Bitcoin to the sat, the leading zeros of an amount under one coin dimmed. */
export function BtcFigure({ msat }: { msat: string | number | bigint }) {
  const text = formatMsatAsBtc(msat);
  const match = text.match(/^(₿0\.0*)(.+)$/);
  if (!match) return <>{text}</>;
  return (
    <>
      <span className="text-muted-foreground/70">{match[1]}</span>
      {match[2]}
    </>
  );
}

/**
 * A leg's amount: exact in its own asset, or its value in the display
 * currency when that is chosen and known for this leg (an unpriced leg says
 * so). Pressing it switches the currency, like the amounts in the table.
 */
export function LegAmount({
  leg,
  className,
  toggle = true,
}: {
  leg: Pick<LinkLeg, "amountMsat" | "asset" | "fiat">;
  className?: string;
  /** Off where the amount sits in a row that is itself the control. */
  toggle?: boolean;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const bitcoin = isBitcoinAsset(leg.asset);
  const exact = bitcoin ? <BtcFigure msat={leg.amountMsat} /> : formatMsatAsAsset(leg.amountMsat, leg.asset, false);
  const shown =
    currency === "btc" || leg.fiat === undefined
      ? exact
      : leg.fiat === null
        ? t("transfersPage.linked.unpriced")
        : formatDisplayMoney(leg.fiat, 0, currency);
  const classes = cn("whitespace-nowrap tabular-nums", blurClass(hideSensitive), className);
  return toggle ? <CurrencyToggleText className={classes}>{shown}</CurrencyToggleText> : <span className={classes}>{shown}</span>;
}

/** A wallet name on one line, cut in the middle when it does not fit. */
export function WalletName({ name, className }: { name: string; className?: string }) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [head, tail] = splitWalletName(name);
  return (
    <span className={cn("flex min-w-0 overflow-hidden", blurClass(hideSensitive), className)} title={hideSensitive ? undefined : name}>
      {/* The start keeps a few letters; the end gives way only after it. */}
      <span className={cn("truncate", tail && "min-w-[2.5em]")}>{head}</span>
      {tail ? <span className="min-w-0 shrink-[0.01] truncate whitespace-pre">{tail}</span> : null}
    </span>
  );
}
