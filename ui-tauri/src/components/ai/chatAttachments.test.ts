import { describe, expect, it } from "vitest";

import { chatThumbnail } from "./chatAttachments";

describe("chatThumbnail", () => {
  it("takes only the daemon's inline PNG", () => {
    expect(chatThumbnail("data:image/png;base64,iVBORw0KGgo=")).toBe("data:image/png;base64,iVBORw0KGgo=");
    for (const value of [
      "https://example.com/a.png",
      "data:image/svg+xml;base64,PHN2Zz4=",
      "file:///home/user/receipt.png",
      undefined,
      42,
    ]) {
      expect(chatThumbnail(value)).toBeUndefined();
    }
  });
});
