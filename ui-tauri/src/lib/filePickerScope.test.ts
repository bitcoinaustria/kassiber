import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/daemon/transport", () => ({ DAEMON_MODE: "bridge" }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const selection = { document_token: "opaque-token", attachment_id: "attachment-a",
  transaction_id: "tx-a", source: { filename: "statement.csv" } };
const scope = { workspace_id: "workspace-a", profile_id: "profile-a" };

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
  vi.clearAllMocks();
});

describe("review attachment scope", () => {
  it("carries the original book to the bridge staging request after native selection", async () => {
    vi.stubGlobal("window", {});
    const fetch = vi.fn().mockResolvedValue({
      ok: true, json: async () => ({ documentImportSource: selection }),
    });
    vi.stubGlobal("fetch", fetch);
    const { pickChatAttachmentSource } = await import("./filePicker");
    expect(await pickChatAttachmentSource({ expected_scope: scope, review_case_id: "quarantine:tx-a" })).toEqual(selection);
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({
      purpose: "chat_attachment", expected_scope: scope, review_case_id: "quarantine:tx-a",
    });
  });

  it("carries the same guard through the native Tauri picker without a renderer path", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    mocks.invoke.mockResolvedValue(selection);
    const { pickChatAttachmentSource } = await import("./filePicker");
    expect(await pickChatAttachmentSource({ expected_scope: scope, review_case_id: "quarantine:tx-a" })).toEqual(selection);
    expect(mocks.invoke).toHaveBeenCalledWith("pick_chat_attachment_source", {
      expectedScope: scope, reviewCaseId: "quarantine:tx-a",
    });
  });
});
