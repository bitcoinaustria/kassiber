import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AcquisitionReview, AcquisitionBlocker } from "./AcquisitionReview";
import { toDashboardTransaction } from "./dashboard/model";

vi.mock("@/daemon/client", () => ({ useDaemonMutation: () => ({ isPending: false, mutateAsync: vi.fn() }) }));
const transaction = toDashboardTransaction({ id: "incoming", date: "2026-04-15 08:00", type: "Buy", account: "Synthetic wallet", counter: "Source", amountSat: 600_000, eur: 300, rate: 50_000, tag: "", conf: 3, feeSat: 0 }, 0);

describe("separate acquisition review control", () => {
  it("requires a preview and discloses the supported valuation contract", () => {
    const html = renderToStaticMarkup(<AcquisitionReview transaction={transaction} dirty={false} hideSensitive={false} />);
    expect(html).toContain("Preview classification");
    expect(html).not.toContain("Apply reviewed classification");
    expect(html).toContain("Zero-cost Austrian acquisitions are not supported");
    expect(html).toContain("taxable lending-style returns");
  });
  it("does not mix the acquisition declaration with other unsaved fields", () => {
    const html = renderToStaticMarkup(<AcquisitionReview transaction={transaction} dirty hideSensitive={false} />);
    expect(html).toContain("Save or discard your other edits");
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Preview classification<\/button>/);
  });
});


it("explains acquisition blockers with an explicit fallback", () => {
  expect(renderToStaticMarkup(<AcquisitionBlocker reason="acquisition_valuation_unsupported" />)).toContain("not supported for the selected tax regime");
  expect(renderToStaticMarkup(<AcquisitionBlocker reason="basis_provenance_incomplete" />)).toContain("acquisition basis is incomplete");
  expect(renderToStaticMarkup(<AcquisitionBlocker reason="future_internal_code" />)).toContain("Open the book review queue");
});
