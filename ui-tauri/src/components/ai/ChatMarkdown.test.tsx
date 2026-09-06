import type { MouseEvent, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ChatMarkdown } from "./ChatMarkdown";
import { ChatMessage } from "./ChatMessage";
import { ChatReasoning } from "./ChatReasoning";

const mocks = vi.hoisted(() => ({
  openExternalUrl: vi.fn(),
  openImage: null as ((event: MouseEvent<HTMLButtonElement>) => void) | null,
}));

vi.mock("@/daemon/transport", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/daemon/transport")>(),
  openExternalUrl: mocks.openExternalUrl,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick, ...props }: {
    children: ReactNode;
    onClick?: (event: MouseEvent<HTMLButtonElement>) => void;
    "data-chat-image-reference"?: boolean;
  }) => {
    if (props["data-chat-image-reference"]) mocks.openImage = onClick ?? null;
    return <button onClick={onClick}>{children}</button>;
  },
}));

describe("ChatMarkdown", () => {
  beforeEach(() => {
    mocks.openExternalUrl.mockReset().mockResolvedValue(undefined);
    mocks.openImage = null;
  });
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

  it.each(["done", "streaming"] as const)("keeps image references inert in %s assistant messages", (status) => {
    const html = renderToStaticMarkup(<ChatMessage message={{
      id: "image-reply", role: "assistant", status,
      content: "![Private chart](https://images.example.invalid/chart?book=private)",
    }} />);
    expect(html).not.toMatch(/<(?:img|link)\b/);
    expect(html).not.toContain("src=");
    expect(html).toContain("Open image: Private chart");
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("does not load image references as a streamed token becomes complete", () => {
    const content = "![Chart](http://127.0.0.1:12345/image)";
    for (let end = 1; end <= content.length; end++) {
      const html = renderToStaticMarkup(<ChatMarkdown content={content.slice(0, end)} />);
      expect(html).not.toMatch(/<(?:img|link)\b/);
    }
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("keeps reference-style images in opened reasoning inert too", () => {
    const html = renderToStaticMarkup(<ChatReasoning
      thinking={"![Trace][image]\n\n[image]: https://images.example.invalid/trace"}
      isStreaming={false} hasAnswer defaultOpen
    />);
    expect(html).not.toMatch(/<(?:img|link)\b/);
    expect(html).toContain("Open image: Trace");
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("opens only the image selected by an explicit click", () => {
    renderToStaticMarkup(<ChatMarkdown content={"[![Chart](https://images.example.invalid/chart)](https://other.example.invalid/)"} />);
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
    expect(mocks.openImage).not.toBeNull();
    const event = { preventDefault: vi.fn(), stopPropagation: vi.fn() };
    mocks.openImage?.(event as unknown as MouseEvent<HTMLButtonElement>);
    expect(mocks.openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://images.example.invalid/chart");
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
  });

  it.each(["file:///private/chart.png", "data:image/png;base64,AA", "/__kassiber__/chart", "https://user:secret@example.invalid/chart"])("keeps unsupported image target %s as text", (source) => {
    const html = renderToStaticMarkup(<ChatMarkdown content={`![Chart](${source})`} />);
    expect(html).not.toMatch(/<(?:img|link|button)\b/);
    expect(html).toContain("Chart");
    expect(mocks.openImage).toBeNull();
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });
});
