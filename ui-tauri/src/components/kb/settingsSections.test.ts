import { describe, expect, it } from "vitest";

import { settingsSectionForHash } from "./settingsSections";

describe("settings section hash mapping", () => {
  // The native menu, deep-link parser (Rust side), direct URL navigation, and
  // the in-app section rail all funnel through this helper. Pinning the
  // canonical sections + the alias map makes accidental rename or removal of a
  // section id a test failure rather than a runtime "panel never re-opens" bug.
  it("maps every Rust deep-link slug to a section", () => {
    const expected = {
      appearance: "general-appearance",
      privacy: "security-privacy",
      developer: "desktop-developer",
      logs: "desktop-developer",
      display: "general-appearance",
      explorer: "network-bitcoin",
      explorers: "network-bitcoin",
      bitcoin: "network-bitcoin",
      lightning: "network-lightning",
      liquid: "network-liquid",
      market: "network-market",
      desktop: "desktop-terminal",
      terminal: "desktop-terminal",
      security: "security-lock",
      lock: "security-lock",
      backends: "network-bitcoin",
      sync: "data-sync",
      replication: "data-sync",
      rates: "network-market",
      ai: "assistant-ai",
      assistant: "assistant-ai",
      data: "data-storage",
      storage: "data-storage",
    } as const;

    for (const [slug, section] of Object.entries(expected)) {
      expect(settingsSectionForHash(slug)).toBe(section);
    }
  });

  it("treats menu aliases as equivalents", () => {
    expect(settingsSectionForHash("replication")).toBe(
      settingsSectionForHash("sync"),
    );
    expect(settingsSectionForHash("assistant")).toBe(
      settingsSectionForHash("ai"),
    );
    expect(settingsSectionForHash("desktop")).toBe(
      settingsSectionForHash("terminal"),
    );
  });

  it("normalizes leading hash, surrounding whitespace, and case", () => {
    expect(settingsSectionForHash("#privacy")).toBe("security-privacy");
    expect(settingsSectionForHash("  Privacy  ")).toBe("security-privacy");
    expect(settingsSectionForHash("AI")).toBe("assistant-ai");
  });

  it("returns null for missing or unknown sections", () => {
    expect(settingsSectionForHash("")).toBeNull();
    expect(settingsSectionForHash("#")).toBeNull();
    expect(settingsSectionForHash("nonexistent")).toBeNull();
  });
});
