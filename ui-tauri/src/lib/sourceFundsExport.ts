export type SourceFundsExportPreview = {
  case?: {
    id?: string | null;
  } | null;
} | null | undefined;

export type SourceFundsExportArgs = {
  case: string;
};

export function sourceFundsExportArgs(
  report: SourceFundsExportPreview,
): SourceFundsExportArgs | null {
  const caseId = report?.case?.id;
  if (!caseId) return null;
  return { case: caseId };
}
