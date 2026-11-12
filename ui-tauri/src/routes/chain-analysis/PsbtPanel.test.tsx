import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import { DaemonScopeContext } from "@/daemon/client";
import { bookEvidenceNetwork, PsbtPanel } from "./PsbtPanel";

const mock = vi.hoisted(() => ({ invoke: vi.fn(), pick: vi.fn(), binding: { environment_id: null as string | null, domains: [] as {chain:string;network:string}[] }, wallets: [] as { environments: string[] }[], effects: [] as Array<() => void | (() => void)> }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useEffect: (effect: () => void | (() => void)) => { mock.effects.push(effect); },
}));
vi.mock("@/daemon/client", async original => ({
  ...await original<typeof import("@/daemon/client")>(),
  useDaemon: (kind: string) => kind === "ui.networks.inventory"
    ? ({ data: { data: { wallets: mock.wallets } } })
    : ({ data: { data: mock.binding } }),
  useDaemonMutation: () => ({ mutateAsync: mock.invoke, isPending: false }),
}));
vi.mock("@/lib/filePicker", async original => ({
  ...await original<typeof import("@/lib/filePicker")>(),
  isFilePickerAvailable: true,
  pickChainAnalysisSource: mock.pick,
}));
function render(initialNetwork?: string) {
  return renderToStaticMarkup(<DaemonScopeContext.Provider value={{ expectedScope: { workspace_id: "w", profile_id: "p" }, daemonSession: 1, isCurrent: () => true }}><PsbtPanel initialNetwork={initialNetwork} onError={() => {}} /></DaemonScopeContext.Provider>);
}
beforeEach(() => { mock.binding = {environment_id:null, domains:[]}; mock.wallets = []; mock.invoke.mockReset(); mock.pick.mockReset(); mock.effects.length = 0; });

describe("PSBT workbench entry", () => {
  it.each(["main", "test", "signet", "regtest"])("preserves the explicit %s network without starting work", async network => {
    const html = render(network);
    expect(html).toContain(`Network: ${network}`);
    expect(html).not.toContain("<select");
    // Opening the surface, including effect replay, is not permission to pick,
    // parse, compare or start an entropy job.
    for (const effect of mock.effects) {
      const cleanup = effect(); cleanup?.(); effect();
    }
    await Promise.resolve();
    expect(mock.invoke).not.toHaveBeenCalled();
    expect(mock.pick).not.toHaveBeenCalled();
    expect(html).toContain('disabled="">Inspect original</button>');
    expect(html).toContain('disabled="">Compare proposals</button>');
  });
  it.each([undefined, "unsupported-network", "https://example.invalid"])("uses the book's wallet evidence for unsupported network %s", network => {
    mock.wallets = [{ environments: ["regtest"] }, { environments: [] }];
    expect(render(network)).toContain("Network: regtest");
    expect(mock.invoke).not.toHaveBeenCalled();
  });
});

it("uses the bound book domain instead of an unrelated device default", () => {
  mock.binding = { environment_id: "book-main", domains: [{chain:"bitcoin",network:"main"}] };
  expect(render()).toContain("Network: main");
  expect(mock.invoke).not.toHaveBeenCalled();
});
it("rejects a handoff from another network instead of reinterpreting it", () => {
  mock.binding = { environment_id: "book-main", domains: [{chain:"bitcoin",network:"main"}] };
  const html = render("regtest");
  expect(html).not.toContain("Inspect original");
  expect(html).toContain("role=\"alert\"");
  expect(mock.invoke).not.toHaveBeenCalled();
});

describe("unbound book evidence", () => {
  it("uses the one environment the wallets show, else mainnet", () => {
    expect(bookEvidenceNetwork([{ environments: ["signet"] }, { environments: ["signet"] }])).toBe("signet");
    expect(bookEvidenceNetwork([{ environments: ["regtest"] }, { environments: ["main"] }])).toBe("main");
    expect(bookEvidenceNetwork([{ environments: [] }])).toBe("main");
    expect(bookEvidenceNetwork(undefined)).toBe("main");
    expect(bookEvidenceNetwork([{ environments: ["liquid-weird"] }])).toBe("main");
  });
});
