import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DataStorageSettingsPanel } from "./DataStorageSettingsPanel";

const renderPanel = (
  overrides: Partial<Parameters<typeof DataStorageSettingsPanel>[0]> = {},
) =>
  // Static markup HTML-escapes "&" to "&amp;"; decode so assertions read naturally.
  renderToStaticMarkup(
    <DataStorageSettingsPanel
      status={null}
      onOpenImports={vi.fn()}
      onResetWelcome={vi.fn()}
      onResetBook={vi.fn()}
      resetBookDisabled={false}
      onDeleteBooks={vi.fn()}
      deleteBooksDisabled={false}
      {...overrides}
    />,
  ).replaceAll("&amp;", "&");

describe("DataStorageSettingsPanel regtest reset", () => {
  it("hides the regtest reset control by default (production app)", () => {
    const html = renderPanel();
    expect(html).toContain("Delete books");
    expect(html).not.toContain("Reset & rebuild");
    expect(html).not.toContain("Reset regtest demo environment");
  });

  it("shows the regtest reset control only when available", () => {
    const html = renderPanel({ resetRegtestAvailable: true });
    expect(html).toContain("Reset regtest demo environment");
    expect(html).toContain("Reset & rebuild");
  });

  it("reflects the pending state while rebuilding", () => {
    const html = renderPanel({ resetRegtestAvailable: true, resetRegtestPending: true });
    expect(html).toContain("Rebuilding…");
  });
});
