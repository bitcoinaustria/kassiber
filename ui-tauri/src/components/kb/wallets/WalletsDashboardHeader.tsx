import { Plus, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const headerActionClassName = "h-9 min-w-[112px] justify-center gap-2";

interface WalletsDashboardHeaderProps {
  isSyncing: boolean;
  onAddWallet: () => void;
  onRefreshAll: () => void;
}

export function WalletsDashboardHeader({
  isSyncing,
  onAddWallet,
  onRefreshAll,
}: WalletsDashboardHeaderProps) {
  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:items-end sm:justify-between">
      <div className="min-w-0 space-y-1">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Wallets and sources
        </p>
        <h2 className="text-xl font-semibold tracking-tight">Wallets</h2>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          className={headerActionClassName}
          onClick={onRefreshAll}
          disabled={isSyncing}
        >
          <RefreshCw
            className={cn("size-4", isSyncing && "animate-spin")}
            aria-hidden="true"
          />
          {isSyncing ? "Refreshing" : "Refresh book"}
        </Button>
        <Button size="sm" className={headerActionClassName} onClick={onAddWallet}>
          <Plus className="size-4" aria-hidden="true" />
          Add wallet
        </Button>
      </div>
    </div>
  );
}
