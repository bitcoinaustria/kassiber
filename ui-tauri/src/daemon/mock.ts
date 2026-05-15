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
  kind?: string;
  syncMode?: string;
  syncSource?: string;
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

    if (req.kind === "ui.wallets.list") {
      const overview = mockOverviewSnapshot();
      return {
        kind: "ui.wallets.list",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          wallets: overview.connections.map((connection) => ({
            id: connection.id,
            label: connection.label,
            kind: connection.kind,
            chain:
              connection.label.toLowerCase().includes("liquid") ||
              connection.label.toLowerCase().includes("l-btc")
                ? "liquid"
                : "bitcoin",
            sync_mode: connection.syncMode ?? "descriptor",
            sync_source: connection.syncSource ?? "",
            transaction_count: 1,
            btcpay_provenance: [],
          })),
          summary: {
            count: overview.connections.length,
          },
        } as T,
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

    if (req.kind === "ui.profiles.rename") {
      const args = (req.args ?? {}) as {
        profile_id?: unknown;
        label?: unknown;
      };
      const profileId =
        typeof args.profile_id === "string" ? args.profile_id : "";
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const workspace = mockProfilesSnapshot.workspaces.find((candidate) =>
        candidate.profiles.some((profile) => profile.id === profileId),
      );
      const profile = workspace?.profiles.find(
        (candidate) => candidate.id === profileId,
      );
      if (!workspace || !profile) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "book not found",
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
            message: "Book name is required.",
            retryable: false,
          },
        };
      }
      const duplicate = workspace.profiles.some(
        (candidate) =>
          candidate.id !== profileId &&
          candidate.name.localeCompare(label, undefined, {
            sensitivity: "accent",
          }) === 0,
      );
      if (duplicate) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "conflict",
            message: "Book name already exists in this books set.",
            retryable: false,
          },
        };
      }
      mockProfilesSnapshot = {
        ...mockProfilesSnapshot,
        workspaces: mockProfilesSnapshot.workspaces.map((candidate) => ({
          ...candidate,
          profiles: candidate.profiles.map((existing) =>
            existing.id === profileId ? { ...existing, name: label } : existing,
          ),
        })),
      };
      return {
        kind: "ui.profiles.rename",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          profile: { id: profileId, name: label },
          workspace: { id: workspace.id },
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

    if (req.kind === "ui.workspace.rename") {
      const args = (req.args ?? {}) as {
        workspace_id?: unknown;
        label?: unknown;
      };
      const workspaceId =
        typeof args.workspace_id === "string" ? args.workspace_id : "";
      const label = typeof args.label === "string" ? args.label.trim() : "";
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
            message: "Books set name is required.",
            retryable: false,
          },
        };
      }
      const duplicate = mockProfilesSnapshot.workspaces.some(
        (candidate) =>
          candidate.id !== workspaceId &&
          candidate.name.localeCompare(label, undefined, {
            sensitivity: "accent",
          }) === 0,
      );
      if (duplicate) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "conflict",
            message: "Books set name already exists.",
            retryable: false,
          },
        };
      }
      mockProfilesSnapshot = {
        ...mockProfilesSnapshot,
        workspaces: mockProfilesSnapshot.workspaces.map((candidate) =>
          candidate.id === workspaceId
            ? { ...candidate, name: label }
            : candidate,
        ),
      };
      return {
        kind: "ui.workspace.rename",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          workspace: { id: workspaceId, name: label },
        } as T,
      };
    }

    if (req.kind === "ui.wallets.update") {
      const args = (req.args ?? {}) as {
        wallet?: unknown;
        label?: unknown;
        store_id?: unknown;
        wallet_material?: unknown;
        source_file?: unknown;
      };
      const walletRef = typeof args.wallet === "string" ? args.wallet : "";
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const overview = mockOverviewSnapshot();
      const connection = overview.connections.find(
        (item) => item.id === walletRef || item.label === walletRef,
      );
      const hasConfigChange =
        (typeof args.store_id === "string" && args.store_id.trim().length > 0) ||
        (typeof args.wallet_material === "string" &&
          args.wallet_material.trim().length > 0) ||
        (typeof args.source_file === "string" &&
          args.source_file.trim().length > 0);
      if (!connection || (!label && !hasConfigChange)) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message:
              "wallet update requires an existing wallet plus a label or config change",
            retryable: false,
          },
        };
      }
      if (label) connection.label = label;
      return {
        kind: "ui.wallets.update",
        schema_version: 1,
        request_id: req.request_id,
        data: { wallet: connection } as T,
      };
    }

    if (req.kind === "ui.wallets.create") {
      const args = (req.args ?? {}) as {
        label?: unknown;
        kind?: unknown;
        source_format?: unknown;
      };
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const kind = typeof args.kind === "string" ? args.kind.trim() : "custom";
      const sourceFormat =
        typeof args.source_format === "string" ? args.source_format.trim() : "";
      if (!label) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "wallet create requires a label",
            retryable: false,
          },
        };
      }
      const overview = mockOverviewSnapshot();
      const connection = {
        id: `mock-wallet-${Date.now()}`,
        label,
        kind,
        ...(sourceFormat
          ? {
              syncMode: "file_import",
              syncSource: sourceFormat,
              sourceFormat,
            }
          : {}),
      };
      overview.connections = [...overview.connections, connection];
      return {
        kind: "ui.wallets.create",
        schema_version: 1,
        request_id: req.request_id,
        data: { wallet: connection } as T,
      };
    }

    if (req.kind === "ui.connections.btcpay.create") {
      const args = (req.args ?? {}) as {
        label?: unknown;
        backend?: unknown;
        backend_label?: unknown;
        server_url?: unknown;
        api_key?: unknown;
        store_id?: unknown;
        payment_method_id?: unknown;
        payment_method_ids?: unknown;
        mode?: unknown;
        routes?: unknown;
      };
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const backendName =
        typeof args.backend === "string"
          ? args.backend.trim()
          : typeof args.backend_label === "string"
            ? args.backend_label.trim().toLowerCase().replace(/[^a-z0-9_.-]+/g, "-")
            : "";
      const hasInlineCredentials =
        typeof args.server_url === "string" &&
        args.server_url.trim() &&
        typeof args.api_key === "string" &&
        args.api_key.trim();
      const storeId = typeof args.store_id === "string" ? args.store_id.trim() : "";
      if (!label || !backendName || !storeId || (!args.backend && !hasInlineCredentials)) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "BTCPay setup requires a label, instance, and store ID",
            retryable: false,
          },
        };
      }
      const overview = mockOverviewSnapshot();
      if (args.mode === "existing_wallets") {
        const routes = Array.isArray(args.routes) ? args.routes : [];
        const mapped = routes
          .map((route) =>
            typeof route === "object" && route !== null && "wallet" in route
              ? overview.connections.find(
                  (connection) =>
                    connection.label ===
                    String((route as { wallet?: unknown }).wallet ?? ""),
                )
              : undefined,
          )
          .filter((connection): connection is MockConnection =>
            Boolean(connection),
          );
        return {
          kind: "ui.connections.btcpay.create",
          schema_version: 1,
          request_id: req.request_id,
          data: {
            mode: "existing_wallets",
            backend: { name: backendName },
            wallet: mapped[0],
            wallets: mapped,
          } as T,
        };
      }
      const rawPaymentMethodIds = Array.isArray(args.payment_method_ids)
        ? args.payment_method_ids
        : [args.payment_method_id];
      const paymentMethodIds = rawPaymentMethodIds
        .filter((id): id is string => typeof id === "string" && Boolean(id.trim()))
        .map((id) => id.trim());
      if (paymentMethodIds.length === 0) paymentMethodIds.push("BTC-CHAIN");
      const connections = paymentMethodIds.map((paymentMethodId, index) => ({
        id: `mock-btcpay-${Date.now()}-${index}`,
        label:
          paymentMethodIds.length === 1
            ? label
            : `${label} - ${paymentMethodId}`,
        kind: "custom",
        syncMode: "btcpay",
        syncSource: "btcpay",
      }));
      overview.connections = [...overview.connections, ...connections];
      return {
        kind: "ui.connections.btcpay.create",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          backend: { name: backendName },
          wallet: connections[0],
          wallets: connections,
        } as T,
      };
    }

    if (req.kind === "ui.connections.btcpay.discover") {
      const args = (req.args ?? {}) as {
        backend?: unknown;
        backend_label?: unknown;
        server_url?: unknown;
        api_key?: unknown;
      };
      const backend =
        typeof args.backend === "string" && args.backend.trim()
          ? args.backend.trim()
          : typeof args.backend_label === "string" && args.backend_label.trim()
            ? args.backend_label.trim()
            : "btcpay";
      const hasSource =
        typeof args.backend === "string" ||
        (typeof args.server_url === "string" &&
          args.server_url.trim() &&
          typeof args.api_key === "string" &&
          args.api_key.trim());
      if (!hasSource) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "BTCPay discovery requires a saved instance or URL and API key",
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.connections.btcpay.discover",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          backend,
          stores: [
            { id: "store-main", name: "Main store", default_currency: "EUR" },
            { id: "store-events", name: "Events", default_currency: "EUR" },
          ],
          payment_methods: [
            {
              store_id: "store-main",
              payment_method_id: "BTC-CHAIN",
              label: "BTC on-chain",
              enabled: true,
              sync_supported: true,
            },
            {
              store_id: "store-main",
              payment_method_id: "LBTC-CHAIN",
              label: "Liquid on-chain",
              enabled: true,
              sync_supported: true,
            },
            {
              store_id: "store-events",
              payment_method_id: "BTC-CHAIN",
              label: "BTC on-chain",
              enabled: true,
              sync_supported: true,
            },
          ],
        } as T,
      };
    }

    if (req.kind === "ui.metadata.bip329.import") {
      return {
        kind: "ui.metadata.bip329.import",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          records: 1,
          imported: 1,
          updated: 0,
          transaction_tags_added: 1,
        } as T,
      };
    }

    if (req.kind === "ui.transactions.metadata.update") {
      const args = (req.args ?? {}) as {
        transaction?: unknown;
        note?: unknown;
        tags?: unknown;
        excluded?: unknown;
      };
      const transactionId = typeof args.transaction === "string" ? args.transaction : "";
      const transactionList = fixtures["ui.transactions.list"] as {
        txs?: Array<Record<string, unknown>>;
      };
      const tx = transactionList.txs?.find((row) => row.id === transactionId);
      if (!tx) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Transaction '${transactionId}' not found`,
            retryable: false,
          },
        };
      }
      if (
        "excluded" in args &&
        typeof args.excluded !== "boolean"
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "excluded must be a boolean",
            retryable: false,
          },
        };
      }
      if (
        "tags" in args &&
        (!Array.isArray(args.tags) ||
          args.tags.some((tag) => typeof tag !== "string"))
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "tags must be a list of strings",
            retryable: false,
          },
        };
      }
      const tags = Array.isArray(args.tags)
        ? args.tags.filter((tag): tag is string => typeof tag === "string")
        : undefined;
      if ("note" in args) tx.note = typeof args.note === "string" ? args.note : "";
      if (tags) {
        tx.tags = tags;
        tx.tag = tags.join(", ");
      }
      if (typeof args.excluded === "boolean") tx.excluded = args.excluded;
      return {
        kind: "ui.transactions.metadata.update",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          transaction_id: transactionId,
          note: typeof args.note === "string" ? args.note : "",
          tags: (tags ?? []).map((tag) => ({ code: tag.toLowerCase(), label: tag })),
          excluded: args.excluded === true,
          updated: true,
        } as T,
      };
    }

    if (req.kind === "ui.connections.sources") {
      return {
        kind: "ui.connections.sources",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          wallet_kinds: [
            { kind: "descriptor", summary: "Watch-only descriptor wallet." },
            { kind: "custom", summary: "Custom config wallet." },
            { kind: "phoenix", summary: "Phoenix CSV importer." },
            { kind: "river", summary: "River CSV importer." },
          ],
          source_formats: [
            "btcpay_csv",
            "btcpay_json",
            "csv",
            "json",
            "phoenix_csv",
            "river_csv",
          ],
        } as T,
      };
    }

    if (req.kind === "ui.wallets.preview_descriptor") {
      const args = (req.args ?? {}) as {
        wallet_material?: unknown;
        descriptor?: unknown;
        count?: unknown;
      };
      const material =
        typeof args.wallet_material === "string"
          ? args.wallet_material.trim()
          : typeof args.descriptor === "string"
            ? args.descriptor.trim()
            : "";
      if (!material) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "preview requires wallet_material or descriptor",
            retryable: false,
          },
        };
      }
      const requested = typeof args.count === "number" ? args.count : 5;
      const count = Math.max(1, Math.min(20, requested));
      const sampleAddresses: Array<{
        branch: "receive" | "change";
        index: number;
        address: string;
        derivation_path: string;
      }> = Array.from({ length: count }, (_, index) => ({
        branch: "receive",
        index,
        address: `bc1qmock${index.toString().padStart(38, "0")}`,
        derivation_path: `m/0/${index}`,
      }));
      sampleAddresses.push({
        branch: "change",
        index: 0,
        address: `bc1qmockchange${"0".repeat(31)}`,
        derivation_path: "m/1/0",
      });
      return {
        kind: "ui.wallets.preview_descriptor",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          chain: "bitcoin",
          network: "main",
          addresses: sampleAddresses,
          has_change_branch: true,
        } as T,
      };
    }

    if (req.kind === "ui.connections.btcpay.test") {
      const args = (req.args ?? {}) as {
        backend?: unknown;
        backend_label?: unknown;
        server_url?: unknown;
        api_key?: unknown;
        store_id?: unknown;
        payment_method_id?: unknown;
      };
      const backend =
        typeof args.backend === "string" && args.backend.trim()
          ? args.backend.trim()
          : typeof args.backend_label === "string" && args.backend_label.trim()
            ? args.backend_label.trim()
            : "";
      const storeId =
        typeof args.store_id === "string" ? args.store_id.trim() : "";
      const hasInlineCredentials =
        typeof args.server_url === "string" &&
        args.server_url.trim() &&
        typeof args.api_key === "string" &&
        args.api_key.trim();
      if ((!backend && !hasInlineCredentials) || !storeId) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "BTCPay test requires instance details and store_id",
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.connections.btcpay.test",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          backend,
          store_id: storeId,
          payment_method_id:
            typeof args.payment_method_id === "string" &&
            args.payment_method_id.trim()
              ? args.payment_method_id.trim()
              : "BTC-CHAIN",
          ok: true,
        } as T,
      };
    }

    if (req.kind === "ui.backends.electrum.test") {
      const args = (req.args ?? {}) as {
        url?: unknown;
        trust_self_signed?: unknown;
        certificate?: unknown;
        proxy?: unknown;
      };
      const url = typeof args.url === "string" ? args.url.trim() : "";
      const trustSelfSigned = args.trust_self_signed === true;
      const certificate =
        typeof args.certificate === "string" ? args.certificate.trim() : "";
      const proxy = typeof args.proxy === "string" ? args.proxy.trim() : "";
      if (!url) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Electrum test requires url",
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.backends.electrum.test",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          ok: true,
          url,
          trust_self_signed: trustSelfSigned,
          logs: [
            `Opening Electrum connection to ${url}`,
            trustSelfSigned
              ? "Certificate verification: self-signed certificate trusted for this test."
              : certificate
                ? `Certificate verification: pinned certificate ${certificate}.`
                : "Certificate verification: system trust store.",
            proxy ? `Proxy: ${proxy}.` : "Proxy: disabled.",
            "Connected.",
            "Server version: Fulcrum 2.0 on protocol version 1.4.2",
            "Server banner: Connected to a Fulcrum 2.0 server",
          ],
        } as T,
      };
    }

    if (req.kind === "ui.backends.http.test") {
      const args = (req.args ?? {}) as { url?: unknown };
      const url = typeof args.url === "string" ? args.url.trim() : "";
      if (!url) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "HTTP backend test requires url",
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.backends.http.test",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          ok: true,
          url,
          status: 200,
          logs: [
            `$ curl -fsS -L --max-time 10 -H 'Accept: application/json' ${url}`,
            `> GET ${url}`,
            "< HTTP 200 OK",
            "< content-type: application/json",
            "< body: 256 bytes sampled",
          ],
        } as T,
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

    if (req.kind === "ui.rates.kraken_csv.import") {
      const args = (req.args ?? {}) as {
        operation?: unknown;
        path?: unknown;
      };
      const operation =
        args.operation === "incremental" ? "incremental" : "full";
      return {
        kind: "ui.rates.kraken_csv.import",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          source: "kraken-csv",
          operation,
          path:
            typeof args.path === "string" && args.path.trim()
              ? args.path.trim()
              : "/mock/Kraken_OHLCVT.zip",
          pair: null,
          summary: [
            {
              pair: "BTC-EUR",
              samples: 10,
              files: 1,
              skipped_rows: 0,
              skipped_files: 0,
              first_timestamp: "2024-05-01T00:00:00Z",
              last_timestamp: "2024-05-01T00:09:00Z",
            },
          ],
          totals: {
            pairs: 1,
            samples: 10,
            rows: 10,
            files: 1,
            skipped_rows: 0,
            skipped_files: 0,
          },
        } as T,
      };
    }

    if (req.kind === "ui.rates.rebuild") {
      return {
        kind: "ui.rates.rebuild",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          source: "coinbase-exchange",
          pair: null,
          days: 30,
          reprice_transactions: true,
          deleted: {
            rates: 420,
            checked_minutes: 900,
            transaction_prices: 8,
            profiles_invalidated: 1,
          },
          sync: [
            {
              pair: "BTC-EUR",
              samples: 300,
              windows: 1,
              missing_minutes: 3,
              checked_minutes: 300,
            },
          ],
          journals: {
            entries_created: 12,
            quarantined: 0,
            auto_priced: 8,
          },
        } as T,
      };
    }

    if (req.kind === "ai.providers.set_api_key") {
      const args = (req.args ?? {}) as { name?: unknown; api_key?: unknown };
      const name = typeof args.name === "string" ? args.name.trim() : "";
      if (!name) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "ai.providers.set_api_key requires a provider name",
            retryable: false,
          },
        };
      }
      return {
        kind: "ai.providers.set_api_key",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          name,
          has_api_key: Boolean(
            typeof args.api_key === "string" && args.api_key.trim(),
          ),
          secret_ref: {
            store_id: "sqlcipher_inline",
            state:
              typeof args.api_key === "string" && args.api_key.trim()
                ? "ok"
                : "missing",
          },
        } as T,
      };
    }

    if (req.kind === "ai.providers.move_api_key") {
      const args = (req.args ?? {}) as { name?: unknown; store_id?: unknown };
      const name = typeof args.name === "string" ? args.name.trim() : "";
      const storeId =
        typeof args.store_id === "string" && args.store_id.trim()
          ? args.store_id.trim()
          : "sqlcipher_inline";
      if (!name) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "ai.providers.move_api_key requires a provider name",
            retryable: false,
          },
        };
      }
      return {
        kind: "ai.providers.move_api_key",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          name,
          has_api_key: true,
          secret_ref: {
            store_id: storeId,
            state: "ok",
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
    if (req.kind === "ui.wallets.sync") {
      return mockWalletsSyncStream<T, R>(req, options);
    }
    // Non-streaming kinds resolve straight through to invoke.
    return mockDaemon.invoke<T>(req);
  },
};

export const mockStream = mockDaemon.stream;

async function mockWalletsSyncStream<T, R>(
  req: DaemonRequest,
  options?: DaemonStreamOptions<R>,
): Promise<DaemonEnvelope<T>> {
  const requestId =
    req.request_id ?? `mock-sync-${Math.random().toString(36).slice(2)}`;
  const args = (req.args ?? {}) as { wallet?: unknown };
  const walletLabel = typeof args.wallet === "string" ? args.wallet : "wallet";
  const total = 1200;
  const steps = [200, 600, 1000, total];
  for (const processed of steps) {
    if (options?.signal?.aborted) break;
    await new Promise((resolve) => setTimeout(resolve, 60));
    options?.onRecord?.({
      kind: "ui.wallets.sync.progress",
      schema_version: 1,
      request_id: requestId,
      data: {
        phase: "importing",
        wallet: walletLabel,
        processed,
        total,
        imported: processed,
        skipped: 0,
      } as R,
    });
  }
  return mockDaemon.invoke<T>(req);
}

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
