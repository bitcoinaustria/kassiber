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
  it("names the servers before the user commits to them", () => {
    const html = renderToStaticMarkup(
      <ConnectionsFields
        form={DEFAULT_FORM}
        update={vi.fn()}
        defaultBackends={BACKENDS}
      />,
    );

    expect(html).toContain("Kassiber connects to these servers when you sync");
    expect(html).toContain("https://mempool.example/api");
    expect(html).toContain("ssl://fulcrum.example:50002");
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

  it("lists no public servers once another option is chosen", () => {
    const html = renderToStaticMarkup(
      <ConnectionsFields
        form={{ ...DEFAULT_FORM, backendSetupMode: "skip" }}
        update={vi.fn()}
        defaultBackends={BACKENDS}
      />,
    );

    expect(html).not.toContain("https://mempool.example/api");
  });
});
