import { useId, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";

interface CurrencyToggleTextProps {
  children: ReactNode;
  className?: string;
  stopPropagation?: boolean;
}

export function CurrencyToggleText({
  children,
  className,
  stopPropagation = true,
}: CurrencyToggleTextProps) {
  const { t } = useTranslation("chrome");
  const actionId = useId();
  const currency = useUiStore((state) => state.currency);
  const setCurrency = useUiStore((state) => state.setCurrency);
  const nextCurrency = currency === "btc" ? "eur" : "btc";
  const toggle = () => setCurrency(nextCurrency);
  const showLabel =
    nextCurrency === "btc"
      ? t("currencyToggleText.showBitcoin")
      : t("currencyToggleText.showFiat");

  const onClick = (event: MouseEvent<HTMLSpanElement>) => {
    if (stopPropagation) event.stopPropagation();
    toggle();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (stopPropagation) event.stopPropagation();
    toggle();
  };

  // The amount names the control, so a screen reader still reads it out; the
  // switch a press makes is its description.
  return (
    <span
      role="button"
      tabIndex={0}
      className={cn(
        "cursor-pointer rounded-sm underline-offset-2 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        className,
      )}
      aria-describedby={actionId}
      title={showLabel}
      onClick={onClick}
      onKeyDown={onKeyDown}
    >
      {children}
      <span id={actionId} hidden>
        {showLabel}
      </span>
    </span>
  );
}
