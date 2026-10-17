import { ArrowUpRight, ShieldAlert } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { currentUiLocale } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import type { QuarantineExportBlock } from "@/lib/quarantineExportBlock";

const QUARANTINE_PREVIEW = 5;

function shortDate(value: string | undefined): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), { dateStyle: "medium" }).format(parsed);
}

function shortTxid(value: string | undefined): string {
  if (!value) return "";
  return value.length > 20 ? `${value.slice(0, 10)}…${value.slice(-6)}` : value;
}

/**
 * An export stopped by quarantined transactions: the first few blockers and a
 * way to review them. Shared by the transaction Export dialog and the Reports
 * year handoff.
 */
export function QuarantineExportBlockNotice({
  block,
  onReview,
  className,
}: {
  block: QuarantineExportBlock;
  onReview: () => void;
  className?: string;
}) {
  const { t } = useTranslation("transactions");
  return (
    <div
      role="alert"
      className={cn(
        "grid gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm",
        className,
      )}
    >
      <div className="flex items-start gap-2.5">
        <ShieldAlert className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
        <div className="min-w-0">
          <p className="font-medium">{t("dashboard.export.blocked.title", { count: block.count })}</p>
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
            {t("dashboard.export.blocked.body")}
          </p>
        </div>
      </div>
      <ul className="grid gap-1 text-xs">
        {block.transactions.slice(0, QUARANTINE_PREVIEW).map((row) => (
          <li
            key={row.transaction_ref ?? row.transaction_id}
            className="grid grid-cols-[auto_1fr_auto] items-baseline gap-x-3 rounded-md bg-background/60 px-2 py-1.5"
          >
            <span className="text-muted-foreground tabular-nums">{shortDate(row.occurred_at)}</span>
            <span className="min-w-0 truncate">
              <span className="font-medium">{row.wallet}</span>
              <span className="ml-2 font-mono text-muted-foreground">
                {shortTxid(row.transaction_id || row.transaction_ref)}
              </span>
            </span>
            <span className="text-muted-foreground">{row.reason}</span>
          </li>
        ))}
      </ul>
      {block.count > QUARANTINE_PREVIEW ? (
        <p className="text-xs text-muted-foreground">
          {t("dashboard.export.blocked.more", { count: block.count - QUARANTINE_PREVIEW })}
        </p>
      ) : null}
      <div>
        <Button type="button" size="sm" variant="outline" onClick={onReview}>
          <ArrowUpRight className="size-4" aria-hidden="true" />
          {t("dashboard.export.blocked.review")}
        </Button>
      </div>
    </div>
  );
}
