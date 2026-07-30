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

  it("shows the attached filename as a removable chip", () => {
    const html = render({
      onAttach: () => {},
      attachedFilename: "xyz-export-2024.csv",
      onClearAttachment: () => {},
    });
    expect(html).toContain("xyz-export-2024.csv");
    expect(html).toContain('aria-label="Remove attachment"');
  });

  it("shows no chip when nothing is attached", () => {
    const html = render({ onAttach: () => {}, attachedFilename: null });
    expect(html).not.toContain('aria-label="Remove attachment"');
  });

  it("omits the remove control when clearing is not offered", () => {
    const html = render({ attachedFilename: "export.csv" });
    expect(html).toContain("export.csv");
    expect(html).not.toContain('aria-label="Remove attachment"');
  });
});
