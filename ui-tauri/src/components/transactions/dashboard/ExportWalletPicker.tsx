import { Search } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

export interface ExportWallet {
  id: string;
  label: string;
  kind?: string;
  chain?: string;
  transaction_count?: number;
}

/** Above this many wallets the list gets a filter field. */
const FILTER_THRESHOLD = 6;

/**
 * Wallets an export covers, as a checklist. Selection is stored as the set of
 * excluded wallet ids, so "everything checked" stays "every wallet" even when
 * a wallet is added while the dialog is open.
 */
export function ExportWalletPicker({
  wallets,
  loading,
  excluded,
  onExcludedChange,
}: {
  wallets: ExportWallet[];
  loading: boolean;
  excluded: ReadonlySet<string>;
  onExcludedChange: (next: Set<string>) => void;
}) {
  const { t } = useTranslation("transactions");
  const [query, setQuery] = React.useState("");
  const needle = query.trim().toLocaleLowerCase();
  const visible = needle
    ? wallets.filter((wallet) =>
        [wallet.label, wallet.kind, wallet.chain]
          .filter(Boolean)
          .some((value) => String(value).toLocaleLowerCase().includes(needle)),
      )
    : wallets;
  // A kind tag only tells wallets apart when the kinds differ.
  const mixedKinds = new Set(wallets.map((wallet) => wallet.kind ?? "")).size > 1;
  const selectedCount = wallets.filter((wallet) => !excluded.has(wallet.id)).length;
  const selectedTransactions = wallets
    .filter((wallet) => !excluded.has(wallet.id))
    .reduce((sum, wallet) => sum + (wallet.transaction_count ?? 0), 0);

  const setVisible = (include: boolean) => {
    const next = new Set(excluded);
    for (const wallet of visible) {
      if (include) next.delete(wallet.id);
      else next.add(wallet.id);
    }
    onExcludedChange(next);
  };

  const toggle = (id: string, include: boolean) => {
    const next = new Set(excluded);
    if (include) next.delete(id);
    else next.add(id);
    onExcludedChange(next);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-sm font-medium">{t("dashboard.export.wallets")}</span>
        <span className="truncate text-xs tabular-nums text-muted-foreground">
          {t("dashboard.export.walletsCount", {
            selected: selectedCount,
            total: wallets.length,
          })}
          {" · "}
          {t("dashboard.export.selectionTransactions", {
            count: selectedTransactions,
            formatted: formatCount(selectedTransactions),
          })}
        </span>
      </div>
      {wallets.length > FILTER_THRESHOLD ? (
        <div className="relative">
          <Search
            className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("dashboard.export.filterWallets")}
            aria-label={t("dashboard.export.filterWallets")}
            className="h-8 pl-8 text-sm"
          />
        </div>
      ) : null}
      <div className="flex items-center gap-1 text-xs">
        <button
          type="button"
          onClick={() => setVisible(true)}
          className="rounded-sm px-1.5 py-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          {t("dashboard.export.selectAll")}
        </button>
        <button
          type="button"
          onClick={() => setVisible(false)}
          className="rounded-sm px-1.5 py-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          {t("dashboard.export.selectNone")}
        </button>
      </div>
      <div
        role="group"
        aria-label={t("dashboard.export.wallets")}
        className="min-h-24 flex-1 overflow-y-auto rounded-lg border bg-background/70"
      >
        {loading ? (
          <div className="grid gap-2 p-3">
            {Array.from({ length: 4 }, (_, index) => (
              <Skeleton key={index} className="h-5" />
            ))}
          </div>
        ) : visible.length ? (
          visible.map((wallet) => {
            const included = !excluded.has(wallet.id);
            const detail = [mixedKinds ? wallet.kind : null, wallet.chain].filter(Boolean).join(" · ");
            return (
              <label
                key={wallet.id}
                className={cn(
                  "flex cursor-pointer items-center gap-2.5 border-b px-3 py-1.5 text-sm transition-colors last:border-b-0 hover:bg-muted/50",
                  included ? undefined : "text-muted-foreground",
                )}
              >
                <Checkbox
                  checked={included}
                  onCheckedChange={(value) => toggle(wallet.id, value === true)}
                  aria-label={wallet.label}
                />
                <span className="min-w-0 flex-1 truncate">
                  {wallet.label}
                  {detail ? (
                    <span className="ml-2 text-xs text-muted-foreground">{detail}</span>
                  ) : null}
                </span>
                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {formatCount(wallet.transaction_count ?? 0)}
                </span>
              </label>
            );
          })
        ) : (
          <p className="p-3 text-xs text-muted-foreground">{t("dashboard.export.noWalletMatch")}</p>
        )}
      </div>
    </div>
  );
}
