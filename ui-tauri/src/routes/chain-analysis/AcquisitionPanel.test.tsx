import { isValidElement, type ReactNode, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it, vi } from "vitest";
import { AcquisitionPanel } from "./InvestigationPanels";
import { DEFAULT_ANALYSIS_QUERY } from "@/lib/chainAnalysis";

type Props = { children?: ReactNode; [key: string]: unknown };
const hooks = vi.hoisted(() => ({
  states: [] as unknown[], cursor: 0, invoke: vi.fn(), queries: vi.fn(), retry: vi.fn(), loading: false, error: false, optionsError: false,
  book: { profile_id: "p", state: "bound", environment_id: "book-1", revision: 1, chain_instance_id: "instance-1", domains: [{ chain: "bitcoin", network: "regtest" }, { chain: "liquid", network: "elementsregtest" }] },
  backends: [
    { name: "local", kind: "bitcoinrpc", chain: "bitcoin", network: "regtest", chain_instance_id: "instance-1" },
    { name: "production", kind: "esplora", chain: "bitcoin", network: "main" },
    { name: "different-instance", kind: "bitcoinrpc", chain: "bitcoin", network: "regtest", chain_instance_id: "instance-2" },
    { name: "liquid", kind: "liquid-esplora", chain: "liquid", network: "elementsregtest" },
  ],
}));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useState: (initial: unknown) => {
    const position = hooks.cursor++;
    if (!(position in hooks.states)) hooks.states[position] = typeof initial === "function" ? initial() : initial;
    return [hooks.states[position], (next: unknown) => { hooks.states[position] = typeof next === "function" ? next(hooks.states[position]) : next; }];
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@tanstack/react-router", () => ({ Link: ({ children, to }: { children: ReactNode; to: string }) => <a href={to}>{children}</a> }));
vi.mock("@/daemon/client", async original => ({
  ...await original<typeof import("@/daemon/client")>(),
  useDaemon: (kind: string) => {
    hooks.queries(kind);
    return kind === "ui.networks.binding"
      ? { data: hooks.loading ? undefined : { data: hooks.book }, isLoading: hooks.loading, isError: hooks.error, error: new Error("binding failed"), refetch: hooks.retry }
      : { data: { data: { backends: hooks.backends } }, isError: hooks.optionsError, error: new Error("options failed"), isLoading: false, isFetching: false };
  },
  useDaemonMutation: (kind: string) => ({ mutateAsync: (args: unknown) => hooks.invoke(kind, args), isPending: false }),
}));
const props = { query: { ...DEFAULT_ANALYSIS_QUERY, subject: "a".repeat(64) }, onError: vi.fn(), onAcquired: vi.fn() };
function wrapper() { return AcquisitionPanel(props); }
function render(): ReactNode {
  hooks.cursor = 0;
  const root = wrapper();
  return typeof root.type === "function" ? (root.type as (value: unknown) => ReactNode)(root.props) : root;
}
function find(node: ReactNode, predicate: (item: ReactElement<Props>) => boolean): ReactElement<Props> | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, predicate)).find(Boolean);
  if (!isValidElement<Props>(node)) return;
  return predicate(node) ? node : find(node.props.children, predicate);
}
function change(tree: ReactNode, label: string, value: string) {
  const field = find(tree, item => item.type === "label" && Array.isArray(item.props.children) && item.props.children[0] === label)!;
  const control = find(field, item => item.type === "select" || item.type === "input")!;
  (control.props.onChange as (event: unknown) => void)({ target: { value } });
}
async function submit(tree: ReactNode) {
  await (find(tree, item => item.type === "form")!.props.onSubmit as (event: unknown) => Promise<void>)({ preventDefault() {} });
}
function button(tree: ReactNode, label: string) { return find(tree, item => item.props.children === label && typeof item.props.onClick === "function"); }
const html = () => renderToStaticMarkup(<>{render()}</>);
beforeEach(() => {
  hooks.states = []; hooks.cursor = 0; hooks.loading = false; hooks.error = false; hooks.optionsError = false;
  hooks.book.state = "bound"; hooks.book.environment_id = "book-1";
  hooks.book.domains = [{ chain: "bitcoin", network: "regtest" }, { chain: "liquid", network: "elementsregtest" }];
  hooks.invoke.mockReset(); hooks.queries.mockReset(); hooks.retry.mockReset();
  hooks.invoke.mockResolvedValue({ data: { plan_id: "plan", backend: { name: "local" }, effects: { max_transactions: 50, max_requests: 106 }, limitations: [] } });
});
it("derives immutable scope and lists compatible backends without a network or standard genesis selector", async () => {
  const output = html();
  expect(output).toContain("instance-1"); expect(output).toContain('href="/settings/bitcoin"');
  expect(output).toContain('<option value="local"');
  expect(output).not.toContain("production"); expect(output).not.toContain("different-instance"); expect(output).not.toContain("acquire.genesis");
  expect(find(render(), item => item.type === "select" && item.props.value === "regtest")).toBeUndefined();
  change(render(), "acquire.backend", "local"); await submit(render());
  expect(hooks.invoke).toHaveBeenCalledWith("ui.chain_analysis.acquire.plan", expect.objectContaining({ chain: "bitcoin", network: "regtest", backend: "local" }));
  expect(hooks.invoke.mock.calls[0][1]).not.toHaveProperty("genesis_hash");
  expect(hooks.queries).not.toHaveBeenCalledWith("ui.networks.inventory");
});
it.each(["unbound", "error", "loading"])("blocks acquisition when binding is %s", state => {
  hooks.book.state = state === "unbound" ? "unbound" : "bound"; hooks.error = state === "error"; hooks.loading = state === "loading";
  const output = html(); expect(output).not.toContain("acquire.preview"); expect(hooks.queries).not.toHaveBeenCalledWith("ui.backends.options");
  if (state !== "loading") expect(output).toContain('href="/settings/bitcoin"');
  if (state === "error") { (button(render(), "common:actions.retry")!.props.onClick as () => void)(); expect(hooks.retry).toHaveBeenCalledOnce(); }
  expect(hooks.invoke).not.toHaveBeenCalled();
});
it("preserves required custom Liquid pins and offers only bound chains", async () => {
  change(render(), "chain", "liquid"); change(render(), "acquire.backend", "liquid");
  expect(html()).toContain("acquire.genesis"); await submit(render()); expect(hooks.invoke).not.toHaveBeenCalled();
  change(render(), "acquire.genesis", "f".repeat(64)); await submit(render());
  expect(hooks.invoke).toHaveBeenCalledWith("ui.chain_analysis.acquire.plan", expect.objectContaining({ chain: "liquid", network: "regtest", genesis_hash: "f".repeat(64) }));
  hooks.states = []; hooks.book.domains = [{ chain: "bitcoin", network: "signet" }];
  expect(html()).not.toContain('<option value="liquid"');
});
it("blocks incompatible selection and stale approval after options or chain change", async () => {
  change(render(), "acquire.backend", "production"); await submit(render()); expect(hooks.invoke).not.toHaveBeenCalled();
  change(render(), "acquire.backend", "local"); await submit(render());
  expect(button(render(), "acquire.apply")!.props.disabled).toBe(false);
  hooks.optionsError = true; expect(button(render(), "acquire.apply")!.props.disabled).toBe(true);
  await (button(render(), "acquire.apply")!.props.onClick as () => Promise<void>)(); expect(hooks.invoke).toHaveBeenCalledOnce();
  hooks.optionsError = false; change(render(), "chain", "liquid"); expect(button(render(), "acquire.apply")!.props.disabled).toBe(true);
});
it("keys form state to the bound book", () => {
  const before = wrapper().key; hooks.book.environment_id = "book-2"; expect(wrapper().key).not.toBe(before);
});
it("links to backend settings when none match the bound domain", async () => {
  hooks.book.domains = [{ chain: "bitcoin", network: "signet" }];
  expect(html()).toContain("acquire.none");
  expect(html()).toContain('href="/settings/bitcoin">acquire.backend</a>');
  expect(find(render(), item => item.props.type === "submit")!.props.disabled).toBe(true);
  await submit(render()); expect(hooks.invoke).not.toHaveBeenCalled();
});
