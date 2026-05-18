/**
 * Native file picker wrapper.
 *
 * Uses the Tauri dialog plugin when running inside the desktop shell so the
 * user gets a real OS file picker that returns an absolute path the daemon
 * can open. In Vite dev bridge mode, a loopback-only bridge endpoint opens
 * the same kind of native picker from the local dev server. Outside those
 * local runtimes the picker is unavailable and callers fall back to text input.
 */

export interface FilePickerOptions {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
  directory?: boolean;
  defaultPath?: string;
}

const FILE_PICKER_BRIDGE_PATH = "/__kassiber__/pick-file";

const isTauriRuntime =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const isDevBridgeRuntime =
  typeof window !== "undefined" && import.meta.env.DEV && !isTauriRuntime;

export const isFilePickerAvailable = isTauriRuntime || isDevBridgeRuntime;

async function pickFileViaDevBridge(
  options: FilePickerOptions,
): Promise<string | null> {
  const response = await fetch(FILE_PICKER_BRIDGE_PATH, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(options),
  });
  if (!response.ok) return null;
  const payload = (await response.json()) as {
    path?: unknown;
    error?: unknown;
  };
  if (payload.error) {
    throw new Error(String(payload.error));
  }
  return typeof payload.path === "string" && payload.path ? payload.path : null;
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
 * Pick multiple files. Returns an array of absolute paths, or `null` when the
 * runtime can't open a native picker. In dev-bridge mode falls back to the
 * single-file bridge for now (returns a one-element array).
 */
export async function pickFiles(
  options: FilePickerOptions = {},
): Promise<string[] | null> {
  if (!isFilePickerAvailable) return null;
  if (isDevBridgeRuntime) {
    const one = await pickFileViaDevBridge(options);
    return one ? [one] : null;
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
  return null;
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
