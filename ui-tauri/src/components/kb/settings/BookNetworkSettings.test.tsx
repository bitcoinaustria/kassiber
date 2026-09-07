import { isValidElement, type ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { BookNetworkSettings } from "./BookNetworkSettings";

const hooks = vi.hoisted(() => ({ states: [] as unknown[], cursor: 0, invoke: vi.fn(), inventory: { profile_id:"p",inventory_digest:"before",wallets:[],binding:{state:"unbound",domains:[]} } }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useState: (initial: unknown) => {
    const index = hooks.cursor++;
    if (!(index in hooks.states)) hooks.states[index] = initial;
    return [hooks.states[index], (next: unknown) => { hooks.states[index] = typeof next === "function" ? next(hooks.states[index]) : next; }];
  },
}));
vi.mock("react-i18next", () => ({useTranslation: () => ({t:(key:string) => key})}));
vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({data:{data:hooks.inventory}}),
  useDaemonMutation: (kind:string) => ({mutateAsync:(args:unknown) => hooks.invoke(kind,args),isPending:false}),
}));
function button(node:ReactNode,label:string):{onClick:()=>void;disabled?:boolean}|undefined {
  if (Array.isArray(node)) return node.map(child=>button(child,label)).find(Boolean);
  if (!isValidElement<{children?:ReactNode;onClick?:()=>void;disabled?:boolean}>(node)) return;
  if (node.props.onClick && node.props.children===label) return {onClick:node.props.onClick,disabled:node.props.disabled};
  return button(node.props.children,label);
}
function render() { hooks.cursor=0; return BookNetworkSettings(); }
beforeEach(() => { hooks.states=[];hooks.cursor=0;hooks.inventory={profile_id:"p",inventory_digest:"before",wallets:[],binding:{state:"unbound",domains:[]}};hooks.invoke.mockReset(); });
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
