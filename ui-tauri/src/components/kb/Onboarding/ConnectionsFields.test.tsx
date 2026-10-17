import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM as UNCHOSEN_FORM } from "./constants";

// The built-in servers chosen, as most of these tests need.
const DEFAULT_FORM = { ...UNCHOSEN_FORM, backendSetupMode: "default" as const };
import { ConnectionsFields } from "./ConnectionsFields";

vi.mock("@/daemon/client", () => ({
  useDaemonMutation: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

const BACKENDS = [
  { name: "mempool", kind: "esplora", url: "https://mempool.example/api", chain: "bitcoin" },
  { name: "fulcrum", kind: "electrum", url: "ssl://fulcrum.example:50002", chain: "bitcoin" },
];

describe("ConnectionsFields public backends", () => {
  it("names the servers and when they are contacted before a choice", () => {
    const html = renderToStaticMarkup(
      <ConnectionsFields
        form={DEFAULT_FORM}
        update={vi.fn()}
        defaultBackends={BACKENDS}
      />,
    );

    expect(html).toContain("Internet · when you sync");
    expect(html).toContain(">mempool.example<");
    expect(html).toContain(">fulcrum.example:50002<");
  });

  it("shows every option's privacy trade-off, not only the chosen one", () => {
    const html = renderToStaticMarkup(
      <ConnectionsFields
        form={DEFAULT_FORM}
        update={vi.fn()}
        defaultBackends={BACKENDS}
      />,
    );

    expect(html).toContain("The operators can see which addresses you query");
    expect(html).toContain("Best privacy and sovereignty");
  });

  it("keeps every option's reach visible whichever is chosen", () => {
    const html = renderToStaticMarkup(
      <ConnectionsFields
        form={{ ...DEFAULT_FORM, backendSetupMode: "skip" }}
        update={vi.fn()}
        defaultBackends={BACKENDS}
      />,
    );

    expect(html).toContain(">mempool.example<");
    expect(html).toContain(">fulcrum.example:50002<");
    expect(html).toContain("Your server · when you sync");
    expect(html).toContain("No connections");
  });

  it("keeps Liquid out of the built-in servers unless the user adds it", () => {
    const withLiquid = [
      ...BACKENDS,
      {
        name: "liquid",
        kind: "electrum",
        url: "ssl://liquid.example:995",
        chain: "liquid",
      },
    ];
    const bitcoinOnly = renderToStaticMarkup(
      <ConnectionsFields
        form={DEFAULT_FORM}
        update={vi.fn()}
        defaultBackends={withLiquid}
      />,
    );
    expect(bitcoinOnly).toContain(">mempool.example<");
    expect(bitcoinOnly).not.toContain(">liquid.example:995<");
    expect(bitcoinOnly).toContain("Also add Liquid servers");
    expect(bitcoinOnly).toMatch(/aria-checked="false"[^>]*id="include-liquid-backends"/);

    const optedIn = renderToStaticMarkup(
      <ConnectionsFields
        form={{ ...DEFAULT_FORM, includeLiquidBackends: true }}
        update={vi.fn()}
        defaultBackends={withLiquid}
      />,
    );
    expect(optedIn).toContain(">liquid.example:995<");
  });

  it("starts with no option chosen", () => {
    const html = renderToStaticMarkup(
      <ConnectionsFields
        form={UNCHOSEN_FORM}
        update={vi.fn()}
        defaultBackends={BACKENDS}
      />,
    );
    expect(html).not.toContain('aria-pressed="true"');
    // Each option still says what it would reach.
    expect(html).toContain(">mempool.example<");
    expect(html).toContain("No connections");
  });
});
