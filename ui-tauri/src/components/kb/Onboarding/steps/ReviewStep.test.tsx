import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM as UNCHOSEN_FORM } from "../constants";

// The built-in servers chosen, as most of these tests need.
const DEFAULT_FORM = { ...UNCHOSEN_FORM, backendSetupMode: "default" as const };
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
          {
            name: "mempool",
            kind: "esplora",
            url: "https://mempool.example/api",
            chain: "bitcoin",
          },
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

  it("states what each connection reaches, including what stays off", () => {
    const html = renderToStaticMarkup(
      <ReviewStep
        form={{ ...DEFAULT_FORM, updateChecksEnabled: true }}
        update={vi.fn()}
        onSubmit={vi.fn()}
        currentStep={4}
        totalSteps={5}
      />,
    );

    expect(html).toContain("Internet · when you sync");
    expect(html).toContain("This device · when you use the assistant");
    expect(html).toContain("Internet · at launch, then daily");
    expect(html).toContain("api.github.com");
    expect(html).toContain("Off until you set it up");
  });

  it("shows no connections for AI and updates in offline setup", () => {
    const html = renderToStaticMarkup(
      <ReviewStep
        form={{
          ...DEFAULT_FORM,
          backendSetupMode: "skip",
          updateChecksEnabled: true,
        }}
        update={vi.fn()}
        onSubmit={vi.fn()}
        currentStep={4}
        totalSteps={5}
      />,
    );

    expect(html).not.toContain("when you sync");
    expect(html).not.toContain("when you use the assistant");
    expect(html).not.toContain("api.github.com");
  });

  it("lists the Liquid servers only when setup adds them", () => {
    const rows = [
      {
        name: "fulcrum",
        kind: "electrum",
        url: "ssl://fulcrum.example:50002",
        chain: "bitcoin",
      },
      {
        name: "liquid",
        kind: "electrum",
        url: "ssl://liquid.example:995",
        chain: "liquid",
      },
    ];
    const render = (includeLiquidBackends: boolean) =>
      renderToStaticMarkup(
        <ReviewStep
          form={{ ...DEFAULT_FORM, includeLiquidBackends }}
          update={vi.fn()}
          onSubmit={vi.fn()}
          currentStep={4}
          totalSteps={5}
          backendPreviewRows={rows}
        />,
      );

    expect(render(false)).toContain("ssl://fulcrum.example:50002");
    expect(render(false)).not.toContain("ssl://liquid.example:995");
    expect(render(true)).toContain("ssl://liquid.example:995");
  });
});
