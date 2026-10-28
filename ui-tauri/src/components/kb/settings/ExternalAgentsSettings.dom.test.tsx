// @vitest-environment happy-dom
//
// The grant chosen before enabling is a draft. Once the core records it, the
// row must show (and a later enable must re-send) the recorded choice, never
// a stale draft.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

type Access = {
  mcp_enabled: boolean;
  ai_features_enabled: boolean | null;
  mcp_available: boolean;
  reason: string | null;
  grant: "read" | "operator" | "accounting_decisions";
  agent_access: "full" | "restricted";
  access_recorded: boolean;
};

const daemon = vi.hoisted(() => ({
  access: {} as Access,
  mutate: vi.fn(),
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string) => ({
    data:
      kind === "ui.agent_access.status"
        ? { kind, data: daemon.access }
        : kind === "status"
          ? { kind, data: { data_root: "/data", current_workspace: "W", current_profile: "B" } }
          : undefined,
  }),
  useDaemonMutation: () => ({ mutate: daemon.mutate, isPending: false, isError: false, error: null }),
  DaemonRequestError: class extends Error {},
}));
vi.mock("@/daemon/transport", () => ({
  terminalCommandStatus: () => new Promise(() => undefined),
}));

import { ExternalAgentsSettings } from "./ExternalAgentsSettings";

const off: Access = {
  mcp_enabled: false,
  ai_features_enabled: true,
  mcp_available: false,
  reason: "mcp_disabled",
  grant: "read",
  agent_access: "restricted",
  access_recorded: false,
};

const checked = (name: string) =>
  screen.getByRole("radio", { name }).getAttribute("aria-checked");

describe("ExternalAgentsSettings grant drafts", () => {
  afterEach(() => {
    cleanup();
    daemon.mutate.mockReset();
  });

  it("sends the chosen draft once, then follows the recorded choice", () => {
    daemon.access = { ...off };
    const view = render(<ExternalAgentsSettings aiFeaturesEnabled />);
    expect(checked("Accounting decisions")).toBe("true");
    fireEvent.click(screen.getByRole("radio", { name: "Read and operate" }));
    // Off: choosing records nothing yet.
    expect(daemon.mutate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("switch", { name: "Allow external agents" }));
    expect(daemon.mutate).toHaveBeenLastCalledWith({
      mcp_enabled: true,
      ai_features_enabled: true,
      grant: "operator",
      agent_access: "full",
    });

    act(() => {
      daemon.access = { ...off, mcp_enabled: true, mcp_available: true, reason: null, grant: "operator", agent_access: "full", access_recorded: true };
      view.rerender(<ExternalAgentsSettings aiFeaturesEnabled />);
    });
    fireEvent.click(screen.getByRole("radio", { name: "Read" }));
    // On: a choice is recorded right away.
    expect(daemon.mutate).toHaveBeenLastCalledWith({ grant: "read" });

    act(() => {
      daemon.access = { ...off, grant: "read", agent_access: "full", access_recorded: true };
      view.rerender(<ExternalAgentsSettings aiFeaturesEnabled />);
    });
    expect(checked("Read")).toBe("true");
    fireEvent.click(screen.getByRole("switch", { name: "Allow external agents" }));
    expect(daemon.mutate).toHaveBeenLastCalledWith({
      mcp_enabled: true,
      ai_features_enabled: true,
      grant: "read",
      agent_access: "full",
    });
  });
});
