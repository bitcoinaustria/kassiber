// @vitest-environment happy-dom
//
// The composer's permission-mode and tool-set pickers: icon and label on the
// trigger, each option with its description, full access marked as such, and
// both usable while an answer streams (the mode switches that answer).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/components/ai/ProviderModelPicker", () => ({
  ProviderModelPicker: () => <span data-testid="picker" />,
}));

import Ai02 from "@/components/ai-02";
import type { AiPermissionMode, AiToolProfile } from "@/daemon/stream";

function Harness({
  onMode,
  onTools,
  isStreaming = false,
}: {
  onMode: (mode: AiPermissionMode) => void;
  onTools: (profile: AiToolProfile) => void;
  isStreaming?: boolean;
}) {
  const [mode, setMode] = React.useState<AiPermissionMode>("auto");
  const [tools, setTools] = React.useState<AiToolProfile>("full");
  return (
    <Ai02
      selection={{ provider: "ollama", model: "qwen3:8b" }}
      onSelectionChange={() => {}}
      onSubmit={() => {}}
      onAbort={() => {}}
      isStreaming={isStreaming}
      permissionMode={mode}
      onPermissionModeChange={(next) => {
        setMode(next);
        onMode(next);
      }}
      toolProfile={tools}
      onToolProfileChange={(next) => {
        setTools(next);
        onTools(next);
      }}
    />
  );
}

function trigger(name: "permission-mode" | "tool-profile"): HTMLElement {
  return document.querySelector<HTMLElement>(`[data-composer-option="${name}"]`)!;
}

function open(name: "permission-mode" | "tool-profile") {
  fireEvent.pointerDown(trigger(name), { button: 0, pointerType: "mouse" });
}

describe("composer mode pickers", () => {
  afterEach(cleanup);

  it("picks a permission mode, with each option described", () => {
    const onMode = vi.fn();
    render(<Harness onMode={onMode} onTools={vi.fn()} isStreaming />);
    const mode = trigger("permission-mode");
    expect(mode.getAttribute("aria-label")).toBe("Permission mode: Auto");
    expect(mode.textContent).toContain("Auto");

    open("permission-mode");
    const options = screen.getAllByRole("menuitemradio");
    expect(options.map((option) => option.textContent)).toEqual([
      expect.stringContaining("Ask before every change."),
      expect.stringContaining("Ask before network access and anything permanent."),
      expect.stringContaining("syncs and other network actions included"),
    ]);
    expect(options[1]!.getAttribute("aria-checked")).toBe("true");

    // Usable mid-answer: the session switches the running turn.
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Full access/ }));
    expect(onMode).toHaveBeenCalledWith("full_access");
    expect(trigger("permission-mode").getAttribute("data-value")).toBe("full_access");
    expect(trigger("permission-mode").className).toContain("text-amber");
  });

  it("picks a tool set, all tools first", () => {
    const onTools = vi.fn();
    render(<Harness onMode={vi.fn()} onTools={onTools} />);
    expect(trigger("tool-profile").getAttribute("aria-label")).toBe("Tools: All tools");
    open("tool-profile");
    expect(screen.getAllByRole("menuitemradio").map((option) => option.textContent)).toEqual([
      expect.stringContaining("All tools"),
      expect.stringContaining("Focused"),
      expect.stringContaining("Minimal"),
    ]);
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Minimal/ }));
    expect(onTools).toHaveBeenCalledWith("core");
    expect(trigger("tool-profile").getAttribute("data-value")).toBe("core");
  });

  it("hides both pickers when the host does not offer them", () => {
    render(
      <Ai02
        selection={{ provider: "ollama", model: "qwen3:8b" }}
        onSelectionChange={() => {}}
        onSubmit={() => {}}
      />,
    );
    expect(document.querySelector("[data-composer-option]")).toBeNull();
  });
});
