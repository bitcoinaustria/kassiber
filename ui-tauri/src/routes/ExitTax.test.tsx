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

it.each(["en", "de"])("puts missing-FMV readiness above enabled exports in %s", async (language) => {
  await i18n.changeLanguage(language);
  state.report = buildExitTaxFixture();
  state.report.status = { needsJournals: false, quarantines: 0, missingFmvAssets: ["BTC"] };
  state.report.fmvSource[0] = { asset: "BTC", pair: "BTC-EUR", rate: null, source: "missing", asOf: null };
  const html = renderToStaticMarkup(<ExitTax />);
  const title = language === "en" ? "This estimate is incomplete." : "Diese Schätzung ist unvollständig.";
  expect(html).toContain(title);
  expect(html.indexOf(title)).toBeLessThan(html.indexOf("Export PDF"));
  const missing = language === "en" ? "Estimate incomplete: no usable FMV quote for BTC." : "Schätzung unvollständig: kein verwendbarer Marktwertkurs für BTC.";
  expect(html.indexOf(missing)).toBeLessThan(html.indexOf("Export PDF"));
  expect(html).toContain(language === "en" ? "Incomplete draft — exports remain available for review." : "Unvollständiger Entwurf — Exporte bleiben zur Prüfung verfügbar.");
  for (const format of ["PDF", "XLSX"]) {
    const button = html.match(new RegExp(`<button[^>]*>(?:(?!<\\/button>).)*Export ${format}<\\/button>`, "s"))?.[0];
    expect(button).toBeDefined();
    expect(button).not.toMatch(/\sdisabled(?:=|\s|>)/);
  }
});

it.each([
  ["2024-12-31T12:34:56.123456+01:00", "2024-12-31T11:34:56.123456Z"],
  ["2024-12-31T12:34:56-02:00", "2024-12-31T14:34:56Z"],
])("renders offset quote %s in UTC without losing precision", (asOf, utc) => {
  state.report = buildExitTaxFixture();
  state.report.fmvSource[0].asOf = asOf;
  const html = renderToStaticMarkup(<ExitTax />);
  expect(html).toContain(`<time dateTime="${utc}">${utc}</time>`);
  expect(html).not.toContain(asOf);
});
