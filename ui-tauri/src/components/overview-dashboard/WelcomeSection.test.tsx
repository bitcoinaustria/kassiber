import { createElement } from "react";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to?: string }) => <a href={to}>{children}</a>,
}));
vi.mock("@/daemon/client", () => ({ useDaemon: () => ({ data: undefined }) }));

import { MOCK_OVERVIEW } from "@/mocks/seed";

import { WelcomeSection } from "./WelcomeSection";

const render = (snapshot: typeof MOCK_OVERVIEW) =>
  renderToStaticMarkup(
    createElement(WelcomeSection, {
      snapshot,
      onAddConnection: () => undefined,
      onProcessJournals: () => undefined,
      isProcessingJournals: false,
    }),
  );

describe("overview readiness pill", () => {
  const status = { ...MOCK_OVERVIEW.status!, needsJournals: true };

  it("reprocesses stale journals in place", () => {
    const html = render({ ...MOCK_OVERVIEW, status });
    expect(html).toContain("<button");
    expect(html).toContain("Reprocess journals");
  });

  it("opens Wallets when a source error comes before stale journals", () => {
    const connections = [{ ...MOCK_OVERVIEW.connections[0], status: "error" as const }];
    const html = render({ ...MOCK_OVERVIEW, status, connections });
    expect(html).toContain('href="/connections"');
    expect(html).toContain("Source attention");
    expect(html).not.toContain("<button type=\"button\" class=\"inline-flex h-7");
  });
});
