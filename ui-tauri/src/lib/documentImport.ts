export function buildDocumentImportPreviewArgs(
  documentToken: string,
  provider: string,
  model: string,
  pages = "",
): Record<string, unknown> {
  const normalizedModel = model.trim();
  const normalizedPages = pages.trim();
  return {
    document_token: documentToken,
    provider,
    ...(normalizedModel ? { model: normalizedModel } : {}),
    ...(normalizedPages ? { pages: normalizedPages } : {}),
  };
}

export function buildDocumentImportArgs(
  documentToken: string,
  wallet: string,
  selectedRowIds: Iterable<string>,
): Record<string, unknown> {
  return {
    document_token: documentToken,
    wallet,
    selected_row_ids: Array.from(selectedRowIds),
  };
}

export interface DocumentImportReadiness {
  hasDraft: boolean;
  wallet: string;
  selectedCount: number;
  pickerBusy: boolean;
  previewPending: boolean;
  importPending: boolean;
}

export function canImportDocumentDraft({
  hasDraft,
  wallet,
  selectedCount,
  pickerBusy,
  previewPending,
  importPending,
}: DocumentImportReadiness) {
  return (
    hasDraft &&
    Boolean(wallet) &&
    selectedCount > 0 &&
    !pickerBusy &&
    !previewPending &&
    !importPending
  );
}
