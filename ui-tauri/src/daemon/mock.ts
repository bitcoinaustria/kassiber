/**
 * Mock daemon transport.
 *
 * Returns hand-rolled fixtures keyed by `kind`. These fixtures are
 * throwaway — they get regenerated from the Pydantic→JSON Schema pipeline
 * once contracts.py + dump_schema.py land (Phase 1.2 §2.2).
 */

import type {
  DaemonEnvelope,
  DaemonRequest,
  DaemonStreamOptions,
  DaemonStreamRecord,
  DaemonTransport,
} from "./transport";
import { MOCK_PROFILES } from "@/mocks/profiles";
import { MOCK_AI_CHAT_STREAM, fixtures } from "./fixtures";

const SIMULATED_LATENCY_MS = 50;

const cloneMockProfiles = () => ({
  activeWorkspaceId: MOCK_PROFILES.activeWorkspaceId,
  activeProfileId: MOCK_PROFILES.activeProfileId,
  workspaces: MOCK_PROFILES.workspaces.map((workspace) => ({
    ...workspace,
    profiles: workspace.profiles.map((profile) => ({ ...profile })),
  })),
});

let mockProfilesSnapshot = cloneMockProfiles();

type MockConnection = {
  id: string;
  label: string;
};

const mockOverviewSnapshot = () =>
  fixtures["ui.overview.snapshot"] as {
    connections: MockConnection[];
  };

