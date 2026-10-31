/**
 * Files the assistant can take: exports the generic ledger reads, and the
 * statements and photos the document importer reads. The daemon checks the
 * same list; this one only decides what the composer offers to attach.
 */
export const CHAT_ATTACHMENT_EXTENSIONS = [
  "csv", "tsv", "xlsx", "xlsm", "pdf", "png", "jpg", "jpeg", "webp", "gif",
] as const;

/** Files one chat holds; the daemon names them file-1 .. file-8. */
export const MAX_CHAT_ATTACHMENTS = 8;

/** The daemon refuses larger pasted files; the picker reads larger ones in place. */
export const MAX_CHAT_UPLOAD_BYTES = 25 * 1024 * 1024;

export type ChatAttachmentKind = "spreadsheet" | "delimited" | "pdf" | "image";

/** What the composer and a sent message show about a file. */
export interface ChatAttachmentView {
  filename: string;
  kind?: string;
  sizeBytes?: number;
  /** A small data: URL for an image the renderer already holds; never a path. */
  previewUrl?: string;
}

function extensionOf(filename: string): string {
  const dot = filename.lastIndexOf(".");
  return dot > 0 ? filename.slice(dot + 1).toLowerCase() : "";
}

export function isChatAttachmentName(filename: string): boolean {
  return (CHAT_ATTACHMENT_EXTENSIONS as readonly string[]).includes(extensionOf(filename));
}

export function chatAttachmentKind(
  filename: string,
  staged?: string | null,
): ChatAttachmentKind | null {
  if (staged === "spreadsheet" || staged === "delimited" || staged === "pdf" || staged === "image") {
    return staged;
  }
  const extension = extensionOf(filename);
  if (extension === "xlsx" || extension === "xlsm") return "spreadsheet";
  if (extension === "csv" || extension === "tsv") return "delimited";
  if (extension === "pdf") return "pdf";
  if (["png", "jpg", "jpeg", "webp", "gif"].includes(extension)) return "image";
  return null;
}

/** The type as people know it from the file name: CSV, XLSX, PDF, PNG. */
export function chatAttachmentTypeLabel(filename: string): string {
  return extensionOf(filename).toUpperCase();
}

export function formatAttachmentSize(bytes: number, language?: string): string {
  // Intl spells a bare byte count out ("42 byte"); file sizes read "42 B".
  if (bytes < 1000) return `${bytes} B`;
  const units = ["byte", "kilobyte", "megabyte"] as const;
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return new Intl.NumberFormat(language, {
    style: "unit",
    unit: units[unit],
    unitDisplay: "short",
    maximumFractionDigits: unit === 0 || value >= 10 ? 0 : 1,
  }).format(value);
}

/**
 * A small thumbnail of an image the user pasted, as a data: URL (the app's
 * content policy allows data: images, not blob: ones). Null when the
 * renderer cannot decode it; the card then shows the file type instead.
 */
export async function chatImageThumbnail(file: File, size = 96): Promise<string | null> {
  if (chatAttachmentKind(file.name) !== "image" || typeof createImageBitmap !== "function") {
    return null;
  }
  try {
    const bitmap = await createImageBitmap(file);
    const scale = Math.min(1, size / Math.max(bitmap.width, bitmap.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(bitmap.width * scale));
    canvas.height = Math.max(1, Math.round(bitmap.height * scale));
    const context = canvas.getContext("2d");
    if (!context) return null;
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    bitmap.close();
    return canvas.toDataURL("image/png");
  } catch {
    return null;
  }
}

/**
 * The daemon's preview of a picked or dropped chat image, if it is the inline
 * PNG it should be; anything else (a URL, another type) is not shown.
 */
export function chatThumbnail(value: unknown): string | undefined {
  return typeof value === "string" && value.startsWith("data:image/png;base64,")
    ? value
    : undefined;
}
