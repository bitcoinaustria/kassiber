import { renderToStaticMarkup } from "react-dom/server";
import type { PropsWithChildren } from "react";
import { describe, expect, it, vi } from "vitest";

vi.mock("recharts", async (importOriginal) => {
  const original = await importOriginal<typeof import("recharts")>();

  return {
    ...original,
    ResponsiveContainer: ({
      children,
      debounce,
    }: PropsWithChildren<{ debounce?: number }>) => (
      <div data-responsive-debounce={debounce}>{children}</div>
    ),
  };
});

import { ChartContainer } from "./chart";

describe("ChartContainer responsive sizing", () => {
  it("paces resize updates to one display frame", () => {
    const html = renderToStaticMarkup(
      <ChartContainer config={{ primary: { color: "#000" } }}>
        <div />
      </ChartContainer>,
    );

    expect(html).toContain('data-responsive-debounce="16"');
  });
});
