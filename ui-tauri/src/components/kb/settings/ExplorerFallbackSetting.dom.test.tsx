// @vitest-environment happy-dom
//
// The Privacy panel's public-explorer switch stores the user's own choice
// beside the setup-derived `publicFallbacks`, so a backend refresh that
// rederives the latter cannot switch it back on.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

import { DEFAULT_EXPLORER_SETTINGS } from "@/lib/explorer";
import { useUiStore } from "@/store/ui";

import { ExplorerFallbackSetting } from "./ExplorerFallbackSetting";

afterEach(() => {
  cleanup();
  useUiStore.setState({ explorerSettings: DEFAULT_EXPLORER_SETTINGS });
});

describe("ExplorerFallbackSetting", () => {
  it("is on by default and explains what a public explorer learns", () => {
    render(<ExplorerFallbackSetting />);
    const toggle = screen.getByRole("switch");
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(document.body.textContent).toContain(
      "third-party website, which learns the transaction or address",
    );
  });

  it("keeps the user's opt-out when the setup is rederived", () => {
    render(<ExplorerFallbackSetting />);
    act(() => {
      fireEvent.click(screen.getByRole("switch"));
    });
    expect(useUiStore.getState().explorerSettings.publicFallbacksAllowed).toBe(false);
    // What SettingsScreen and AppShell do after every backend refresh.
    act(() => {
      useUiStore.getState().setExplorerSettings({
        bitcoinBaseUrl: "",
        liquidBaseUrl: "",
        publicFallbacks: true,
      });
    });
    expect(useUiStore.getState().explorerSettings.publicFallbacksAllowed).toBe(false);
    expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
  });

  it("is inert where the setup never falls back (manual or regtest)", () => {
    act(() => {
      useUiStore.getState().setExplorerSettings({ publicFallbacks: false });
    });
    render(<ExplorerFallbackSetting />);
    const toggle = screen.getByRole("switch");
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.hasAttribute("disabled")).toBe(true);
  });
});
