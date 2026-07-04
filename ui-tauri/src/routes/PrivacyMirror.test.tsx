import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";
import { router } from "@/routeTree";
import { fixtures } from "@/daemon/fixtures";

import { PrivacyMirrorPayloadView } from "./PrivacyMirror";

describe("PrivacyMirror route", () => {
  it("registers the dedicated Privacy Mirror page route", () => {
    expect(router.routesByPath["/privacy-mirror"]).toBeTruthy();
  });

  it("renders the north-star sections and mobile stack from the redacted payload", () => {
    const html = renderToStaticMarkup(
      <PrivacyMirrorPayloadView
        payload={fixtures["ui.reports.privacy_mirror"] as never}
      />,
    );

    expect(html).toContain('data-testid="privacy-mirror-page"');
    expect(html).toContain('data-testid="privacy-mirror-mobile-stack"');
    expect(html).toContain("Worst local privacy risk");
    expect(html).toContain("Adversaries");
    expect(html).toContain("Wallets");
    expect(html).toContain("Transactions");
    expect(html).toContain("UTXOs");
    expect(html).toContain("Timeline");
    expect(html).toContain("Evidence");
    expect(html).toContain("PSBT");
  });

  it("keeps AI/export-redacted material out of the rendered mirror", () => {
    const html = renderToStaticMarkup(
      <PrivacyMirrorPayloadView
        payload={fixtures["ui.reports.privacy_mirror"] as never}
      />,
    );

    expect(html).not.toContain("xpub");
    expect(html).not.toContain("descriptor");
    expect(html).not.toContain("bc1q");
    expect(html).not.toContain("script_pubkey");
    expect(html).not.toContain("raw_json");
  });
});
