import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

const headerActionClassName = "h-9 min-w-[112px] justify-center gap-2";

interface WalletsDashboardHeaderProps {
  onAddWallet: () => void;
}

export function WalletsDashboardHeader({
  onAddWallet,
}: WalletsDashboardHeaderProps) {
  const { t } = useTranslation("connections");
  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:items-end sm:justify-between">
      <div className="min-w-0 space-y-1">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("dashboard.eyebrow")}
        </p>
        <h2 className="text-xl font-semibold tracking-tight">
          {t("dashboard.title")}
        </h2>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" className={headerActionClassName} onClick={onAddWallet}>
          <Plus className="size-4" aria-hidden="true" />
          {t("dashboard.addWallet")}
        </Button>
      </div>
    </div>
  );
}
