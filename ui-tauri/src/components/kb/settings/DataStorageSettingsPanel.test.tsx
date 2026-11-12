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

describe("DataStorageSettingsPanel database size", () => {
  it("shows the database size and notes a non-empty write-ahead log", () => {
    const html = renderPanel({
      storage: { database_bytes: 5 * 1024 * 1024, wal_bytes: 2048 },
    });
    expect(html).toContain("Database size");
    expect(html).toContain("5.0 MiB");
    expect(html).toContain("Plus 2.0 KiB in the write-ahead log");
  });

  it("stays quiet about an empty log, and says when the size is unknown", () => {
    const empty = renderPanel({ storage: { database_bytes: 1000, wal_bytes: 0 } });
    expect(empty).toContain("1000 B");
    expect(empty).not.toContain("write-ahead log");
    expect(renderPanel({ storage: { database_bytes: null, wal_bytes: null } })).toContain(
      "Unknown",
    );
    expect(renderPanel({ storage: null })).toContain("Loading");
  });
});