export const mockDaemon: DaemonTransport = {
  async invoke<T = unknown>(
    req: DaemonRequest,
  ): Promise<DaemonEnvelope<T>> {
    await new Promise((resolve) =>
      setTimeout(resolve, SIMULATED_LATENCY_MS),
    );

    if (req.kind === "ai.chat.cancel") {
      return {
        kind: "ai.chat.cancel",
        schema_version: 1,
        request_id: req.request_id,
        data: { cancelled: true } as T,
      };
    }

    if (req.kind === "ai.tool_call.consent") {
      return {
        kind: "ai.tool_call.consent",
        schema_version: 1,
        request_id: req.request_id,
        data: { recorded: true } as T,
      };
    }

    if (req.kind === "daemon.lock") {
      return {
        kind: "daemon.lock",
        schema_version: 1,
        request_id: req.request_id,
        data: { locked: true } as T,
      };
    }

    if (req.kind === "daemon.unlock") {
      return {
        kind: "daemon.unlock",
        schema_version: 1,
        request_id: req.request_id,
        data: { unlocked: true } as T,
      };
    }

    if (req.kind === "ui.secrets.init") {
      return {
        kind: "ui.secrets.init",
        schema_version: 1,
        request_id: req.request_id,
        data: { encrypted: true, already_encrypted: false } as T,
      };
    }

    if (req.kind === "ui.secrets.change_passphrase") {
      return {
        kind: "ui.secrets.change_passphrase",
        schema_version: 1,
        request_id: req.request_id,
        data: { changed: true } as T,
      };
    }

    if (req.kind === "ui.workspace.delete") {
      return {
        kind: "ui.workspace.delete",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          deleted: true,
          workspace: { id: "mock-workspace", label: "My Books" },
          removed: { profiles: 2, wallets: 4, transactions: 24 },
        } as T,
      };
    }

    if (req.kind === "ui.profiles.snapshot") {
      return {
        kind: "ui.profiles.snapshot",
        schema_version: 1,
        request_id: req.request_id,
        data: mockProfilesSnapshot as T,
      };
    }

    if (req.kind === "ui.profiles.switch") {
      const args = (req.args ?? {}) as { profile_id?: unknown };
      const profileId = typeof args.profile_id === "string" ? args.profile_id : "";
      const exists = mockProfilesSnapshot.workspaces.some((workspace) =>
        workspace.profiles.some((profile) => profile.id === profileId),
      );
      if (!exists) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "books not found",
            retryable: false,
          },
        };
      }
      mockProfilesSnapshot = {
        ...mockProfilesSnapshot,
        activeWorkspaceId: mockProfilesSnapshot.workspaces.find((workspace) =>
          workspace.profiles.some((profile) => profile.id === profileId),
        )?.id,
        activeProfileId: profileId,
        workspaces: mockProfilesSnapshot.workspaces.map((workspace) => ({
          ...workspace,
          profiles: workspace.profiles.map((profile) => ({
            ...profile,
            active: profile.id === profileId,
            lastOpened: profile.id === profileId ? "Just now" : profile.lastOpened,
          })),
        })),
      };
      return {
        kind: "ui.profiles.switch",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          activeProfileId: profileId,
          activeWorkspaceId: mockProfilesSnapshot.activeWorkspaceId,
        } as T,
      };
    }

    if (req.kind === "ui.profiles.create") {
      const args = (req.args ?? {}) as {
        workspace_id?: unknown;
        label?: unknown;
        source_profile_id?: unknown;
      };
      const workspaceId =
        typeof args.workspace_id === "string" ? args.workspace_id : "";
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const sourceProfileId =
        typeof args.source_profile_id === "string"
          ? args.source_profile_id
          : "";
      const workspace = mockProfilesSnapshot.workspaces.find(
        (candidate) => candidate.id === workspaceId,
      );
      if (!workspace) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "books set not found",
            retryable: false,
          },
        };
      }
      if (!label) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Books label is required.",
            retryable: false,
          },
        };
      }
      const sourceProfile = sourceProfileId
        ? workspace.profiles.find((candidate) => candidate.id === sourceProfileId)
        : null;
      if (sourceProfileId && !sourceProfile) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "source book not found in this books set",
            retryable: false,
          },
        };
      }
      const firstProfile = workspace.profiles[0];
      const profile = {
        id: `mock-profile-${Date.now()}`,
        name: label,
        role: "Owner" as const,
        taxPolicy:
          sourceProfile?.taxPolicy ??
          firstProfile?.taxPolicy ??
          `${workspace.jurisdiction} defaults`,
        fiatCurrency:
          sourceProfile?.fiatCurrency ??
          firstProfile?.fiatCurrency ??
          workspace.currency,
        taxCountry:
          sourceProfile?.taxCountry ??
          firstProfile?.taxCountry ??
          (workspace.jurisdiction === "Austria" ? "at" : "generic"),
        taxLongTermDays:
          sourceProfile?.taxLongTermDays ??
          firstProfile?.taxLongTermDays ??
          (workspace.jurisdiction === "Austria" ? 0 : 365),
        gainsAlgorithm:
          sourceProfile?.gainsAlgorithm ??
          firstProfile?.gainsAlgorithm ??
          (workspace.jurisdiction === "Austria" ? "MOVING_AVERAGE_AT" : "FIFO"),
        accounts: 1,
        wallets: 0,
        lastOpened: "Just now",
        active: true,
      };
      mockProfilesSnapshot = {
        activeWorkspaceId: workspace.id,
        activeProfileId: profile.id,
        workspaces: mockProfilesSnapshot.workspaces.map((candidate) => ({
          ...candidate,
          profiles:
            candidate.id === workspace.id
              ? [
                  ...candidate.profiles.map((existing) => ({
                    ...existing,
                    active: false,
                  })),
                  profile,
                ]
              : candidate.profiles.map((existing) => ({
                  ...existing,
                  active: false,
                })),
        })),
      };
      return {
        kind: "ui.profiles.create",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          activeProfileId: profile.id,
          activeWorkspaceId: workspace.id,
          profile: { id: profile.id, name: profile.name },
          workspace: { id: workspace.id, name: workspace.name },
        } as T,
      };
    }

    if (req.kind === "ui.workspace.create") {
      const args = (req.args ?? {}) as { label?: unknown };
      const label = typeof args.label === "string" ? args.label.trim() : "";
      if (!label) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Books set name is required.",
            retryable: false,
          },
        };
      }
      const workspace = {
        id: `mock-workspace-${Date.now()}`,
        name: label,
        kind: "Personal" as const,
        currency: "Mixed",
        jurisdiction: "Generic",
        created: new Date().toISOString().slice(0, 10),
        profiles: [],
      };
      mockProfilesSnapshot = {
        activeWorkspaceId: workspace.id,
        activeProfileId: "",
        workspaces: [
          ...mockProfilesSnapshot.workspaces.map((existing) => ({
            ...existing,
            profiles: existing.profiles.map((profile) => ({
              ...profile,
              active: false,
            })),
          })),
          workspace,
        ],
      };
      return {
        kind: "ui.workspace.create",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          workspace: { id: workspace.id, name: workspace.name },
          activeWorkspaceId: workspace.id,
          activeProfileId: "",
        } as T,
      };
    }

    if (req.kind === "ui.wallets.update") {
      const args = (req.args ?? {}) as {
        wallet?: unknown;
        label?: unknown;
      };
      const walletRef = typeof args.wallet === "string" ? args.wallet : "";
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const overview = mockOverviewSnapshot();
      const connection = overview.connections.find(
        (item) => item.id === walletRef || item.label === walletRef,
      );
      if (!connection || !label) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "wallet update requires an existing wallet and label",
            retryable: false,
          },
        };
      }
      connection.label = label;
      return {
        kind: "ui.wallets.update",
        schema_version: 1,
        request_id: req.request_id,
        data: { wallet: connection } as T,
      };
    }

    if (req.kind === "ui.wallets.delete") {
      const args = (req.args ?? {}) as {
        wallet?: unknown;
        confirm_wallet?: unknown;
      };
      const walletRef = typeof args.wallet === "string" ? args.wallet : "";
      const overview = mockOverviewSnapshot();
      const connection = overview.connections.find(
        (item) => item.id === walletRef || item.label === walletRef,
      );
      if (
        !connection ||
        typeof args.confirm_wallet !== "string" ||
        args.confirm_wallet !== connection.label
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "wallet delete requires the exact connection label",
            retryable: false,
          },
        };
      }
      overview.connections = overview.connections.filter(
        (item) => item.id !== connection.id,
      );
      return {
        kind: "ui.wallets.delete",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          wallet: {
            id: connection.id,
            label: connection.label,
            deleted: true,
            cascaded_transactions: 0,
          },
        } as T,
      };
    }

    const fixture = fixtures[req.kind];
    if (fixture === undefined) {
      return {
        kind: "error",
        schema_version: 1,
        error: {
          code: "kind_not_found",
          message: `mock daemon has no fixture for kind="${req.kind}"`,
          retryable: false,
        },
      };
    }

    return {
      kind: req.kind,
      schema_version: 1,
      data: fixture as T,
    };
  },
  async stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>> {
    if (req.kind === "ai.chat") {
      return mockAiChatStream<T, R>(req, options);
    }
    // Non-streaming kinds resolve straight through to invoke.
    return mockDaemon.invoke<T>(req);
  },
};

