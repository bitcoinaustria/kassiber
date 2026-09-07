import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import { DEFAULT_ANALYSIS_QUERY } from "@/lib/chainAnalysis";
import { WatchAction, WatchInbox } from "./WatchControls";
const mock = vi.hoisted(() => ({ states: [] as unknown[], cursor: 0, buttons: [] as {
        children: ReactNode;
        onClick?: () => unknown;
        disabled?: boolean;
    }[], mutate: vi.fn(), data: {} as Record<string, unknown> }));
vi.mock("react", async (original) => ({ ...await original<typeof import("react")>(), useState: (initial: unknown) => {
        const index = mock.cursor++;
        if (!(index in mock.states))
            mock.states[index] = initial;
        return [mock.states[index], (value: unknown) => { mock.states[index] = value; }];
    } }));
vi.mock("@tanstack/react-router", () => ({ Link: ({ children, to }: {
        children: ReactNode;
        to: string;
    }) => <a href={to}>{children}</a> }));
vi.mock("@/components/ui/button", () => ({ Button: (props: typeof mock.buttons[number]) => { mock.buttons.push(props); return <button disabled={props.disabled}>{props.children}</button>; } }));
vi.mock("@/daemon/client", async (original) => ({ ...await original<typeof import("@/daemon/client")>(),
    useDaemonMutation: (kind: string) => ({ isPending: false, mutateAsync: (args: unknown) => mock.mutate(kind, args) }),
    useDaemon: (kind: string) => ({ data: { data: mock.data[kind] }, refetch: vi.fn() }),
}));
const query = { ...DEFAULT_ANALYSIS_QUERY, subject: `bitcoin:main:out:${"a".repeat(64)}:0`, chain: "bitcoin" as const, network: "main", observer: "public" as const };
const error = vi.fn();
function render(element: ReactNode) { mock.cursor = 0; mock.buttons = []; return renderToStaticMarkup(element); }
async function click(label: string) {
    const button = mock.buttons.find(item => item.children === label);
    expect(button, label).toBeDefined();
    expect(button!.disabled).not.toBe(true);
    await button!.onClick?.();
}
beforeEach(() => { mock.states = []; mock.cursor = 0; mock.buttons = []; mock.mutate.mockReset(); mock.data = { "ui.networks.binding": { state: "bound", environment: "main", domains: [{ chain: "bitcoin", network: "main" }, { chain: "liquid", network: "liquidv1" }] } }; error.mockReset(); });
describe("local watch controls", () => {
    it("requires preview then explicit activation with unchanged facts and public observer", async () => {
        const plan = { plan_id: "reviewed", definition: { rule: "output_spent", query }, baseline: { status: "unknown" } };
        mock.mutate.mockResolvedValue({ data: plan });
        const element = <WatchAction output query={query} onError={error}/>;
        render(element);
        await click("Watch");
        render(element);
        await click("Preview");
        expect(mock.mutate).toHaveBeenCalledWith("ui.chain_analysis.watches.preview", { rule: "output_spent", query });
        expect(mock.mutate).not.toHaveBeenCalledWith("ui.chain_analysis.watches.create", expect.anything());
        expect(render(element)).toContain("Baseline: Unknown");
        await click("Start watching");
        expect(mock.mutate).toHaveBeenLastCalledWith("ui.chain_analysis.watches.create", { plan });
        expect(render(element)).toContain("The baseline does not trigger a notification");
    });
    it("discards rejected previews", async () => {
        const element = <WatchAction output query={query} onError={error}/>;
        mock.mutate.mockResolvedValueOnce({ data: { plan_id: "old", baseline: { status: "partial" } } }).mockRejectedValueOnce(new Error("Evidence changed"));
        render(element);
        await click("Watch");
        render(element);
        await click("Preview");
        render(element);
        await click("Start watching");
        expect(error).toHaveBeenCalled();
        expect(render(element)).not.toContain("Start watching");
    });
    it("offers a settings handoff for an unbound book instead of an unusable preview", async () => {
        mock.data["ui.networks.binding"] = { state: "unbound", domains: [] };
        const element = <WatchAction output query={query} onError={error}/>;
        render(element); await click("Watch");
        expect(render(element)).toContain('href="/settings/bitcoin"');
        expect(render(element)).toContain("Set this book’s network");
        expect(mock.buttons.find(button => button.children === "Preview")?.disabled).toBe(true);
        expect(mock.mutate).not.toHaveBeenCalled();
    });
    it("binds saved case identity", async () => {
        const element = <WatchAction caseId="saved" query={query} onError={error}/>;
        mock.mutate.mockResolvedValue({ data: null });
        render(element);
        await click("Watch");
        render(element);
        await click("Preview");
        expect(mock.mutate).toHaveBeenCalledWith("ui.chain_analysis.watches.preview", { rule: "findings_changed", case_id: "saved" });
    });
    it("shows coverage loss and pauses with current revision", async () => {
        mock.data["ui.chain_analysis.watches.list"] = { items: [{ id: "watch", revision: 4, enabled: true, baseline: { status: "partial" }, definition: { rule: "findings_changed", query } }] };
        mock.data["ui.chain_analysis.watches.inbox"] = { unread_count: 1, items: [{ id: "event", watch_id: "watch", code: "coverage_lost", created_at: "2026-01-01", acknowledged_at: null }] };
        mock.mutate.mockResolvedValue({});
        const element = <WatchInbox onOpen={vi.fn()} onError={error}/>;
        expect(render(element)).toContain("Evidence coverage decreased");
        await click("Pause");
        expect(mock.mutate).toHaveBeenCalledWith("ui.chain_analysis.watches.configure", { id: "watch", expected_revision: 4, enabled: false });
    });
});
