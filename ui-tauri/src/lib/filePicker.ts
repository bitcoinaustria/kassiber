/**
 * Native file picker wrapper.
 *
 * Uses the Tauri dialog plugin when running inside the desktop shell so the
 * user gets a real OS file picker that returns an absolute path the daemon
 * can open. In Vite browser bridge mode, a loopback-only bridge endpoint opens
 * the same kind of native picker from the local dev server. Outside those
 * local runtimes the picker is unavailable and callers fall back to text input.
 * Saving remains Tauri-only; browser builds use their existing download/save
 * fallbacks because the bridge deliberately does not accept destination paths.
 *
 * Contract:
 *   - `null` (single) / `[]` (multi) means the user cancelled, or the runtime
 *     has no picker available (see `isFilePickerAvailable`). Both cases are
 *     "no path returned, no error".
 *   - Anything else throws — failed bridge fetch, non-2xx response, Tauri
 *     error, or a server-side `{ error: "…" }` payload. Callers that care
 *     about the difference between cancel and failure should try/catch.
 *
 * Document OCR is the exception to the path-returning contract: its dedicated
 * helper stages the native selection inside the daemon and returns only an
 * opaque session token plus display metadata.
 */

import { DAEMON_MODE } from "@/daemon/transport";

export interface FilePickerOptions {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
  /**
   * Pick a directory instead of a file. Honored by the Tauri picker and the
   * browser bridge (osascript `choose folder`, zenity `--directory`).
   */
  directory?: boolean;
  defaultPath?: string;
}

export interface PickedFileWithContents {
  path: string;
  contentsBase64: string;
}

export interface DocumentImportSourceSelection {
  document_token: string;
  attachment_id?: string;
  transaction_id?: string;
  source: {
    filename: string;
    media_type?: string;
    size_bytes?: number;
    kind?: "image" | "pdf" | string;
  };
}

const FILE_PICKER_BRIDGE_PATH = "/__kassiber__/pick-file";

const isTauriRuntime =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const isBridgeRuntime =
  typeof window !== "undefined" &&
  DAEMON_MODE === "bridge" &&
  !isTauriRuntime;

export const isFilePickerAvailable = isTauriRuntime || isBridgeRuntime;
export const isFileSaveAvailable = isTauriRuntime;

async function callFilePickerBridge(
  body: Record<string, unknown>,
): Promise<{
  path?: unknown;
  paths?: unknown;
  contentsBase64?: unknown;
  documentImportSource?: unknown;
  error?: unknown;
}> {
  const response = await fetch(FILE_PICKER_BRIDGE_PATH, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(
      `file picker bridge returned ${response.status} ${response.statusText}`,
    );
  }
  return (await response.json()) as {
    path?: unknown;
    paths?: unknown;
    contentsBase64?: unknown;
    documentImportSource?: unknown;
    error?: unknown;
  };
}

function documentImportSourceSelection(
  value: unknown,
): DocumentImportSourceSelection | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const candidate = value as {
    document_token?: unknown;
    source?: { filename?: unknown };
  };
  if (
    typeof candidate.document_token !== "string" ||
    !candidate.document_token ||
    !candidate.source ||
    typeof candidate.source.filename !== "string" ||
    !candidate.source.filename
  ) {
    throw new Error("document picker returned an invalid session");
  }
  return value as DocumentImportSourceSelection;
}

