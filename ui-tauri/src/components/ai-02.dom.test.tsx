// @vitest-environment happy-dom
//
// Mounted, not static: prompt recall, the IME guard and the picker shortcut
// are keyboard behaviour that only exists once the composer is live.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Sparkles } from "lucide-react";
import * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/components/ai/ProviderModelPicker", () => ({
  ProviderModelPicker: ({ open }: { open?: boolean }) => (
    <span data-testid="picker" data-open={open ? "true" : "false"} />
  ),
}));

import Ai02 from "@/components/ai-02";

const HISTORY = [
  { id: "u1", role: "user", content: "first question" },
  { id: "a1", role: "assistant", content: "an answer" },
  { id: "u2", role: "user", content: "second question" },
];

function Harness(
  props: Partial<React.ComponentProps<typeof Ai02>> & { initial?: string },
) {
  const { initial = "", ...rest } = props;
  const [value, setValue] = React.useState(initial);
  return (
    <Ai02
      selection={{ provider: "ollama", model: "qwen3:8b" }}
      onSelectionChange={() => {}}
      onSubmit={() => {}}
      value={value}
      onValueChange={setValue}
      historyMessages={HISTORY}
      {...rest}
    />
  );
}

function field(): HTMLTextAreaElement {
  return screen.getByRole("textbox") as HTMLTextAreaElement;
}

describe("Ai02 composer (mounted)", () => {
  afterEach(cleanup);

  it("recalls earlier prompts with ArrowUp and walks back down to empty", () => {
    render(<Harness />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("second question");
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("first question");
    // Oldest entry: further ArrowUp leaves the text alone.
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("first question");
    fireEvent.keyDown(field(), { key: "ArrowDown" });
    expect(field().value).toBe("second question");
    fireEvent.keyDown(field(), { key: "ArrowDown" });
    expect(field().value).toBe("");
  });

  it("never replaces a typed draft", () => {
    render(<Harness initial="my draft" />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("my draft");
  });

  it("does not recall into a composer holding an attachment", () => {
    render(<Harness attachedFilename="export.csv" />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("");
  });

  it("ends recall once the recalled prompt is edited", () => {
    render(<Harness />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    fireEvent.change(field(), { target: { value: "second question, edited" } });
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("second question, edited");
  });

  it("treats restoring the recalled wording by hand as a draft", () => {
    render(<Harness />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    fireEvent.change(field(), { target: { value: "second question!" } });
    fireEvent.change(field(), { target: { value: "second question" } });
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("second question");
    fireEvent.keyDown(field(), { key: "ArrowDown" });
    expect(field().value).toBe("second question");
  });

  it("ends recall when a suggestion is inserted", () => {
    render(
      <Harness
        alwaysShowSuggestions
        prompts={[
          { icon: Sparkles, text: "Suggest", prompt: "second question" },
        ]}
      />,
    );
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    fireEvent.click(screen.getByRole("button", { name: "Suggest" }));
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("second question");
  });

  it("does not send while an IME composition is being confirmed", () => {
    const onSubmit = vi.fn();
    render(<Harness initial="こんにちは" onSubmit={onSubmit} />);
    fireEvent.keyDown(field(), { key: "Enter", keyCode: 229 });
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(onSubmit).toHaveBeenCalledWith("こんにちは");
  });

  it("toggles the model picker with Mod+Shift+M from the composer", () => {
    render(<Harness />);
    const picker = screen.getByTestId("picker");
    expect(picker.dataset.open).toBe("false");
    fireEvent.keyDown(field(), { key: "M", ctrlKey: true, shiftKey: true });
    expect(screen.getByTestId("picker").dataset.open).toBe("true");
    fireEvent.keyDown(field(), { key: "m", metaKey: true, shiftKey: true });
    expect(screen.getByTestId("picker").dataset.open).toBe("false");
  });

  it("keeps a compact composer's toolbar unfolded while the picker is open", () => {
    render(<Harness compact />);
    const toolbar = () =>
      screen.getByTestId("picker").closest("div.relative.z-10") as HTMLElement;
    expect(toolbar().className).toContain("max-h-0");
    fireEvent.keyDown(field(), { key: "m", ctrlKey: true, shiftKey: true });
    expect(toolbar().className).not.toContain("max-h-0");
  });

  it("ignores the picker shortcut when the picker is disabled", () => {
    render(<Harness modelPickerEnabled={false} />);
    fireEvent.keyDown(field(), { key: "m", ctrlKey: true, shiftKey: true });
    expect(screen.getByTestId("picker").dataset.open).toBe("false");
  });

  it("offers stop and queue while a reply streams", () => {
    const onAbort = vi.fn();
    const onSubmit = vi.fn();
    render(
      <Harness
        initial="follow up"
        isStreaming
        onAbort={onAbort}
        onSubmit={onSubmit}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Stop generating" }));
    expect(onAbort).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Queue message" }));
    expect(onSubmit).toHaveBeenCalledWith("follow up");
  });
});
