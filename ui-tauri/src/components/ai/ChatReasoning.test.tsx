import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatReasoning } from "./ChatReasoning";

describe("ChatReasoning", () => {
  it("keeps thinking content collapsed by default", () => {
    const html = renderToStaticMarkup(
      <ChatReasoning
        thinking="Checking report readiness..."
        isStreaming={true}
        hasAnswer={false}
      />,
    );

    expect(html).toContain("Thinking");
    expect(html).not.toContain("Checking report readiness");
  });

  it("renders thinking content as markdown", () => {
    const html = renderToStaticMarkup(
      <ChatReasoning
        thinking={[
          "## Plan",
          "",
          "- Inspect `transactions`",
          "- **Compare** balances",
        ].join("\n")}
        isStreaming={true}
        hasAnswer={false}
        defaultOpen
      />,
    );

    expect(html).toContain("<h2");
    expect(html).toContain("<ul");
    expect(html).toContain("<code");
    expect(html).toContain("<strong");
  });

  it("renders one pane per thinking segment", () => {
    const html = renderToStaticMarkup(
      <ChatReasoning
        segments={[
          { id: "s1", content: "First round plan." },
          { id: "s2", content: "Second round after tools." },
        ]}
        isStreaming={false}
        hasAnswer={true}
        defaultOpen
      />,
    );

    expect(html).toContain("Thoughts · round 1");
    expect(html).toContain("Thoughts · round 2");
    expect(html).toContain("First round plan.");
    expect(html).toContain("Second round after tools.");
  });
});
