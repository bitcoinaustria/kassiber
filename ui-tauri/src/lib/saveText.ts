/**
 * Wrappers around the purpose-specific Tauri text-export commands.
 *
 * Each command in the native side hard-codes which file extensions it
 * accepts (see `ui-tauri/src-tauri/src/lib.rs::write_text_export`). The
 * renderer cannot influence that list — if you want a new extension,
 * add a new Tauri command on the native side, do not parameterize an
 * existing one. This keeps the WebView-invoke boundary from becoming an
 * arbitrary text-file-write primitive.
 */

async function invokeSave(
  command: "save_chat_export_as" | "save_logs_export_as",
  destinationPath: string,
  contents: string,
): Promise<string> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>(command, { destinationPath, contents });
}

export function saveChatExportAs(
  destinationPath: string,
  contents: string,
): Promise<string> {
  return invokeSave("save_chat_export_as", destinationPath, contents);
}

export function saveLogsExportAs(
  destinationPath: string,
  contents: string,
): Promise<string> {
  return invokeSave("save_logs_export_as", destinationPath, contents);
}
