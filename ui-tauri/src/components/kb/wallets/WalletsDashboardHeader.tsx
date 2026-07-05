import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
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
    <div className="flex justify-end">
      <div className={pageHeaderActionsClassName}>
        <Button size="sm" className={headerActionClassName} onClick={onAddWallet}>
          <Plus className="size-4" aria-hidden="true" />
          {t("dashboard.addWallet")}
        </Button>
      </div>
    </div>
  );
}
