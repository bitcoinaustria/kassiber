import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import type * as React from "react";
import { useTranslation } from "react-i18next";

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { basisIncompleteFromMs } from "@/lib/fiatCompleteness";
import type { FiatCompleteness } from "@/mocks/seed";

import { basisGapCauses, formatTreasuryDetailDate } from "./model";

/** The date the cost basis stops being exact, or null when it never was. */
function basisGapDate(completeness: FiatCompleteness): string | null {
  const from = basisIncompleteFromMs(completeness);
  return from !== null && Number.isFinite(from)
    ? formatTreasuryDetailDate(new Date(from).toISOString().slice(0, 10))
    : null;
}

/**
 * Says what an incomplete cost basis means and what causes it, each cause
 * linking to the page that resolves it. Shared by the chart's boundary label
 * and the chart header's amber note so both explain the same thing.
 */
export function BasisGapPopover({
  completeness,
  align = "start",
  children,
}: {
  completeness: FiatCompleteness;
  align?: "start" | "center" | "end";
  /** The trigger; rendered as the popover's button. */
  children: React.ReactNode;
}) {
  const { t } = useTranslation("overview");
  const date = basisGapDate(completeness);
  const causes = basisGapCauses(completeness);
  return (
    <Popover>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent align={align} className="w-88 space-y-3 text-sm">
        <p className="font-medium">{t("treasury.basisIncomplete")}</p>
        <p className="text-xs leading-5 text-muted-foreground">
          {date
            ? t("treasury.basisGap.bodyFrom", { date })
            : t("treasury.basisGap.body")}
        </p>
        {causes.length ? (
          <ul className="divide-y rounded-md border text-xs">
            {causes.map((cause) => (
              <li
                key={cause.key}
                className="flex items-center justify-between gap-3 px-3 py-2"
              >
                {/* dynamic key */}
                <span>{t(cause.copy.key as never, cause.copy.params)}</span>
                <Link
                  to={cause.href}
                  className="inline-flex shrink-0 items-center gap-1 font-medium text-foreground underline-offset-4 hover:underline"
                >
                  {cause.href === "/quarantine"
                    ? t("treasury.basisGap.openQuarantine")
                    : t("treasury.basisGap.openJournals")}
                  <ArrowRight className="size-3" aria-hidden="true" />
                </Link>
              </li>
            ))}
          </ul>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
