import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatMarkdown } from "./ChatMarkdown";

describe("ChatMarkdown", () => {
  it("renders headings and GFM tables with structured elements", () => {
    const html = renderToStaticMarkup(
      <ChatMarkdown
        content={[
          "## Sources",
          "",
          "| Topic | Reference |",
          "| --- | --- |",
          "| CLI | `reports tax-summary` |",
        ].join("\n")}
      />,
    );

    expect(html).toContain("<h2");
    expect(html).toContain("<table");
    expect(html).toContain("<thead");
    expect(html).toContain("<code");
    expect(html).toContain("reports tax-summary");
  });
});
