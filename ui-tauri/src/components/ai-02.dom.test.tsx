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

  it("holds sending while the provider settings load, and says why", () => {
    const onSubmit = vi.fn();
    const reason = "Loading this book's AI provider settings…";
    const view = render(
      <Harness initial="hello" onSubmit={onSubmit} sendBlockedReason={reason} />,
    );
    const send = screen.getByRole("button", { name: reason }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(onSubmit).not.toHaveBeenCalled();

    view.rerender(<Harness initial="hello" onSubmit={onSubmit} sendBlockedReason={null} />);
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(onSubmit).toHaveBeenCalledWith("hello");
  });

  it("never replaces a typed draft", () => {
    render(<Harness initial="my draft" />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("my draft");
  });

  it("does not recall into a composer holding a new attachment", () => {
    render(<Harness attachment={{ filename: "export.csv" }} />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("");
  });

  it("recalls while the chat's file was already sent", () => {
    render(<Harness attachment={{ filename: "export.csv" }} attachmentState="sent" />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("second question");
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

  it("tells its own replacements of the text apart from the user's edits", () => {
    // A draft that keeps a parked handoff's context through edits needs to
    // know which writes are the user's typing.
    const edits: string[] = [];
    const replaced: string[] = [];
    function Writes() {
      const [value, setValue] = React.useState("");
      return (
        <Ai02
          selection={{ provider: "ollama", model: "qwen3:8b" }}
          onSelectionChange={() => {}}
          onSubmit={() => {}}
          value={value}
          onValueChange={(next) => {
            edits.push(next);
            setValue(next);
          }}
          onValueReplace={(next) => {
            replaced.push(next);
            setValue(next);
          }}
          historyMessages={HISTORY}
          prompts={[{ icon: Sparkles, text: "Suggest", prompt: "suggested question" }]}
          alwaysShowSuggestions
        />
      );
    }
    render(<Writes />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    fireEvent.click(screen.getByRole("button", { name: "Suggest" }));
    fireEvent.change(field(), { target: { value: "suggested question, please" } });
    fireEvent.keyDown(field(), { key: "Enter" });

    // A recalled prompt, a suggestion and the clear after a send replace the
    // text; only typing is an edit.
    expect(replaced).toEqual(["second question", "suggested question", ""]);
    expect(edits).toEqual(["suggested question, please"]);
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

describe("Ai02 attachments (mounted)", () => {
  afterEach(cleanup);

  const csv = () => new File(["date,amount\n"], "export.csv", { type: "text/csv" });

  it("attaches a pasted file instead of pasting it as text", () => {
    const onAttachFiles = vi.fn();
    render(<Harness onAttachFiles={onAttachFiles} />);
    const file = csv();
    const pasted = fireEvent.paste(field(), { clipboardData: { files: [file], getData: () => "" } });
    expect(onAttachFiles).toHaveBeenCalledWith([file]);
    expect(pasted).toBe(false);
  });

  it("leaves a text paste alone", () => {
    const onAttachFiles = vi.fn();
    render(<Harness onAttachFiles={onAttachFiles} />);
    const pasted = fireEvent.paste(field(), { clipboardData: { files: [], getData: () => "text" } });
    expect(onAttachFiles).not.toHaveBeenCalled();
    expect(pasted).toBe(true);
  });

  it("takes a file dropped on the drop target and hints while it is dragged over", () => {
    const onAttachFiles = vi.fn();
    function Target() {
      const ref = React.useRef<HTMLDivElement>(null);
      return (
        <div ref={ref} data-testid="target">
          <Harness onAttachFiles={onAttachFiles} dropTargetRef={ref} />
        </div>
      );
    }
    const view = render(<Target />);
    const target = screen.getByTestId("target");
    const file = csv();
    const dataTransfer = { files: [file], types: ["Files"], dropEffect: "none" };
    fireEvent.dragEnter(target, { dataTransfer });
    expect(view.container.querySelector("[data-composer-drop-hint]")).not.toBeNull();
    expect(screen.getByText("Drop to attach")).toBeTruthy();
    fireEvent.drop(target, { dataTransfer });
    expect(onAttachFiles).toHaveBeenCalledWith([file]);
    expect(view.container.querySelector("[data-composer-drop-hint]")).toBeNull();
  });

  it("ignores a drag that carries no files", () => {
    const onAttachFiles = vi.fn();
    const view = render(<Harness onAttachFiles={onAttachFiles} />);
    const surface = view.container.querySelector(".kb-composer-glass") as HTMLElement;
    fireEvent.dragEnter(surface, { dataTransfer: { files: [], types: ["text/plain"] } });
    expect(view.container.querySelector("[data-composer-drop-hint]")).toBeNull();
  });
});
