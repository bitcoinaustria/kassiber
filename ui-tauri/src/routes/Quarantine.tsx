import { ShieldAlert } from "lucide-react";

import { ReviewDataTable } from "@/components/kb/ReviewDataTable";
import { MOCK_QUARANTINE_EVENTS } from "@/mocks/review";

export function Quarantine() {
  return (
    <ReviewDataTable
      kind="quarantine"
      eyebrow="Review · blocked records"
      title="Quarantine"
      description="Resolve under-specified transactions, missing prices, and manual-pair decisions that are being held out of trusted reports."
      icon={ShieldAlert}
      rows={MOCK_QUARANTINE_EVENTS}
    />
  );
}
