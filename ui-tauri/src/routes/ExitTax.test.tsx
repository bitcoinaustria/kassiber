import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { buildExitTaxFixture } from "@/mocks/exitTax";

import { ExitTax } from "./ExitTax";

const state = vi.hoisted(() => ({ report: {} as ReturnType<typeof buildExitTaxFixture> }));

vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({ data: { data: state.report }, isLoading: false, isError: false }),
  useDaemonMutation: () => ({ mutate: vi.fn() }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

afterEach(async () => { await i18n.changeLanguage("en"); });

describe("exit-tax FMV disclosure", () => {
  it.each(["cache", "transaction"] as const)("shows the %s quote date in the actual route", (source) => {
    state.report = buildExitTaxFixture();
    state.report.fmvSource[0] = {
      asset: "BTC", pair: "BTC-EUR", rate: 65000,
      source, asOf: "2024-03-31T23:59:59.500000Z",
    };
    const html = renderToStaticMarkup(<ExitTax />);
    expect(html).toContain("2024-03-31T23:59:59.500000Z</time>");
    expect(html).toContain(source === "cache" ? "Cache" : "Transaction");
    expect(html).toContain("As of:");
  });

  it.each(["en", "de"])("makes missing FMV visibly incomplete in %s", async (language) => {
    await i18n.changeLanguage(language);
    state.report = buildExitTaxFixture();
    state.report.fmvSource[0] = { asset: "BTC", pair: "BTC-EUR", rate: null, source: "missing", asOf: null };
    state.report.status.missingFmvAssets = ["BTC"];
    const html = renderToStaticMarkup(<ExitTax />);
    expect(html).toContain(language === "en" ? "Estimate incomplete: no usable FMV quote for BTC." : "Schätzung unvollständig: kein verwendbarer Marktwertkurs für BTC.");
    expect(html).toContain(language === "en" ? "Unavailable" : "Nicht verfügbar");
  });
});
