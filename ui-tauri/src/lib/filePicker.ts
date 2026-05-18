/**
 * Native file picker wrapper.
 *
 * Uses the Tauri dialog plugin when running inside the desktop shell so the
 * user gets a real OS file picker that returns an absolute path the daemon
 * can open. In Vite dev bridge mode, a loopback-only bridge endpoint opens
 * the same kind of native picker from the local dev server. Outside those
 * local runtimes the picker is unavailable and callers fall back to text input.
 *
 * Contract:
 *   - `null` (single) / `[]` (multi) means the user cancelled, or the runtime
 *     has no picker available (see `isFilePickerAvailable`). Both cases are
 *     "no path returned, no error".
 *   - Anything else throws — failed bridge fetch, non-2xx response, Tauri
 *     error, or a server-side `{ error: "…" }` payload. Callers that care
 *     about the difference between cancel and failure should try/catch.
 */

export interface FilePickerOptions {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
  /**
   * Pick a directory instead of a file. Honored by the Tauri picker and the
   * dev bridge (osascript `choose folder`, zenity `--directory`).
   */
  directory?: boolean;
  defaultPath?: string;
}

const FILE_PICKER_BRIDGE_PATH = "/__kassiber__/pick-file";

const isTauriRuntime =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const isDevBridgeRuntime =
  typeof window !== "undefined" && import.meta.env.DEV && !isTauriRuntime;

export const isFilePickerAvailable = isTauriRuntime || isDevBridgeRuntime;

async function callFilePickerBridge(
  body: Record<string, unknown>,
): Promise<{ path?: unknown; paths?: unknown; error?: unknown }> {
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
    error?: unknown;
  };
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
  if (isDevBridgeRuntime) {
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

/**
 * Pick one or more files. Returns the selected paths, or `[]` when the user
 * cancels or no picker is available. Errors throw.
 */
export async function pickFiles(
  options: FilePickerOptions = {},
): Promise<string[]> {
  if (!isFilePickerAvailable) return [];
  if (isDevBridgeRuntime) {
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
  if (!isFilePickerAvailable) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const selection = await save({
    title: options.title,
    filters: options.filters,
    defaultPath: options.defaultPath,
  });
  if (typeof selection === "string") return selection;
  return null;
}
