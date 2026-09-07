import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it, vi } from "vitest";
import "@/i18n";
import { SourceAcquisitionPanel } from "./SourceAcquisitionPanel";
import type { ReactNode } from "react";
vi.mock("@tanstack/react-router", () => ({ Link: ({ to, children }: { to: string; children: ReactNode }) => <a href={to}>{children}</a> }));

const mock = vi.hoisted(() => ({ invoke: vi.fn(), bound: true }));
vi.mock("@/daemon/client", async original => ({
  ...await original<typeof import("@/daemon/client")>(),
  useDaemon: (kind: string) => ({ data: { data: kind === "ui.backends.options" ? { backends: [
    { name: "own-main", kind: "bitcoinrpc", network: "main" },
    { name: "own-lab", kind: "bitcoinrpc", network: "regtest" },
    { name: "explorer", kind: "esplora", network: "regtest" },
  ] } : kind === "ui.networks.binding" ? { state: mock.bound ? "bound" : "unbound", domains: mock.bound ? [{ chain: "bitcoin", network: "regtest", chain_instance_id: "lab-a" }] : [] } : { items: [] } }, refetch: vi.fn() }),
  useDaemonMutation: () => ({ mutateAsync: mock.invoke, isPending: false }),
}));
beforeEach(() => { mock.invoke.mockClear(); mock.bound = true; });

it("offers only the bound book's Core connections and performs no acquisition on render", () => {
  const html = renderToStaticMarkup(<SourceAcquisitionPanel onError={() => {}} />);
  expect(html).toContain('value="own-lab"');
  expect(html).not.toContain('value="own-main"');
  expect(html).not.toContain('value="explorer"');
  expect(html).not.toContain("Allow recurring acquisition");
  expect(html).toContain("Review permission");
  expect(mock.invoke).not.toHaveBeenCalled();
});

it("does not silently choose a network for an unbound book", () => {
  mock.bound = false;
  const html = renderToStaticMarkup(<SourceAcquisitionPanel onError={() => {}} />);
  expect(html).not.toContain('value="own-lab"');
  expect(html).not.toContain('value="own-main"');
  expect(html).toContain('href="/settings/bitcoin"');
  expect(html).not.toContain("Review permission");
  expect(mock.invoke).not.toHaveBeenCalled();
});