export async function pickDocumentImportSource(): Promise<DocumentImportSourceSelection | null> {
  if (!isFilePickerAvailable) return null;
  if (isBridgeRuntime) {
    const payload = await callFilePickerBridge({ purpose: "document_import" });
    if (payload.error) throw new Error(String(payload.error));
    return documentImportSourceSelection(payload.documentImportSource);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return documentImportSourceSelection(
    await invoke<unknown>("pick_document_import_source"),
  );
}

/**
 * Pick a file for the assistant to analyze, returning only a staging token.
 *
 * Same contract as `pickDocumentImportSource` — the renderer never learns the
 * path, and only the native picker can mint the grant — with a wider filter,
 * since an assistant attachment is usually a CSV/XLSX export from an exchange
 * that has no dedicated importer.
 */
export async function pickChatAttachmentSource(options?: {
  expected_scope?: { workspace_id: string; profile_id: string };
  review_case_id?: string;
  review_recipe?: Record<string, unknown>;
  expected_review_fingerprint?: string;
}): Promise<DocumentImportSourceSelection | null> {
  if (!isFilePickerAvailable) return null;
  if (isBridgeRuntime) {
    const payload = await callFilePickerBridge({
      purpose: "chat_attachment", ...options,
    });
    if (payload.error) throw new Error(String(payload.error));
    return documentImportSourceSelection(payload.documentImportSource);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return documentImportSourceSelection(
    await invoke<unknown>("pick_chat_attachment_source", {
      expectedScope: options?.expected_scope,
      reviewCaseId: options?.review_case_id,
      ...(options?.review_recipe === undefined ? {} : { reviewRecipe: options.review_recipe }),
      ...(options?.expected_review_fingerprint === undefined ? {} : { expectedReviewFingerprint: options.expected_review_fingerprint }),
    }),
  );
}

async function pickFileViaDevBridge(
  options: FilePickerOptions,
): Promise<string | null> {
  const payload = await callFilePickerBridge({ ...options, multiple: false });
  if (payload.error) {
    throw new Error(String(payload.error));
  }
  return typeof payload.path === "string" && payload.path ? payload.path : null;
}

async function pickFileWithContentsViaDevBridge(
  options: FilePickerOptions,
): Promise<PickedFileWithContents | null> {
  const payload = await callFilePickerBridge({
    ...options,
    multiple: false,
    includeContentsBase64: true,
  });
  if (payload.error) {
    throw new Error(String(payload.error));
  }
  return typeof payload.path === "string" &&
    payload.path &&
    typeof payload.contentsBase64 === "string"
    ? { path: payload.path, contentsBase64: payload.contentsBase64 }
    : null;
}

async function pickFilesViaDevBridge(
  options: FilePickerOptions,
): Promise<string[]> {
  const payload = await callFilePickerBridge({ ...options, multiple: true });
  if (payload.error) {
    throw new Error(String(payload.error));
  }
  if (!Array.isArray(payload.paths)) return [];
  return payload.paths.filter(
    (entry): entry is string => typeof entry === "string" && entry.length > 0,
  );
}

export async function pickFile(
  options: FilePickerOptions = {},
): Promise<string | null> {
  if (!isFilePickerAvailable) return null;
  if (isBridgeRuntime) {
    return pickFileViaDevBridge(options);
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selection = await open({
    multiple: false,
    directory: options.directory ?? false,
    title: options.title,
    filters: options.filters,
  });
  if (typeof selection === "string") return selection;
  return null;
}

export async function pickFileWithContentsBase64(
  options: FilePickerOptions = {},
): Promise<PickedFileWithContents | null> {
  if (!isFilePickerAvailable) return null;
  if (isBridgeRuntime) {
    return pickFileWithContentsViaDevBridge(options);
  }
  const selected = await pickFile(options);
  if (!selected) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  const contentsBase64 = await invoke<string>("read_ledger_preview_file_base64", {
    path: selected,
  });
  return { path: selected, contentsBase64 };
}

/**
 * Pick one or more files. Returns the selected paths, or `[]` when the user
 * cancels or no picker is available. Errors throw.
 */
export async function pickFiles(
  options: FilePickerOptions = {},
): Promise<string[]> {
  if (!isFilePickerAvailable) return [];
  if (isBridgeRuntime) {
    return pickFilesViaDevBridge(options);
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selection = await open({
    multiple: true,
    directory: options.directory ?? false,
    title: options.title,
    filters: options.filters,
  });
  if (Array.isArray(selection)) {
    return selection.filter((path): path is string => typeof path === "string");
  }
  if (typeof selection === "string") return [selection];
  return [];
}

export async function saveFile(
  options: FilePickerOptions = {},
): Promise<string | null> {
  if (!isFileSaveAvailable) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const selection = await save({
    title: options.title,
    filters: options.filters,
    defaultPath: options.defaultPath,
  });
  if (typeof selection === "string") return selection;
  return null;
}
