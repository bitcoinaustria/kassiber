// @vitest-environment happy-dom
//
// The composer's permission-mode picker: icon and label on the trigger, each
// option with its description, full access marked as such, and usable while an
// answer streams (the mode switches that answer). It is the composer's only
// mode control; the tool set belongs to the provider's settings.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/components/ai/ProviderModelPicker", () => ({
  ProviderModelPicker: () => <span data-testid="picker" />,
}));

import Ai02 from "@/components/ai-02";
import type { AiPermissionMode } from "@/daemon/stream";

function Harness({
  onMode,
  isStreaming = false,
}: {
  onMode: (mode: AiPermissionMode) => void;
  isStreaming?: boolean;
}) {
  const [mode, setMode] = React.useState<AiPermissionMode>("auto");
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
    />
  );
}

function trigger(): HTMLElement {
  return document.querySelector<HTMLElement>('[data-composer-option="permission-mode"]')!;
}

function open() {
  fireEvent.pointerDown(trigger(), { button: 0, pointerType: "mouse" });
}

describe("composer permission mode picker", () => {
  afterEach(cleanup);

  it("picks a permission mode, with each option described", () => {
    const onMode = vi.fn();
    render(<Harness onMode={onMode} isStreaming />);
    const mode = trigger();
    expect(mode.getAttribute("aria-label")).toBe("Permission mode: Auto");
    expect(mode.textContent).toContain("Auto");
    // The only mode control in the toolbar.
    expect(
      [...document.querySelectorAll<HTMLElement>("[data-composer-option]")].map(
        (control) => control.dataset.composerOption,
      ),
    ).toEqual(["permission-mode"]);

    open();
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
    expect(trigger().getAttribute("data-value")).toBe("full_access");
    expect(trigger().className).toContain("text-amber");
  });

  it("hides the picker when the host does not offer it", () => {
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
