import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

const daemon = vi.hoisted(() => ({
  access: {
    mcp_enabled: false,
    ai_features_enabled: true as boolean | null,
    mcp_available: false,
    reason: "mcp_disabled" as string | null,
  },
  status: { data_root: "/data/books", current_workspace: "Personal", current_profile: "Main" },
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string) => ({
    data:
      kind === "ui.agent_access.status"
        ? { kind, data: daemon.access }
        : kind === "status"
          ? { kind, data: daemon.status }
          : undefined,
  }),
  useDaemonMutation: () => ({ mutate: vi.fn(), isPending: false, isError: false }),
}));

import { ExternalAgentsSettings } from "./ExternalAgentsSettings";

const render = (aiFeaturesEnabled: boolean) =>
  renderToStaticMarkup(<ExternalAgentsSettings aiFeaturesEnabled={aiFeaturesEnabled} />);

describe("ExternalAgentsSettings", () => {
  beforeEach(() => {
    daemon.access = { mcp_enabled: false, ai_features_enabled: true, mcp_available: false, reason: "mcp_disabled" };
  });

  it("is off by default and shows no command", () => {
    const html = render(true);
    expect(html).toContain("External agents (MCP)");
    expect(html).toContain('aria-checked="false"');
    expect(html).not.toContain("claude mcp add");
  });

  it("is disabled and off while AI features are off, even if enabled before", () => {
    daemon.access = { mcp_enabled: true, ai_features_enabled: false, mcp_available: false, reason: "ai_features_disabled" };
    const html = render(false);
    expect(html).toContain('aria-checked="false"');
    const toggle = html.match(/<button[^>]*id="settings-external-agents"[^>]*>/)?.[0] ?? "";
    expect(toggle).toContain('disabled=""');
    expect(html).not.toContain("claude mcp add");
  });

  it("shows the book-pinned command once enabled", () => {
    daemon.access = { mcp_enabled: true, ai_features_enabled: true, mcp_available: true, reason: null };
    const html = render(true);
    expect(html).toContain('aria-checked="true"');
    expect(html).toContain(
      "claude mcp add kassiber -- kassiber --data-root /data/books mcp serve --workspace Personal --profile Main",
    );
  });
});
