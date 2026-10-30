import type { DocumentImportSourceSelection } from "@/lib/filePicker";
import {
  getTransport,
  makeDaemonRequestId,
  type DaemonEnvelope,
} from "@/daemon/transport";
import {
  MAX_CHAT_UPLOAD_BYTES,
  isChatAttachmentName,
} from "@/components/ai/chatAttachments";

/** Below the daemon's request line limit once base64-encoded. */
const UPLOAD_CHUNK_BYTES = 512 * 1024;
const UPLOAD_KIND = "ui.chat.attachments.upload";

const isTauriRuntime =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function base64Chunk(bytes: Uint8Array): string {
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  }
  return btoa(binary);
}

export class ChatAttachmentError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

type UploadReply = {
  complete: boolean;
  upload_id?: string;
} & Partial<DocumentImportSourceSelection>;

/**
 * Hand a pasted (or, in the browser preview, dropped) file to the daemon in
 * chunks. It keeps the bytes in a private temporary file and returns the same
 * token a picked file gets; the renderer never names a path.
 */
export async function uploadChatAttachment(
  file: File,
  expectedScope: { workspace_id: string; profile_id: string },
): Promise<DocumentImportSourceSelection> {
  if (!isChatAttachmentName(file.name)) {
    throw new ChatAttachmentError("chat_attachment_unsupported", file.name);
  }
  if (file.size > MAX_CHAT_UPLOAD_BYTES) {
    throw new ChatAttachmentError("chat_attachment_too_large", file.name);
  }
  if (file.size === 0) {
    throw new ChatAttachmentError("validation", file.name);
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  let uploadId: string | null = null;
  for (let offset = 0; offset < bytes.length; offset += UPLOAD_CHUNK_BYTES) {
    const envelope: DaemonEnvelope<UploadReply> = await getTransport().invoke<UploadReply>({
      kind: UPLOAD_KIND,
      request_id: makeDaemonRequestId(),
      args: {
        ...(uploadId === null
          ? { filename: file.name, total_bytes: bytes.length, expected_scope: expectedScope }
          : { upload_id: uploadId, expected_scope: expectedScope }),
        offset,
        chunk_base64: base64Chunk(bytes.subarray(offset, offset + UPLOAD_CHUNK_BYTES)),
      },
    });
    const data: UploadReply | undefined = envelope.data;
    if (envelope.kind !== UPLOAD_KIND || !data) {
      throw new ChatAttachmentError(
        envelope.error?.code ?? "chat_attachment_failed",
        envelope.error?.message ?? file.name,
      );
    }
    if (data.complete) {
      if (!data.document_token || !data.source) {
        throw new ChatAttachmentError("chat_attachment_failed", file.name);
      }
      return { document_token: data.document_token, source: data.source };
    }
    uploadId = data.upload_id ?? null;
  }
  throw new ChatAttachmentError("chat_attachment_failed", file.name);
}

export type DroppedChatAttachment =
  | { status: "staged"; selection: DocumentImportSourceSelection; droppedCount: number }
  | { status: "unsupported"; droppedCount: number }
  | { status: "none" };

/**
 * Stage the file just dropped on the desktop window. The native side kept
 * the paths of that drop; the webview can only ask for it, once.
 */
export async function stageDroppedChatAttachment(
  expectedScope: { workspace_id: string; profile_id: string },
): Promise<DroppedChatAttachment> {
  if (!isTauriRuntime) return { status: "none" };
  const { invoke } = await import("@tauri-apps/api/core");
  const result = await invoke<Record<string, unknown> | null>(
    "stage_dropped_chat_attachment",
    { expectedScope },
  );
  if (!result) return { status: "none" };
  const droppedCount = typeof result.dropped_count === "number" ? result.dropped_count : 1;
  if (result.rejected) return { status: "unsupported", droppedCount };
  if (typeof result.document_token !== "string" || !result.source) return { status: "none" };
  return {
    status: "staged",
    selection: result as unknown as DocumentImportSourceSelection,
    droppedCount,
  };
}
