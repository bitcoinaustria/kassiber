import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";
import { router } from "@/routeTree";
import { fixtures } from "@/daemon/fixtures";

import { PrivacyMirrorPayloadView } from "./PrivacyMirror";

// The elevated PSBT panel uses a daemon mutation, so the view needs a query
// client in context; the mutation is idle at render so no transport is needed.
function renderMirror() {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <PrivacyMirrorPayloadView
        payload={fixtures["ui.reports.privacy_mirror"] as never}
      />
    </QueryClientProvider>,
  );
}

describe("PrivacyMirror route", () => {
  it("registers the dedicated Privacy Mirror page route", () => {
    expect(router.routesByPath["/privacy-mirror"]).toBeTruthy();
  });

  it("renders the score hero, primary recommendation, findings, and detail sections", () => {
    const html = renderMirror();

    expect(html).toContain('data-testid="privacy-mirror-page"');
    expect(html).toContain('data-testid="privacy-score-grade"');
    expect(html).toContain('data-testid="privacy-mirror-worst-risk"');
    expect(html).toContain('data-testid="privacy-mirror-findings"');
    // Score hero: with weighted tells the mock payload lands at grade C.
    expect(html).toContain("notable exposure");
    expect(html).toContain("What to fix first");
    // The worst risk is shown in plain language, not the engine's phrasing.
    expect(html).toContain("Wallets linked by a shared-input spend");
    // Machine tell tokens are humanized into readable finding titles.
    expect(html).toContain("Common input");
    // Detail sections remain as collapsible triggers.
    expect(html).toContain("Who can infer it");
    expect(html).toContain("The evidence");
    expect(html).toContain("All details");
    // Grounded score: the waterfall shows real factor counts, not a made-up base.
    expect(html).toContain("Linked wallets");
    expect(html).toContain("Origin coverage");
    // Pre-broadcast check is elevated to its own visible section, not buried.
    expect(html).toContain('data-testid="privacy-mirror-psbt"');
    // The old tab shell and its duplicate mobile stack are gone.
    expect(html).not.toContain('data-testid="privacy-mirror-mobile-stack"');
    expect(html).not.toContain('role="tablist"');
  });

  it("keeps AI/export-redacted material out of the rendered mirror", () => {
    const html = renderMirror();

    expect(html).not.toContain("xpub");
    expect(html).not.toContain("descriptor");
    expect(html).not.toContain("bc1q");
    expect(html).not.toContain("script_pubkey");
    expect(html).not.toContain("raw_json");
  });
});
