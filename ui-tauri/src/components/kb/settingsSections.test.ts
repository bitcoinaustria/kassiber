import { describe, expect, it } from "vitest";

import { selectedIntegrationForHash } from "./settingsSections";

describe("settings integration hash mapping", () => {
  // The native menu, deep-link parser (Rust side), and direct URL
  // navigation all funnel through this helper. Pinning the canonical
  // sections + the alias map makes accidental rename or removal of an
  // integration ID a test failure rather than a runtime "panel never
  // re-opens" bug.
  it("maps each canonical settings section to an integration", () => {
    expect(selectedIntegrationForHash("privacy")).toBe("privacy-sensitive");
    expect(selectedIntegrationForHash("display")).toBe("display-currency");
    expect(selectedIntegrationForHash("security")).toBe("security-lock-now");
    expect(selectedIntegrationForHash("backends")).toBe("sync-add-backend");
    expect(selectedIntegrationForHash("rates")).toBe("rate-providers");
    expect(selectedIntegrationForHash("ai")).toBe("ai-providers");
    expect(selectedIntegrationForHash("data")).toBe("data-root");
  });

  it("treats `sync` and `assistant` as aliases", () => {
    expect(selectedIntegrationForHash("sync")).toBe(
      selectedIntegrationForHash("backends"),
    );
    expect(selectedIntegrationForHash("assistant")).toBe(
      selectedIntegrationForHash("ai"),
    );
  });

  it("normalizes leading hash, surrounding whitespace, and case", () => {
    expect(selectedIntegrationForHash("#privacy")).toBe("privacy-sensitive");
    expect(selectedIntegrationForHash("  Privacy  ")).toBe("privacy-sensitive");
    expect(selectedIntegrationForHash("AI")).toBe("ai-providers");
  });

  it("returns null for missing or unknown sections", () => {
    expect(selectedIntegrationForHash("")).toBeNull();
    expect(selectedIntegrationForHash("#")).toBeNull();
    expect(selectedIntegrationForHash("nonexistent")).toBeNull();
  });
});
