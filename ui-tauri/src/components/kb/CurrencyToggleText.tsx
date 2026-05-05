import { type KeyboardEvent, type MouseEvent, type ReactNode } from "react";

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
  const currency = useUiStore((state) => state.currency);
  const setCurrency = useUiStore((state) => state.setCurrency);
  const nextCurrency = currency === "btc" ? "eur" : "btc";
  const toggle = () => setCurrency(nextCurrency);

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

  return (
    <span
      role="button"
      tabIndex={0}
      className={cn(
        "cursor-pointer rounded-sm underline-offset-2 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        className,
      )}
      aria-label={`Show amounts in ${nextCurrency === "btc" ? "bitcoin" : "fiat"}`}
      title={`Show amounts in ${nextCurrency === "btc" ? "bitcoin" : "fiat"}`}
      onClick={onClick}
      onKeyDown={onKeyDown}
    >
      {children}
    </span>
  );
}
