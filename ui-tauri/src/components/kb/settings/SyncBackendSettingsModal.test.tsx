import { isValidElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SyncBackendSettingsModal } from "./SyncBackendSettingsModal";
import type { Backend } from "./SettingsModel";

const hooks = vi.hoisted(() => ({
  states: [] as unknown[], cursor: 0,
  effects: [] as Array<() => void>,
  invoke: vi.fn(),
}));
// Replay the modal's mount effects and inspect its real button callbacks without
// a DOM. Only React scheduling is replaced; test/save payloads run unchanged.
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useState: (initial: unknown) => {
    const index = hooks.cursor++;
    if (!(index in hooks.states)) hooks.states[index] = initial;
    return [hooks.states[index], (next: unknown) => { hooks.states[index] = next; }];
  },
  useMemo: (factory: () => unknown) => factory(),
  useEffect: (effect: () => void) => { hooks.effects.push(effect); },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/daemon/client", () => ({
  useDaemonMutation: (kind: string) => ({
    mutateAsync: (args: unknown) => hooks.invoke(kind, args),
  }),
}));

function button(node: ReactNode, label: string): { onClick: () => void; disabled?: boolean } | undefined {
  if (Array.isArray(node)) return node.map(child => button(child, label)).find(Boolean);
  if (!isValidElement<{ children?: ReactNode; onClick?: () => void; disabled?: boolean }>(node)) return;
  const children = node.props.children;
  if (node.props.onClick && (children === label || (Array.isArray(children) && children.includes(label)))) {
    return { onClick: node.props.onClick, disabled: node.props.disabled };
  }
  return button(children, label);
}

function mount(initial?: Backend) {
  const save = vi.fn().mockResolvedValue(undefined);
  const props = { open: true, initial: initial ?? null, onClose: vi.fn(), onSave: save };
  const render = () => { hooks.cursor = 0; return SyncBackendSettingsModal(props); };
  render();
  hooks.effects.splice(0).forEach(effect => effect());
  return { tree: render(), save };
}

beforeEach(() => {
  hooks.states.length = 0;
  hooks.effects.length = 0;
  hooks.cursor = 0;
  hooks.invoke.mockReset().mockResolvedValue({ data: { reachable: true, ok: true, logs: [] } });
});

describe("backend settings network identity", () => {
  it.each([
    { chain: "bitcoin", network: "regtest", net: "BTC", kind: "bitcoinrpc", url: "http://127.0.0.1:18443" },
    { chain: "bitcoin", network: "test", net: "BTC", kind: "bitcoinrpc", url: "http://127.0.0.1:18332" },
    { chain: "bitcoin", network: "signet", net: "BTC", kind: "bitcoinrpc", url: "http://127.0.0.1:38332" },
    { chain: "liquid", network: "regtest", net: "LIQUID", kind: "electrum", url: "tcp://127.0.0.1:50001" },
    { chain: "liquid", network: "liquidtestnet", net: "LIQUID", kind: "electrum", url: "tcp://127.0.0.1:50001" },
    { chain: "liquid", network: "liquidv1", net: "LIQUID", kind: "electrum", url: "tcp://127.0.0.1:50001" },
  ] as const)("preserves $chain/$network when testing and saving", async identity => {
    const { tree, save } = mount({
      id: "existing", name: "Existing backend", health: "", on: true, auth: "none", ...identity,
    });
    const test = button(tree, "backendModal.testConnection")!;
    expect(test).toBeDefined();
    test.onClick();
    await vi.waitFor(() => expect(hooks.invoke).toHaveBeenCalledTimes(1));
    if (identity.kind === "bitcoinrpc") {
      expect(hooks.invoke).toHaveBeenLastCalledWith("ui.backends.bitcoinrpc.test", expect.objectContaining({
        backend: "existing", network: identity.network,
      }));
    } else {
      // Electrum's transport probe has no network argument; saving must still
      // retain the exact stored network instead of replacing it with liquidv1.
      expect(hooks.invoke.mock.calls[0][0]).toBe("ui.backends.electrum.test");
    }
    const connect = button(tree, "backendModal.saveBackend")!;
    expect(connect.disabled).toBe(false);
    connect.onClick();
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ chain: identity.chain, network: identity.network }));
  });

  it("retains mainnet defaults for a new Bitcoin backend", async () => {
    const { tree, save } = mount();
    button(tree, "backendModal.connectAndSave")!.onClick();
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ chain: "bitcoin", network: "main" }));
  });

  it("preserves a Core Lightning regtest backend without a remote test", async () => {
    const { tree, save } = mount({
      id: "cln", name: "Core Lightning", health: "", on: true, auth: "none",
      chain: "bitcoin", network: "regtest", net: "LN", kind: "coreln",
      url: "cln://local", rpcFile: "/tmp/lightning-rpc",
    });
    const connect = button(tree, "backendModal.saveBackend")!;
    expect(connect.disabled).toBe(false);
    connect.onClick();
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ chain: "bitcoin", network: "regtest" }));
    expect(hooks.invoke).not.toHaveBeenCalled();
  });

  it.each([undefined, null])("keeps legacy defaults for missing identity %s", async missing => {
    const { tree, save } = mount({
      id: "legacy", name: "Legacy", health: "", on: true, auth: "none",
      net: "LIQUID", kind: "electrum", url: "tcp://127.0.0.1:50001",
      chain: missing, network: missing,
    } as Backend);
    button(tree, "backendModal.saveBackend")!.onClick();
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ chain: "liquid", network: "liquidv1" }));
  });
});