export const mockStream = mockDaemon.stream;

async function mockAiChatStream<T, R>(
  req: DaemonRequest,
  options?: DaemonStreamOptions<R>,
): Promise<DaemonEnvelope<T>> {
  const requestId = req.request_id ?? `mock-${Math.random().toString(36).slice(2)}`;
  let cancelled = false;
  const args = (req.args ?? {}) as {
    model?: string;
    provider?: string;
    tools_enabled?: boolean;
  };
  options?.onRecord?.({
    kind: "ai.chat.status",
    schema_version: 1,
    request_id: requestId,
    data: { phase: "waiting_for_model", label: "Loading model" } as R,
  });
  if (args.tools_enabled) {
    options?.onRecord?.({
      kind: "ai.chat.tool_call",
      schema_version: 1,
      request_id: requestId,
      data: {
        call_id: "mock-tool-1",
        name: "ui.overview.snapshot",
        arguments: {},
        kind_class: "read_only",
        needs_consent: false,
      } as R,
    });
    await new Promise((resolve) => setTimeout(resolve, 80));
    if (options?.signal?.aborted) {
      cancelled = true;
    } else {
      options?.onRecord?.({
        kind: "ai.chat.tool_result",
        schema_version: 1,
        request_id: requestId,
        data: {
          call_id: "mock-tool-1",
          ok: true,
          envelope: { kind: "ui.overview.snapshot", schema_version: 1, data: fixtures["ui.overview.snapshot"] },
        } as R,
      });
    }
  }
  for (const chunk of MOCK_AI_CHAT_STREAM) {
    if (options?.signal?.aborted) {
      cancelled = true;
      break;
    }
    await new Promise((resolve) =>
      setTimeout(resolve, chunk.delayMs ?? 30),
    );
    const delta: { content?: string; reasoning?: string } = {};
    if (chunk.content !== undefined) delta.content = chunk.content;
    if (chunk.reasoning !== undefined) delta.reasoning = chunk.reasoning;
    const record: DaemonStreamRecord<R> = {
      kind: "ai.chat.delta",
      schema_version: 1,
      request_id: requestId,
      data: { delta } as R,
    };
    options?.onRecord?.(record);
  }
  return {
    kind: "ai.chat",
    schema_version: 1,
    request_id: requestId,
    data: {
      provider: args.provider ?? "ollama",
      model: args.model ?? "mock-model",
      finish_reason: cancelled ? "cancelled" : "stop",
    } as T,
  };
}
