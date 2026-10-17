import type { QuarantineExportBlock } from "@/lib/quarantineExportBlock";

export interface ReportExportStatus {
  year: number;
  tone: "success" | "warning" | "error";
  message: string;
  /** What the file leaves out or could not reconcile, kept beside it. */
  notes?: string[];
  path?: string;
  openPath?: string;
  /** Set when quarantined transactions stopped the export. */
  quarantine?: QuarantineExportBlock;
}

export function reportExportStatusForYear(
  status: ReportExportStatus | null,
  year: number,
): ReportExportStatus | null {
  return status?.year === year ? status : null;
}
