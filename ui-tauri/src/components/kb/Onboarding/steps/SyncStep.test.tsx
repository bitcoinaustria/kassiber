import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM } from "../constants";
import { SyncStep } from "./SyncStep";

vi.mock("../ConnectionsFields", () => ({
  ConnectionsFields: () => <div>Sync backends</div>,
}));

vi.mock("../explainers", () => ({
  SyncExplainer: () => <div>Sync explainer</div>,
}));

describe("SyncStep update privacy", () => {
  it("places GitHub update consent below the sync backend choices", () => {
    const html = renderToStaticMarkup(
      <SyncStep
        form={DEFAULT_FORM}
        update={vi.fn()}
        onSubmit={vi.fn()}
        goBack={vi.fn()}
        currentStep={2}
        totalSteps={5}
      />,
    );

    const backends = html.indexOf("Sync backends");
    const updates = html.indexOf("App updates");

    expect(backends).toBeGreaterThanOrEqual(0);
    expect(updates).toBeGreaterThan(backends);
    expect(html).toContain("Allow Kassiber to check GitHub for updates");
    expect(html).toContain("GitHub sees your IP address");
    expect(html).toContain('id="allow-update-checks"');
  });
});
