import { Link } from "@tanstack/react-router";
import { ListChecks, Loader2, RefreshCw, TableProperties } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

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
        className="h-9"
        onClick={onOpenResolvePlan}
        disabled={resolvePlanCount === 0}
      >
        <ListChecks className="size-4" aria-hidden="true" />
        {t("quarantine.resolvePlan.button")}
      </Button>
      <Button asChild variant="outline" className="h-9">
        <Link to="/transactions">
          <TableProperties className="size-4" aria-hidden="true" />
          {t("quarantine.actions.transactions")}
        </Link>
      </Button>
      <Button
        type="button"
        className="h-9"
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
