import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import { DaemonScopeContext } from "@/daemon/client";
import { PsbtPanel } from "./PsbtPanel";

const mock = vi.hoisted(() => ({ invoke: vi.fn(), pick: vi.fn(), effects: [] as Array<() => void | (() => void)> }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useEffect: (effect: () => void | (() => void)) => { mock.effects.push(effect); },
}));
vi.mock("@/daemon/client", async original => ({
  ...await original<typeof import("@/daemon/client")>(),
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
beforeEach(() => { mock.invoke.mockReset(); mock.pick.mockReset(); mock.effects.length = 0; });

describe("PSBT workbench entry", () => {
  it.each(["main", "test", "signet", "regtest"])("preserves the explicit %s network without starting work", async network => {
    const html = render(network);
    expect(html).toContain(`<option selected="">${network}</option>`);
    expect(html.match(/<option selected="">/g)).toHaveLength(1);
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
  it.each([undefined, "unsupported-network", "https://example.invalid"])("uses the local default for unsupported network %s", network => {
    expect(render(network)).toContain('<option selected="">main</option>');
    expect(mock.invoke).not.toHaveBeenCalled();
  });
});
