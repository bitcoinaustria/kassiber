export interface ReportExportStatus {
  year: number;
  tone: "success" | "error";
  message: string;
  path?: string;
  openPath?: string;
}

export function reportExportStatusForYear(
  status: ReportExportStatus | null,
  year: number,
): ReportExportStatus | null {
  return status?.year === year ? status : null;
}
