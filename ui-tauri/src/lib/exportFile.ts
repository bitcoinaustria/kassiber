import { canSaveExportedFiles, saveExportedFileAs } from "@/daemon/transport";
import { saveFile } from "@/lib/filePicker";

export interface SaveDaemonExportOptions {
  /** Absolute path of the file the daemon wrote into the managed exports dir. */
  exportPath: string;
  /** Save-dialog title. */
  title: string;
  /** Suggested filename in the save dialog. */
  defaultName: string;
  /** Optional save-dialog file-type filters. */
  filters?: { name: string; extensions: string[] }[];
}

export interface SaveDaemonExportResult {
  /** Where the file ended up (the chosen destination, or the managed path). */
  savedPath: string;
  /** True when the file was copied to a user-chosen destination. */
  copied: boolean;
}

/**
 * Offer to save a daemon-produced export to a user-chosen location.
 *
 * In the desktop app this opens a native save dialog and copies the managed
 * export there; outside the desktop app (bridge/mock) it is a no-op that
 * returns the managed path so callers can still surface it.
 */
export async function saveDaemonExport(
  options: SaveDaemonExportOptions,
): Promise<SaveDaemonExportResult> {
  const { exportPath } = options;
  if (!exportPath || !canSaveExportedFiles()) {
    return { savedPath: exportPath, copied: false };
  }
  const destination = await saveFile({
    title: options.title,
    defaultPath: options.defaultName,
    filters: options.filters,
  });
  if (!destination) return { savedPath: exportPath, copied: false };
  const savedPath = await saveExportedFileAs(exportPath, destination);
  return { savedPath, copied: true };
}

export function exportBasename(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}
