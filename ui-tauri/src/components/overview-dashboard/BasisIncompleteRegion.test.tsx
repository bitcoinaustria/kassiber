import { Children, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { UNKNOWN_FIAT_COMPLETENESS } from "@/lib/fiatCompleteness";
import { MOCK_OVERVIEW, type FiatCompleteness } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";
import { BasisIncompleteRegion } from "./BasisIncompleteRegion";
import { BtcActivityChart } from "./BtcActivityChart";
import { basisIncompleteRange, powerLawDaysFor } from "./model";

vi.mock("@/components/ui/chart", () => ({
  ChartContainer: ({ children }: { children: ReactNode }) => children,
}));

// Static rendering has no Recharts layout effects. Supply the measured plot
// and its scale, while rendering the real annotation and translations.
vi.mock("recharts", async (importOriginal) => ({
  ...await importOriginal<typeof import("recharts")>(),
  ComposedChart: ({ children }: { children: ReactNode }) => (
    <svg>{Children.toArray(children).filter((child) =>
      isValidElement(child) && child.type === BasisIncompleteRegion,
    )}</svg>
  ),
  usePlotArea: () => ({ x: 68, y: 44, width: 600, height: 240 }),
  useXAxisScale: () => (value: unknown) => {
    if (typeof value === "number") {
      const first = powerLawDaysFor(Date.parse("2026-01-01T00:00:00Z"));
      const last = powerLawDaysFor(Date.parse("2026-01-04T00:00:00Z"));
      return 68 + 600 * Math.log(value / first) / Math.log(last / first);
    }
    return ({ "2026-01-01": 68, "2026-01-02": 268, "2026-01-03": 468, "2026-01-04": 668 })[String(value)];
  },
}));

const points = ["2026-01-01", "2026-01-02", "2026-01-03", "2026-01-04"].map((date) => ({
  date,
  sortTimeMs: Date.parse(`${date}T00:00:00Z`),
}));
const incomplete: FiatCompleteness = {
  ...UNKNOWN_FIAT_COMPLETENESS,
  state: "incomplete",
  earliestIncompleteAt: "2026-01-03T10:00:00Z",
};

function renderRegion(
  completeness = incomplete,
  selected = points,
  visible = true,
  logTime = false,
) {
  const range = basisIncompleteRange(completeness, selected, visible);
  return renderToStaticMarkup(
    <svg>{range && <BasisIncompleteRegion range={range} logTime={logTime} />}</svg>,
  );
}

afterEach(async () => {
  useUiStore.setState({ lang: "en" });
  await i18n.changeLanguage("en");
});

describe("basis frost annotation", () => {
  it("covers the first incomplete day through the right edge with a neutral fill and dated label", () => {
    const html = renderRegion();
    expect(html).toContain('<rect x="468" y="44" width="200" height="240"');
    expect(html).toContain('fill="color-mix(in oklch, var(--muted) 85%, var(--foreground))"');
    expect(html).toContain('fill-opacity="0.3"');
    expect(html).toContain('stroke-width="1" stroke-dasharray="3 4"');
    expect(html).toContain("Cost basis incomplete from Jan 3, 2026");
    expect(html).toContain('<foreignObject x="68" y="4" width="600" height="32">');
    expect(html).toContain("text-muted-foreground");
    expect(html).toContain('pointer-events="none"');
  });

  it.each([null, "invalid-date"])("frosts the whole plot when the start is unknown (%s)", (earliestIncompleteAt) => {
    const html = renderRegion({ ...incomplete, earliestIncompleteAt });
    expect(html).toContain('<rect x="68" y="44" width="600" height="240"');
    expect(html).toContain("Cost basis incomplete</div>");
    expect(html).not.toContain("<line");
  });

  it("omits the region and label for complete basis or a hidden basis series", () => {
    expect(renderRegion({ ...incomplete, costBasisComplete: true })).toBe("<svg></svg>");
    expect(renderRegion(incomplete, points, false)).toBe("<svg></svg>");
  });

  it("follows brush windows before and after the gap and handles empty windows", () => {
    expect(renderRegion(incomplete, points.slice(0, 2))).toBe("<svg></svg>");
    const afterGap = renderRegion(incomplete, points.slice(2));
    expect(afterGap).toContain('<rect x="68" y="44" width="600" height="240"');
    expect(afterGap).toContain("Cost basis incomplete from Jan 3, 2026");
    expect(afterGap).not.toContain("<line");
    expect(renderRegion(incomplete, [])).toBe("<svg></svg>");
  });

  it("uses the existing log time scale at the beginning of the gap day", () => {
    const html = renderRegion(incomplete, points, true, true);
    const x = Number(html.match(/<rect x="([^"]+)"/)?.[1]);
    expect(x).toBeGreaterThan(468);
    expect(x).toBeLessThan(469);
    expect(html).toContain("Cost basis incomplete from Jan 3, 2026");
  });

  it("uses Austrian German copy and date formatting", async () => {
    useUiStore.setState({ lang: "de" });
    await i18n.changeLanguage("de");
    expect(renderRegion()).toContain("Anschaffungskosten ab 3. Jän. 2026 unvollständig");
    expect(renderRegion({ ...incomplete, earliestIncompleteAt: null })).toContain("Anschaffungskosten unvollständig</div>");
  });

  it("keeps the chart annotation in hidden-values mode and omits it when fiat series are disabled", () => {
    const snapshot = {
      ...MOCK_OVERVIEW,
      fiat: { ...MOCK_OVERVIEW.fiat, completeness: { ...incomplete, earliestIncompleteAt: null } },
    };
    const renderChart = (fiatSeriesEnabled: boolean) => renderToStaticMarkup(
      <BtcActivityChart snapshot={snapshot} hideSensitive currency="eur" fiatSeriesEnabled={fiatSeriesEnabled} />,
    );
    expect(renderChart(true)).toContain('class="basis-incomplete-region"');
    expect(renderChart(false)).not.toContain('class="basis-incomplete-region"');
  });
});
