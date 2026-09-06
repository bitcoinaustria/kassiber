/** A review click applies to a current, fully rendered disclosure preview only. */
export function canApproveSourceFundsPreview(input: {
  current: boolean;
  exportable: boolean;
  diagramLoading: boolean;
  diagramError: boolean;
  fingerprint?: string;
}): boolean {
  return input.current && input.exportable && !input.diagramLoading && !input.diagramError && Boolean(input.fingerprint);
}

/** Navigation history and saved exports never approve a changed current case. */
export function isReviewedSourceFundsPreview(current: boolean, exportable: boolean, fingerprint: string | undefined, reviewedFingerprint: string | null): boolean {
  return current && exportable && Boolean(fingerprint) && fingerprint === reviewedFingerprint;
}
