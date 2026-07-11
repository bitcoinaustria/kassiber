export function buildDocumentImportPreviewArgs(
  documentToken: string,
  provider: string,
  model: string,
): Record<string, unknown> {
  const normalizedModel = model.trim();
  return {
    document_token: documentToken,
    provider,
    ...(normalizedModel ? { model: normalizedModel } : {}),
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
