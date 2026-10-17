import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM } from "../constants";
import { ReviewStep } from "./ReviewStep";

describe("ReviewStep grouping", () => {
  it("leads with what reaches the network, then what stays local", () => {
    const html = renderToStaticMarkup(
      <ReviewStep
        form={DEFAULT_FORM}
        update={vi.fn()}
        onSubmit={vi.fn()}
        currentStep={4}
        totalSteps={5}
        backendPreviewRows={[
          { name: "mempool", kind: "esplora", url: "https://mempool.example/api" },
        ]}
      />,
    );

    const network = html.indexOf("Connects to the network");
    const local = html.indexOf("Stays on this machine");
    expect(network).toBeGreaterThanOrEqual(0);
    expect(local).toBeGreaterThan(network);

    // Sync, with its servers, sits in the network group; storage in the local one.
    const servers = html.indexOf("https://mempool.example/api");
    expect(servers).toBeGreaterThan(network);
    expect(servers).toBeLessThan(local);
    expect(html.indexOf("SQLCipher encrypted database")).toBeGreaterThan(local);
  });
});
