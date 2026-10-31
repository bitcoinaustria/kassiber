import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

const daemon = vi.hoisted(() => ({
  access: {
    mcp_enabled: false,
    ai_features_enabled: true as boolean | null,
    mcp_available: false,
    reason: "mcp_disabled" as string | null,
    grant: "read" as "read" | "operator" | "accounting_decisions",
    agent_access: "restricted" as "full" | "restricted",
    access_recorded: false,
    session: undefined as
      | {
          needed: boolean;
          active: boolean;
          expires_at: string | null;
          agents?: {
            id: string;
            label: string | null;
            pid: number;
            state: "pending" | "allowed" | "denied";
            calls: number;
            last_call_at: string | null;
          }[];
        }
      | undefined,
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
const tag = (html: string, pattern: string) =>
  html.match(new RegExp(`<button[^>]*${pattern}[^>]*>`))?.[0] ?? "";
const agentsSwitch = (html: string) => tag(html, 'id="settings-external-agents"');
const grantShown = (html: string) => html.match(/data-agent-grant="([^"]+)"/)?.[1] ?? null;
const restrictBox = (html: string) => tag(html, 'id="settings-external-agents-restrict"');
const base = {
  mcp_available: true,
  reason: null,
  grant: "read" as const,
  agent_access: "restricted" as const,
  access_recorded: false,
  session: undefined,
};

describe("ExternalAgentsSettings", () => {
  beforeEach(() => {
    daemon.access = {
      mcp_enabled: false,
      ai_features_enabled: true,
      mcp_available: false,
      reason: "mcp_disabled",
      grant: "read",
      agent_access: "restricted",
      access_recorded: false,
      session: undefined,
    };
  });

  it("is off by default and shows no command", () => {
    const html = render(true);
    expect(html).toContain("External agents");
    expect(html).toContain("Allow agents");
    expect(agentsSwitch(html)).toContain('aria-checked="false"');
    expect(html).not.toContain("claude mcp add");
  });

  it("preselects accounting decisions with full access for a first enable", () => {
    const html = render(true);
    expect(html).toContain("What agents may do");
    expect(grantShown(html)).toBe("accounting_decisions");
    expect(html).toContain("make accounting decisions: resolve quarantine");
    // Full access: the restrict box starts unchecked.
    expect(restrictBox(html)).toContain('aria-checked="false"');
    expect(html).toContain("only prompt before a change");
    expect(html).toContain("never available to agents");
  });

  it("keeps a recorded choice instead of the recommendation", () => {
    daemon.access = { ...daemon.access, grant: "read", agent_access: "restricted", access_recorded: true };
    const html = render(true);
    expect(grantShown(html)).toBe("read");
    expect(restrictBox(html)).toContain('aria-checked="true"');
    expect(html).toContain("They change nothing and contact no service.");
  });

  it("shows the current grant once enabled", () => {
    daemon.access = {
      ...base,
      mcp_enabled: true,
      ai_features_enabled: true,
      grant: "operator",
      agent_access: "full",
      access_recorded: true,
    };
    const html = render(true);
    expect(agentsSwitch(html)).toContain('aria-checked="true"');
    expect(grantShown(html)).toBe("operator");
    expect(html).toContain("contacts your configured backends");
  });

  it("shows the legacy read-only grant of an earlier enable", () => {
    // A preference from before grants: read + restricted, not the recommendation.
    daemon.access = { ...base, mcp_enabled: true, ai_features_enabled: true };
    const html = render(true);
    expect(grantShown(html)).toBe("read");
    expect(restrictBox(html)).toContain('aria-checked="true"');
  });

  it("is disabled and off while AI features are off, even if enabled before", () => {
    daemon.access = {
      ...base,
      mcp_enabled: true,
      ai_features_enabled: false,
      mcp_available: false,
      reason: "ai_features_disabled",
      session: { needed: true, active: false, expires_at: null },
    };
    const html = render(false);
    const toggle = agentsSwitch(html);
    expect(toggle).toContain('aria-checked="false"');
    expect(toggle).toContain('disabled=""');
    expect(html).not.toContain("claude mcp add");
    expect(html).not.toContain("What agents may do");
  });

  it("shows the book-pinned command once enabled", () => {
    daemon.access = { ...base, mcp_enabled: true, ai_features_enabled: true };
    const html = render(true);
    expect(agentsSwitch(html)).toContain('aria-checked="true"');
    expect(html).toContain(
      "claude mcp add kassiber -- kassiber --data-root /data/books mcp serve --workspace Personal --profile Main",
    );
  });

  it("offers to unlock an encrypted book agents cannot open on their own", () => {
    daemon.access = {
      ...base,
      mcp_enabled: true,
      ai_features_enabled: true,
      session: { needed: true, active: false, expires_at: null },
    };
    const html = render(true);
    expect(html).toContain("Agents can use it only while you unlock it for them.");
    expect(html).toContain("Unlock for agents");
  });

  it("offers to lock while agents can read the book", () => {
    daemon.access = {
      ...base,
      mcp_enabled: true,
      ai_features_enabled: true,
      session: { needed: true, active: true, expires_at: "2026-09-28T18:00:00Z" },
    };
    const html = render(true);
    expect(html).toContain("Agents you allow can use this book until you lock Kassiber");
    expect(html).toContain("after 15 minutes without calls");
    expect(html).toContain(">Lock<");
    expect(html).not.toContain("Unlock for agents");
    expect(html).toContain("Start your agent; it appears here for you to allow.");
  });

  it("keeps an unconfirmed lease visible after agents are turned off", () => {
    daemon.access = {
      ...base,
      mcp_enabled: false,
      ai_features_enabled: true,
      mcp_available: false,
      reason: "mcp_disabled",
      session: { needed: true, active: true, expires_at: "2026-09-28T18:00:00Z" },
    };
    const html = render(true);
    expect(agentsSwitch(html)).toContain('aria-checked="false"');
    expect(html).toContain(">Lock<");
  });

  it("shows no unlock for books agents can already open", () => {
    daemon.access = {
      ...base,
      mcp_enabled: true,
      ai_features_enabled: true,
      session: { needed: false, active: false, expires_at: null },
    };
    expect(render(true)).not.toContain("Unlock for agents");
  });

  it("asks to allow a waiting agent and shows what allowed ones did", () => {
    daemon.access = {
      ...base,
      mcp_enabled: true,
      ai_features_enabled: true,
      session: {
        needed: true,
        active: true,
        expires_at: "2026-09-28T18:00:00Z",
        agents: [
          { id: "s1", label: "claude", pid: 41, state: "pending", calls: 0, last_call_at: null },
          { id: "s2", label: "codex", pid: 42, state: "allowed", calls: 3, last_call_at: null },
          { id: "s3", label: "stranger", pid: 43, state: "denied", calls: 0, last_call_at: null },
        ],
      },
    };
    const html = render(true);
    expect(html).toContain("claude (41) wants to use this book");
    expect(html).toContain(">Allow<");
    expect(html).toContain(">Deny<");
    expect(html).toContain("codex (42) can use this book · 3 calls");
    expect(html).toContain(">Remove<");
    expect(html).not.toContain("stranger");
    expect(html).not.toContain("Start your agent");
  });
});
