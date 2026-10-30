import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import Ai02 from "@/components/ai-02";

vi.mock("@/components/ai/ProviderModelPicker", () => ({
  ProviderModelPicker: () => null,
}));

function render(props: Partial<React.ComponentProps<typeof Ai02>> = {}) {
  return renderToStaticMarkup(
    <Ai02
      selection={{ provider: "ollama", model: "gemma" }}
      onSelectionChange={() => {}}
      onSubmit={() => {}}
      {...props}
    />,
  );
}

/** The attach button's opening tag, so class-name utilities are not mistaken
 *  for the `disabled` attribute (Tailwind emits `disabled:opacity-50` etc.). */
function attachButtonTag(html: string): string {
  const tag = html.match(/<button[^>]*aria-label="Attach a file"[^>]*>/)?.[0];
  expect(tag).toBeDefined();
  return tag as string;
}

const DISABLED_ATTRIBUTE = /\sdisabled(=|\s|>)/;

describe("Ai02 file attachment", () => {
  it("disables the attach button when no handler is wired", () => {
    // The button predates the feature; without a handler it must read as
    // unavailable rather than looking clickable and doing nothing.
    expect(attachButtonTag(render())).toMatch(DISABLED_ATTRIBUTE);
  });

  it("enables the attach button once a handler is wired", () => {
    expect(attachButtonTag(render({ onAttach: () => {} }))).not.toMatch(
      DISABLED_ATTRIBUTE,
    );
  });

  it("shows the attached file as a card with its type and size", () => {
    const html = render({
      onAttach: () => {},
      attachments: [
        { key: "a", filename: "xyz-export-2024.csv", kind: "delimited", sizeBytes: 12_400, state: "ready" },
        { key: "b", filename: "receipt.png", kind: "image", sizeBytes: 800, state: "ready" },
      ],
      onRemoveAttachment: () => {},
    });
    expect(html).toContain("xyz-export-2024.csv");
    expect(html).toContain("CSV · 12 kB");
    expect(html).toContain('aria-label="Remove xyz-export-2024.csv"');
    // Several files, each its own card.
    expect(html).toContain('aria-label="Remove receipt.png"');
    expect(html.match(/data-chat-attachment/g)).toHaveLength(2);
  });

  it("shows no card when nothing is attached", () => {
    const html = render({ onAttach: () => {}, attachments: [] });
    expect(html).not.toContain("data-chat-attachment");
  });

  it("omits the remove control when clearing is not offered", () => {
    const html = render({ attachments: [{ key: "a", filename: "export.csv", state: "ready" }] });
    expect(html).toContain("export.csv");
    expect(html).not.toContain('aria-label="Remove export.csv"');
  });

  it("says a sent file stays with the chat, and holds removal while uploading", () => {
    const sent = render({
      attachments: [{ key: "a", filename: "export.csv", state: "sent" }],
      onRemoveAttachment: () => {},
    });
    expect(sent).toContain("Stays with this chat");
    const uploading = render({
      attachments: [{ key: "b", filename: "screenshot.png", state: "uploading" }],
      onRemoveAttachment: () => {},
    });
    expect(uploading).toContain("Attaching…");
    expect(uploading).not.toContain('aria-label="Remove screenshot.png"');
  });
});
