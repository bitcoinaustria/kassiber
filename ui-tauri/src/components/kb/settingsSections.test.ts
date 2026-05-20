import { describe, expect, it } from "vitest";

import { settingsSectionForHash } from "./settingsSections";

describe("settings section hash mapping", () => {
  // The native menu, deep-link parser (Rust side), direct URL navigation, and
  // the in-app section rail all funnel through this helper. Pinning the
  // canonical sections + the alias map makes accidental rename or removal of a
  // section id a test failure rather than a runtime "panel never re-opens" bug.
  it("maps each Rust deep-link slug to a section", () => {
    expect(settingsSectionForHash("privacy")).toBe("security-privacy");
    expect(settingsSectionForHash("developer")).toBe("desktop-developer");
    expect(settingsSectionForHash("logs")).toBe("desktop-developer");
    expect(settingsSectionForHash("display")).toBe("general-appearance");
    expect(settingsSectionForHash("security")).toBe("security-lock");
    expect(settingsSectionForHash("backends")).toBe("network-bitcoin");
    expect(settingsSectionForHash("sync")).toBe("network-bitcoin");
    expect(settingsSectionForHash("rates")).toBe("network-market");
    expect(settingsSectionForHash("ai")).toBe("assistant-ai");
    expect(settingsSectionForHash("data")).toBe("data-storage");
  });

  it("maps the canonical layer-forward slugs", () => {
    expect(settingsSectionForHash("appearance")).toBe("general-appearance");
    expect(settingsSectionForHash("explorers")).toBe("network-bitcoin");
    expect(settingsSectionForHash("bitcoin")).toBe("network-bitcoin");
    expect(settingsSectionForHash("lightning")).toBe("network-lightning");
    expect(settingsSectionForHash("liquid")).toBe("network-liquid");
    expect(settingsSectionForHash("market")).toBe("network-market");
    expect(settingsSectionForHash("terminal")).toBe("desktop-terminal");
  });

  it("treats menu aliases as equivalents", () => {
    expect(settingsSectionForHash("sync")).toBe(
      settingsSectionForHash("backends"),
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
