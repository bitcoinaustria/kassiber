import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => async () => {},
}));

import { TransactionWorkbench } from "./TransactionWorkbench";

describe("transaction workbench chart", () => {
  it("keeps a height at every responsive breakpoint", () => {
    const html = renderToStaticMarkup(
      <TransactionWorkbench
        period="all"
        records={[]}
        hideSensitive={false}
        currency="btc"
        chartSelection={null}
        onFlowSelectionChange={() => {}}
        onQuickFilterChange={() => {}}
        onBreakdownSelectionChange={() => {}}
        onTableFiltersReset={() => {}}
      />,
    );

    expect(html).toContain("min-h-[360px]");
    expect(html).not.toContain("xl:min-h-0");
  });
});
