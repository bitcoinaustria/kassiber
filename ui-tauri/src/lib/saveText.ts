/**
 * Shared wrapper around the `save_text_file_as` Tauri command.
 *
 * Each call site supplies the extensions it expects so the Rust command
 * can keep its allowlist guard accurate without baking the choice into
 * the helper. Used by chat export, diagnostics export, and any future
 * "save this text blob to a user-chosen path" surface.
 */

export async function saveTextFileAs(
  destinationPath: string,
  contents: string,
  allowedExtensions: string[],
): Promise<string> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("save_text_file_as", {
    destinationPath,
    contents,
    allowedExtensions,
  });
}
