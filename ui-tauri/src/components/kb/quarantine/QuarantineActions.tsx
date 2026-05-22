import { Link } from "@tanstack/react-router";
import { Loader2, RefreshCw, TableProperties } from "lucide-react";

import { Button } from "@/components/ui/button";

interface QuarantineActionsProps {
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
}

export function QuarantineActions({
  isProcessingJournals,
  onProcessJournals,
}: QuarantineActionsProps) {
  return (
    <>
      <Button asChild variant="outline" className="h-9">
        <Link to="/transactions">
          <TableProperties className="size-4" aria-hidden="true" />
          Transactions
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
        Process journals
      </Button>
    </>
  );
}
