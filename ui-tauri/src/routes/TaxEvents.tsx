import { CircleDollarSign } from "lucide-react";

import { ReviewDataTable } from "@/components/kb/ReviewDataTable";
import { MOCK_TAX_EVENTS } from "@/mocks/review";

export function TaxEvents() {
  return (
    <ReviewDataTable
      kind="tax-events"
      eyebrow="Review · taxable ledger"
      title="Tax Events"
      description="Review normalized tax events before journals and reports are trusted. Rows stay focused on classification, basis, and tax impact."
      icon={CircleDollarSign}
      rows={MOCK_TAX_EVENTS}
    />
  );
}
