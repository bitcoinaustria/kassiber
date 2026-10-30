import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@/daemon/transport", () => ({
  getTransport: () => ({ invoke }),
  makeDaemonRequestId: () => "req",
}));

import {
  ChatAttachmentError,
  uploadChatAttachment,
} from "@/components/ai/chatAttachmentUpload";

const SCOPE = { workspace_id: "w", profile_id: "p" };

function decode(chunks: string[]): Uint8Array {
  const parts = chunks.map((chunk) => Uint8Array.from(atob(chunk), (char) => char.charCodeAt(0)));
  const joined = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    joined.set(part, offset);
    offset += part.length;
  }
  return joined;
}

describe("uploadChatAttachment", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("hands a file over in chunks the daemon can read and returns its token", async () => {
    const bytes = new Uint8Array(1_200_000).map((_, index) => index % 251);
    let received = 0;
    invoke.mockImplementation(async ({ args }: { args: Record<string, unknown> }) => {
      received += atob(String(args.chunk_base64)).length;
      return received < bytes.length
        ? { kind: "ui.chat.attachments.upload", data: { complete: false, upload_id: "u1", received_bytes: received } }
        : {
            kind: "ui.chat.attachments.upload",
            data: {
              complete: true,
              document_token: "tok",
              source: { filename: "export.csv", kind: "delimited", size_bytes: bytes.length },
            },
          };
    });
    const selection = await uploadChatAttachment(new File([bytes], "export.csv"), SCOPE);
    expect(selection).toEqual({
      document_token: "tok",
      source: { filename: "export.csv", kind: "delimited", size_bytes: bytes.length },
    });
    const calls = invoke.mock.calls.map(([request]) => request.args);
    expect(calls).toHaveLength(3);
    expect(calls[0]).toMatchObject({ filename: "export.csv", total_bytes: bytes.length, offset: 0, expected_scope: SCOPE });
    expect(calls[1]).toMatchObject({ upload_id: "u1", offset: 512 * 1024, expected_scope: SCOPE });
    expect(calls[1]).not.toHaveProperty("filename");
    // Each request stays below the daemon's 1,000,000-character line limit.
    for (const args of calls) expect(String(args.chunk_base64).length).toBeLessThan(800_000);
    expect(decode(calls.map((args) => String(args.chunk_base64)))).toEqual(bytes);
  });

  it("refuses a file type the chat does not take without sending it", async () => {
    await expect(uploadChatAttachment(new File(["x"], "wallet.dat"), SCOPE)).rejects.toMatchObject({
      code: "chat_attachment_unsupported",
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reports the daemon's refusal", async () => {
    invoke.mockResolvedValue({
      kind: "error",
      error: { code: "stale_context", message: "The active book changed" },
    });
    const upload = uploadChatAttachment(new File(["a,b\n"], "export.csv"), SCOPE);
    await expect(upload).rejects.toBeInstanceOf(ChatAttachmentError);
    await expect(upload).rejects.toMatchObject({ code: "stale_context" });
  });
});
