import { isValidElement, type ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { BookNetworkSettings } from "./BookNetworkSettings";
import { NetworkPartitionSettings } from "./NetworkPartitionSettings";
import enSettings from "@/i18n/locales/en/settings.json";
import deSettings from "@/i18n/locales/de/settings.json";

const locale = vi.hoisted(() => ({ translate: undefined as undefined | ((key: string) => string) }));

const hooks = vi.hoisted(() => ({ states: [] as unknown[], cursor: 0, invoke: vi.fn(), queries: vi.fn(), retryBinding: vi.fn(), retryInventory: vi.fn(), bindingLoading: false, bindingError: false, inventoryError: false, binding: { profile_id:"p", state:"unbound", environment:"main", domains:[{chain:"bitcoin",network:"main"}] }, inventory: { profile_id:"p",inventory_digest:"before",wallets:[],binding:{state:"unbound",domains:[]} } }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useState: (initial: unknown) => {
    const index = hooks.cursor++;
    if (!(index in hooks.states)) hooks.states[index] = initial;
    return [hooks.states[index], (next: unknown) => { hooks.states[index] = typeof next === "function" ? next(hooks.states[index]) : next; }];
  },
}));
vi.mock("react-i18next", () => ({useTranslation: () => ({t:(key:string) => locale.translate?.(key) ?? key})}));
vi.mock("@/daemon/client", () => ({
  useDaemon: (kind:string, _args:unknown, options?:{enabled?:boolean}) => {
    hooks.queries(kind, options);
    return kind === "ui.networks.binding"
      ? {data:hooks.bindingLoading ? undefined : {data:hooks.binding},isLoading:hooks.bindingLoading,isError:hooks.bindingError,refetch:hooks.retryBinding}
      : {data:{data:hooks.inventory},isError:hooks.inventoryError,refetch:hooks.retryInventory};
  },
  useDaemonMutation: (kind:string) => ({mutateAsync:(args:unknown) => hooks.invoke(kind,args),isPending:false}),
}));
function button(node:ReactNode,label:string):{onClick:()=>void;disabled?:boolean}|undefined {
  if (Array.isArray(node)) return node.map(child=>button(child,label)).find(Boolean);
  if (!isValidElement<{children?:ReactNode;onClick?:()=>void;disabled?:boolean}>(node)) return;
  if (node.props.onClick && node.props.children===label) return {onClick:node.props.onClick,disabled:node.props.disabled};
  return button(node.props.children,label);
}
function render() { hooks.cursor=0; return BookNetworkSettings(); }
beforeEach(() => { hooks.states=[];hooks.cursor=0;hooks.inventory={profile_id:"p",inventory_digest:"before",wallets:[],binding:{state:"unbound",domains:[]}};hooks.invoke.mockReset();hooks.queries.mockReset();hooks.retryBinding.mockReset();hooks.retryInventory.mockReset();hooks.bindingLoading=false;hooks.bindingError=false;hooks.inventoryError=false;hooks.binding={profile_id:"p",state:"unbound",environment:"main",domains:[{chain:"bitcoin",network:"main"}]}; });
beforeEach(() => { locale.translate = undefined; });
it.each([["en", enSettings], ["de", deSettings]] as const)("explains the accounting partition blocker in %s and hides export", async (_lang, messages) => {
  locale.translate = key => {
    let value: unknown = messages;
    for (const part of key.split(".")) value = typeof value === "object" && value !== null ? (value as Record<string, unknown>)[part] : undefined;
    return typeof value === "string" ? value : key;
  };
  hooks.states = [["wallet"]];
  hooks.invoke.mockResolvedValue({ data: { profile_id: "p", inventory_digest: "before", plan_id: "blocked", can_apply: false,
    blockers: [{ code: "accounting_partition_unsupported" }], counts: { wallets: 1 } } });
  const renderPartition = () => {
    hooks.cursor = 0;
    return NetworkPartitionSettings({ profileId: "p", inventoryDigest: "before", environment: "main", instance: "",
      wallets: [{ wallet_id: "wallet", label: "Wallet" }], declared: [] });
  };
  button(renderPartition(), messages.bookNetwork.preview)!.onClick();
  await vi.waitFor(() => expect(hooks.invoke).toHaveBeenCalledOnce());
  const tree = renderPartition();
  expect(JSON.stringify(tree)).toContain(messages.bookNetwork.partition.blockers.accounting_partition_unsupported);
  expect(JSON.stringify(tree)).not.toContain("bookNetwork.partition.blockers.accounting_partition_unsupported");
  expect(button(tree, messages.bookNetwork.partition.export)).toBeUndefined();
});
it("requires a current preview and invalidates it when source history changes", async () => {
  hooks.invoke.mockResolvedValue({data:{profile_id:"p",inventory_digest:"before",plan_id:"reviewed",environment:"main",chain_instance_id:null,declared_wallet_ids:[],can_apply:true,blockers:[]}});
  let tree=render();
  expect(button(tree,"bookNetwork.bind")).toBeUndefined();
  button(tree,"bookNetwork.preview")!.onClick();
  await vi.waitFor(()=>expect(hooks.invoke).toHaveBeenCalledOnce());
  tree=render();
  expect(button(tree,"bookNetwork.bind")).toBeDefined();
  hooks.inventory.inventory_digest="after";
  expect(button(render(),"bookNetwork.bind")).toBeUndefined();
});
it("does not confirm a preview in another active book", async () => {
  hooks.invoke.mockResolvedValue({data:{profile_id:"p",inventory_digest:"before",plan_id:"reviewed",environment:"main",chain_instance_id:null,declared_wallet_ids:[],can_apply:true,blockers:[]}});
  button(render(),"bookNetwork.preview")!.onClick();
  await vi.waitFor(()=>expect(hooks.invoke).toHaveBeenCalledOnce());
  hooks.inventory.profile_id="other";
  expect(button(render(),"bookNetwork.bind")).toBeUndefined();
});

