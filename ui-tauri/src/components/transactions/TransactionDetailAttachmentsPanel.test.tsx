import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { AttachmentsPanel } from "./TransactionDetailAttachmentsPanel";

describe("AttachmentsPanel", () => {
  it("shows inline rename controls only for URL attachments", () => {
    const html = renderToStaticMarkup(
      <AttachmentsPanel
        hideSensitive={false}
        items={[
          {
            id: "url-1",
            kind: "url",
            label: "Treasury sheet",
            detail: "https://docs.google.com/spreadsheets/d/abc123/edit",
          },
          {
            id: "file-1",
            kind: "file",
            label: "invoice.pdf",
            detail: "application/pdf",
          },
        ]}
        onRename={() => undefined}
      />,
    );

    expect(html).toContain("Edit link text for Treasury sheet");
    expect(html).not.toContain("Edit link text for invoice.pdf");
  });
});
