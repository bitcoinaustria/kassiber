import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";

const headerActionClassName = cn(
  pageHeaderActionClassName,
  "min-w-[112px] justify-center",
);

interface WalletsDashboardHeaderProps {
  onAddWallet: () => void;
}

export function WalletsDashboardHeader({
  onAddWallet,
}: WalletsDashboardHeaderProps) {
  const { t } = useTranslation("connections");
  return (
    <div className={pageHeaderClassName}>
      <div className="min-w-0 space-y-1">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("dashboard.eyebrow")}
        </p>
        <h2 className="text-xl font-semibold tracking-tight">
          {t("dashboard.title")}
        </h2>
      </div>
      <div className={pageHeaderActionsClassName}>
        <Button size="sm" className={headerActionClassName} onClick={onAddWallet}>
          <Plus className="size-4" aria-hidden="true" />
          {t("dashboard.addWallet")}
        </Button>
      </div>
    </div>
  );
}
