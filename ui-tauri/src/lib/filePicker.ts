/**
 * Native file picker wrapper.
 *
 * Uses the Tauri dialog plugin when running inside the desktop shell so the
 * user gets a real OS file picker that returns an absolute path the daemon
 * can open. Outside Tauri (Vite dev with the bridge transport, or vitest)
 * the picker is unavailable and callers fall back to the path text input.
 */

export interface FilePickerOptions {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
  directory?: boolean;
}

export const isFilePickerAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function pickFile(
  options: FilePickerOptions = {},
): Promise<string | null> {
  if (!isFilePickerAvailable) return null;
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