it("renders immutable binding without enabling the full inventory query", () => {
  hooks.binding.state="bound";
  const tree=render();
  expect(hooks.queries).toHaveBeenCalledWith("ui.networks.binding", undefined);
  expect(hooks.queries).toHaveBeenCalledWith("ui.networks.inventory", {enabled:false});
  expect(button(tree,"bookNetwork.preview")).toBeUndefined();
  expect(JSON.stringify(tree)).toContain("bookNetwork.immutable");
  expect(JSON.stringify(tree)).toContain("bitcoin: main");
});
it("waits for binding and retries its error before loading inventory", () => {
  hooks.bindingLoading=true;
  expect(button(render(),"bookNetwork.preview")).toBeUndefined();
  expect(hooks.queries).toHaveBeenLastCalledWith("ui.networks.inventory", {enabled:false});
  hooks.bindingLoading=false;hooks.bindingError=true;
  button(render(),"common:actions.retry")!.onClick();
  expect(hooks.retryBinding).toHaveBeenCalledOnce();
  expect(hooks.retryInventory).not.toHaveBeenCalled();
});
it("enables inventory for unbound books and retains its retry action", () => {
  hooks.inventoryError=true;
  const tree=render();
  expect(hooks.queries).toHaveBeenLastCalledWith("ui.networks.inventory", {enabled:true});
  button(tree,"common:actions.retry")!.onClick();
  expect(hooks.retryInventory).toHaveBeenCalledOnce();
});

it("hides a saved preview if either scope response now reports the book bound", async () => {
  hooks.invoke.mockResolvedValue({data:{profile_id:"p",inventory_digest:"before",plan_id:"reviewed",environment:"main",chain_instance_id:null,declared_wallet_ids:[],can_apply:true,blockers:[]}});
  button(render(),"bookNetwork.preview")!.onClick();
  await vi.waitFor(()=>expect(hooks.invoke).toHaveBeenCalledOnce());
  expect(button(render(),"bookNetwork.bind")).toBeDefined();
  hooks.inventory.binding.state="bound";
  expect(button(render(),"bookNetwork.bind")).toBeUndefined();
  hooks.inventory.binding.state="unbound";
  hooks.binding.state="bound";
  expect(button(render(),"bookNetwork.bind")).toBeUndefined();
});
