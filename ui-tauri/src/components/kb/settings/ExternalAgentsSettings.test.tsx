import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

const daemon = vi.hoisted(() => ({
  access: {
    mcp_enabled: false,
    ai_features_enabled: true as boolean | null,
    mcp_available: false,
    reason: "mcp_disabled" as string | null,
    session: undefined as { needed: boolean; active: boolean; expires_at: string | null } | undefined,
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
  useDaemonMutation: () => ({ mutate: vi.fn(), isPending: false, isError: false, error: null }),
  DaemonRequestError: class extends Error {},
}));

import { ExternalAgentsSettings } from "./ExternalAgentsSettings";

const render = (aiFeaturesEnabled: boolean) =>
  renderToStaticMarkup(<ExternalAgentsSettings aiFeaturesEnabled={aiFeaturesEnabled} />);

describe("ExternalAgentsSettings", () => {
  beforeEach(() => {
    daemon.access = {
      mcp_enabled: false,
      ai_features_enabled: true,
      mcp_available: false,
      reason: "mcp_disabled",
      session: undefined,
    };
  });

  it("is off by default and shows no command", () => {
    const html = render(true);
    expect(html).toContain("External agents (MCP)");
    expect(html).toContain('aria-checked="false"');
    expect(html).not.toContain("claude mcp add");
  });

  it("is disabled and off while AI features are off, even if enabled before", () => {
    daemon.access = {
      mcp_enabled: true,
      ai_features_enabled: false,
      mcp_available: false,
      reason: "ai_features_disabled",
      session: { needed: true, active: false, expires_at: null },
    };
    const html = render(false);
    expect(html).toContain('aria-checked="false"');
    const toggle = html.match(/<button[^>]*id="settings-external-agents"[^>]*>/)?.[0] ?? "";
    expect(toggle).toContain('disabled=""');
    expect(html).not.toContain("claude mcp add");
  });

  it("shows the book-pinned command once enabled", () => {
    daemon.access = { mcp_enabled: true, ai_features_enabled: true, mcp_available: true, reason: null, session: undefined };
    const html = render(true);
    expect(html).toContain('aria-checked="true"');
    expect(html).toContain(
      "claude mcp add kassiber -- kassiber --data-root /data/books mcp serve --workspace Personal --profile Main",
    );
  });

  it("offers to unlock an encrypted book agents cannot open on their own", () => {
    daemon.access = {
      mcp_enabled: true,
      ai_features_enabled: true,
      mcp_available: true,
      reason: null,
      session: { needed: true, active: false, expires_at: null },
    };
    const html = render(true);
    expect(html).toContain("Agents can read it only while you unlock it for them.");
    expect(html).toContain("Unlock for agents");
  });

  it("offers to lock while agents can read the book", () => {
    daemon.access = {
      mcp_enabled: true,
      ai_features_enabled: true,
      mcp_available: true,
      reason: null,
      session: { needed: true, active: true, expires_at: "2026-09-28T18:00:00Z" },
    };
    const html = render(true);
    expect(html).toContain("Agents can read this book until you lock Kassiber");
    expect(html).toContain(">Lock<");
    expect(html).not.toContain("Unlock for agents");
  });

  it("keeps an unconfirmed lease visible after agents are turned off", () => {
    daemon.access = {
      mcp_enabled: false,
      ai_features_enabled: true,
      mcp_available: false,
      reason: "mcp_disabled",
      session: { needed: true, active: true, expires_at: "2026-09-28T18:00:00Z" },
    };
    const html = render(true);
    expect(html).toContain('aria-checked="false"');
    expect(html).toContain(">Lock<");
  });

  it("shows no unlock for books agents can already open", () => {
    daemon.access = {
      mcp_enabled: true,
      ai_features_enabled: true,
      mcp_available: true,
      reason: null,
      session: { needed: false, active: false, expires_at: null },
    };
    expect(render(true)).not.toContain("Unlock for agents");
  });
});
