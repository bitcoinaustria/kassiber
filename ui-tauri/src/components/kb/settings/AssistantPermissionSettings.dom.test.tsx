// @vitest-environment happy-dom
//
// Settings → AI: the permission mode new chats start in, Auto by default.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

import { DEFAULT_ASSISTANT_PERMISSION_MODE, useUiStore } from "@/store/ui";

import { AssistantPermissionSettings } from "./AssistantPermissionSettings";

describe("AssistantPermissionSettings", () => {
  beforeEach(() => {
    useUiStore.setState({ assistantDefaultPermissionMode: DEFAULT_ASSISTANT_PERMISSION_MODE });
  });
  afterEach(cleanup);

  it("defaults to Auto and stores the mode new chats start in", () => {
    expect(DEFAULT_ASSISTANT_PERMISSION_MODE).toBe("auto");
    const { container } = render(<AssistantPermissionSettings />);
    const group = screen.getByRole("radiogroup", { name: "Default permission mode" });
    expect(screen.getByRole("radio", { name: /Auto/ }).getAttribute("aria-checked")).toBe("true");
    expect(group).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: /Full access/ }));
    expect(useUiStore.getState().assistantDefaultPermissionMode).toBe("full_access");
    expect(
      container.querySelector('[data-permission-mode-description="full_access"]')?.textContent,
    ).toContain("network actions");
  });
});
