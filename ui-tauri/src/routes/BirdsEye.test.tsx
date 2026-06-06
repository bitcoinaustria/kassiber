import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { MOCK_PROFILES } from "@/mocks/profiles";
import {
  MOCK_WORKSPACE_OVERVIEW,
  mockWorkspaceOverviewSnapshot,
} from "@/mocks/workspaceOverview";
import { router } from "@/routeTree";
import { WorkspaceSection } from "@/routes/Books";

import { BookRow } from "./BirdsEye";

describe("Bird's Eye route and model", () => {
  it("registers the workspace Bird's Eye route", () => {
    expect(router.routesByPath["/books/$workspaceId/birds-eye"]).toBeTruthy();
  });

  it("marks mixed fiat as partial and keeps per-book fiat rows", () => {
    const snapshot = mockWorkspaceOverviewSnapshot("w3");

    expect(snapshot.fiat.mode).toBe("single");
    expect(snapshot.fiat.books).toHaveLength(1);

    const mixed = {
      ...MOCK_WORKSPACE_OVERVIEW,
      fiat: {
        ...MOCK_WORKSPACE_OVERVIEW.fiat,
        mode: "mixed" as const,
        mixed: true,
        partial: true,
        fiatCurrency: null,
        eurBalance: null,
        currencies: ["CHF", "EUR"],
      },
      status: { ...MOCK_WORKSPACE_OVERVIEW.status, mixedFiat: true },
    };

    expect(mixed.fiat.partial).toBe(true);
    expect(mixed.fiat.eurBalance).toBeNull();
    expect(mixed.fiat.currencies).toEqual(["CHF", "EUR"]);
    expect(mixed.fiat.books.map((row) => row.profileLabel)).toContain("Alice");
  });
});

describe("Bird's Eye rendering", () => {
  it("renders all per-book drilldown buttons", () => {
    const html = renderToStaticMarkup(
      <BookRow
        book={MOCK_WORKSPACE_OVERVIEW.books[0]}
        hideSensitive={false}
        disabled={false}
        onOpenRoute={vi.fn()}
      />,
    );

    expect(html).toContain("Overview");
    expect(html).toContain("Transactions");
    expect(html).toContain("Ledger");
    expect(html).toContain("Quarantine");
    expect(html).toContain("Wallets");
    expect(html).toContain("Reports");
  });

  it("places the Bird's Eye action in each Books workspace header", () => {
    const workspace = MOCK_PROFILES.workspaces[0];
    const html = renderToStaticMarkup(
      <WorkspaceSection
        workspace={workspace}
        activeId={MOCK_PROFILES.activeProfileId}
        onCreateProfile={vi.fn()}
        onOpenBirdsEye={vi.fn()}
        onPick={vi.fn()}
        onRename={vi.fn()}
        onRenameWorkspace={vi.fn()}
      />,
    );

    expect(html).toContain("Bird&#x27;s Eye");
    expect(html).toContain(`data-testid="birds-eye-${workspace.id}"`);
  });
});
