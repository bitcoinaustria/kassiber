import { describe, expect, it } from "vitest";

import {
  buildDocumentImportArgs,
  buildDocumentImportPreviewArgs,
} from "./documentImport";

const FORBIDDEN_RENDERER_FIELDS = [
  "source_file",
  "file",
  "file_path",
  "draft",
  "rows",
  "expected_source_sha256",
  "include_quarantined",
  "attach_evidence",
];

describe("document import request boundary", () => {
  it("previews with only the opaque document session", () => {
    const args = buildDocumentImportPreviewArgs("session-1", "local", "model");
    expect(args).toEqual({
      document_token: "session-1",
      provider: "local",
      model: "model",
    });
    for (const field of FORBIDDEN_RENDERER_FIELDS) {
      expect(args).not.toHaveProperty(field);
    }
  });

  it("imports only selected ids from the daemon-owned preview", () => {
    const args = buildDocumentImportArgs(
      "session-1",
      "wallet-1",
      new Set(["row-1", "row-2"]),
    );
    expect(args).toEqual({
      document_token: "session-1",
      wallet: "wallet-1",
      selected_row_ids: ["row-1", "row-2"],
    });
    for (const field of FORBIDDEN_RENDERER_FIELDS) {
      expect(args).not.toHaveProperty(field);
    }
  });
});
