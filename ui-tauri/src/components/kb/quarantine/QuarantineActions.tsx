import { Link } from "@tanstack/react-router";
import { ListChecks, Loader2, RefreshCw, TableProperties } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { pageHeaderActionClassName } from "@/lib/screen-layout";

interface QuarantineActionsProps {
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
  onOpenResolvePlan: () => void;
  resolvePlanCount: number;
}

export function QuarantineActions({
  isProcessingJournals,
  onProcessJournals,
  onOpenResolvePlan,
  resolvePlanCount,
}: QuarantineActionsProps) {
  const { t } = useTranslation("journals");
  return (
    <>
      <Button
        type="button"
        variant="outline"
        className={pageHeaderActionClassName}
        onClick={onOpenResolvePlan}
        disabled={resolvePlanCount === 0}
      >
        <ListChecks className="size-4" aria-hidden="true" />
        {t("quarantine.resolvePlan.button")}
      </Button>
      <Button asChild variant="outline" className={pageHeaderActionClassName}>
        <Link to="/transactions">
          <TableProperties className="size-4" aria-hidden="true" />
          {t("quarantine.actions.transactions")}
        </Link>
      </Button>
      <Button
        type="button"
        className={pageHeaderActionClassName}
        onClick={onProcessJournals}
        disabled={isProcessingJournals}
      >
        {isProcessingJournals ? (
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        ) : (
          <RefreshCw className="size-4" aria-hidden="true" />
        )}
        {t("quarantine.actions.processJournals")}
      </Button>
    </>
  );
}
