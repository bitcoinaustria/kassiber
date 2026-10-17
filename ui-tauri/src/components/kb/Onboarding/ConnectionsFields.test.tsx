import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM } from "./constants";
import { ConnectionsFields } from "./ConnectionsFields";

vi.mock("@/daemon/client", () => ({
  useDaemonMutation: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

const BACKENDS = [
  { name: "mempool", kind: "esplora", url: "https://mempool.example/api" },
  { name: "fulcrum", kind: "electrum", url: "ssl://fulcrum.example:50002" },
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
});
