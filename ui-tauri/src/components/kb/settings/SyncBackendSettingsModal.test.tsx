import { isValidElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SyncBackendSettingsModal } from "./SyncBackendSettingsModal";
import { backendPayload, type Backend } from "./SettingsModel";

const hooks = vi.hoisted(() => ({
  states: [] as unknown[], cursor: 0,
  effects: [] as Array<() => void>,
  invoke: vi.fn(),
  binding: {state:"unbound", domains:[] as {chain:string;network:string}[]},
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
  useDaemon: () => ({data:{data:hooks.binding}}),
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

/** Every string child and element id in the rendered tree. */
function texts(node: ReactNode, out: string[] = []): string[] {
  if (typeof node === "string") out.push(node);
  else if (Array.isArray(node)) node.forEach(child => texts(child, out));
  else if (isValidElement<{ children?: ReactNode; id?: string }>(node)) {
    if (node.props.id) out.push(`#${node.props.id}`);
    texts(node.props.children, out);
  }
  return out;
}

function mount(initial?: Backend, initialTypeId?: "coreln") {
  const save = vi.fn().mockResolvedValue(undefined);
  const props = { open: true, initial: initial ?? null, initialTypeId, onClose: vi.fn(), onSave: save };
  const render = () => { hooks.cursor = 0; return SyncBackendSettingsModal(props); };
  render();
  hooks.effects.splice(0).forEach(effect => effect());
  return { tree: render(), save };
}

beforeEach(() => {
  hooks.binding = {state:"unbound",domains:[]};
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

it("blocks public mainnet presets when adding a backend to a regtest book", () => {
  hooks.binding = {state:"bound",domains:[{chain:"bitcoin",network:"regtest"}]};
  const {tree} = mount();
  expect(button(tree,"backendModal.testConnection")?.disabled).toBe(true);
  expect(button(tree,"backendModal.connectAndSave")?.disabled).toBe(true);
  expect(hooks.invoke).not.toHaveBeenCalled();
});

describe("Core Lightning commando", () => {
  // lightning-cli takes a commando rune only on its command line, so the core
  // refuses commando; the form offers local RPC only.
  const commando: Backend = {
    id: "cln", name: "Remote node", health: "", on: true, auth: "apikey",
    chain: "bitcoin", network: "main", net: "LN", kind: "coreln",
    url: "cln://commando", commandoPeerId: "Configured peer",
  };

  it("offers no rune or commando peer field for a new backend", () => {
    const { tree } = mount(undefined, "coreln");
    const rendered = texts(tree);
    expect(rendered).toContain("#backend-lightning-cli");
    expect(rendered).toContain("#backend-rpc-file");
    expect(rendered).not.toContain("#backend-commando-peer");
    expect(rendered).not.toContain("#backend-commando-rune");
    expect(rendered).not.toContain("backendModal.corelnCommandoUnsupported");
  });

  it("saves a new backend as local RPC with no secret", async () => {
    const { tree, save } = mount({
      id: "cln", name: "Node", health: "", on: true, auth: "none",
      chain: "bitcoin", network: "main", net: "LN", kind: "coreln",
      url: "", lightningCli: "/usr/local/bin/cln-wrapper",
    });
    const connect = button(tree, "backendModal.saveBackend")!;
    expect(connect.disabled).toBe(false);
    connect.onClick();
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(save).toHaveBeenCalledWith(expect.objectContaining({
      url: "cln://local", auth: "none", token: undefined,
      lightningCli: "/usr/local/bin/cln-wrapper",
    }));
    expect(hooks.invoke).not.toHaveBeenCalled();
  });

  it("shows an existing commando backend as unsupported and cannot save it unchanged", () => {
    const { tree, save } = mount(commando);
    const rendered = texts(tree);
    expect(rendered).toContain("backendModal.corelnCommandoUnsupported");
    expect(rendered).toContain("backendModal.corelnCommandoHint");
    expect(rendered).not.toContain("#backend-commando-rune");
    expect(button(tree, "backendModal.saveBackend")?.disabled).toBe(true);
    expect(save).not.toHaveBeenCalled();
  });

  it("converts an existing commando backend to local RPC and clears the rune and peer", async () => {
    const { tree, save } = mount({ ...commando, rpcFile: "Configured RPC file" });
    const connect = button(tree, "backendModal.saveBackend")!;
    expect(connect.disabled).toBe(false);
    connect.onClick();
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    const saved = save.mock.calls[0][0] as Backend;
    expect(saved).toMatchObject({ url: "cln://local", auth: "none" });
    expect(saved.token).toBeUndefined();
    expect(saved.commandoPeerId).toBeUndefined();
    const payload = backendPayload(saved);
    expect(payload).not.toHaveProperty("token");
    expect(payload.config).not.toHaveProperty("commando_peer_id");
    expect(payload.clear).toEqual(expect.arrayContaining(["token", "commando_peer_id"]));
  });
});
