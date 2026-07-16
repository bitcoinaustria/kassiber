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
import {
  DEFAULT_OPEN_COST_SAT,
  connectionSupportsLightningCapability,
  type LightningCapabilities,
} from "@/lib/lightning";
import { accountMatchesLabel } from "@/lib/connectionTransactions";
import { MOCK_PROFILES } from "@/mocks/profiles";
import { MOCK_TRANSACTION_GRAPHS } from "@/mocks/transactions";
import type {
  ProfileGainsAlgorithm,
  ProfileTaxCountry,
  Workspace,
} from "@/mocks/profiles";
import { mockWorkspaceOverviewSnapshot } from "@/mocks/workspaceOverview";
import { buildExitTaxFixture, type ExitTaxDestination } from "@/mocks/exitTax";
import { MOCK_AI_CHAT_STREAM, fixtures } from "./fixtures";

// Mirror the daemon's `_tax_policy_label` / `_human_tax_method`: an Austrian
// book reflects its ACTUAL stored method, so an AT book on FIFO reads as
// "Austria - FIFO", not the moving-average label "ATM".
function mockTaxPolicyLabel(
  country: ProfileTaxCountry,
  algorithm: ProfileGainsAlgorithm,
  fiat: string,
): string {
  if (country === "at") {
    const method = algorithm === "MOVING_AVERAGE_AT" ? "ATM" : algorithm;
    return `Austria - ${method} - ${fiat}`;
  }
  return `Generic - ${algorithm} - ${fiat}`;
}

interface MockChatSession {
  id: string;
  title: string;
  provider: string;
  model: string;
  created_at: string;
  updated_at: string;
  entries: { role: "user" | "assistant"; content: string }[];
}

function mockGraphlessTransactionGraph(transactionId: string) {
  return {
    transaction: {
      id: transactionId,
      txid: transactionId,
      externalId: transactionId,
      inputCount: 0,
      outputCount: 0,
    },
    supportLevel: "graphless",
    unsupportedReason: "graphless_import",
    warnings: [
      {
        code: "graphless_import",
        level: "info",
        message:
          "This source record does not contain a valued Bitcoin input/output graph.",
      },
    ],
    inputs: [],
    outputs: [],
    fee: null,
    annotations: [],
    accounting: { quarantine: null, linkedPairs: [], transferGroupIds: [] },
  };
}

// Mock chat history uses "encrypted database" semantics: the auto policy
// persists, mirroring a real install after `secrets init`.
let mockChatHistoryMode: "auto" | "on" | "off" = "auto";
const mockChatSessions: MockChatSession[] = [
  {
    id: "mock-chat-session-1",
    title: "Largest outbound BTC transactions this quarter",
    provider: "ollama",
    model: "mock-model",
    created_at: "2026-06-08T09:12:00Z",
    updated_at: "2026-06-09T16:40:00Z",
    entries: [
      {
        role: "user",
        content:
          "What were my largest outbound BTC transactions this quarter?",
      },
      {
        role: "assistant",
        content:
          "Your largest outbound transaction was 0.85 BTC from Cold storage on 2026-05-14, followed by 0.32 BTC from the Lightning sweep wallet on 2026-04-02.",
      },
    ],
  },
  {
    id: "mock-chat-session-2",
    title: "Why is my tax summary stale?",
    provider: "ollama",
    model: "mock-model",
    created_at: "2026-06-05T18:03:00Z",
    updated_at: "2026-06-05T18:05:00Z",
    entries: [
      { role: "user", content: "Why is my tax summary stale?" },
      {
        role: "assistant",
        content:
          "Journals have not been reprocessed since the last BTC import. Run journal processing and the tax summary will refresh.",
      },
    ],
  },
];

function mockChatHistoryEnabled(): boolean {
  return mockChatHistoryMode !== "off";
}

function mockChatSessionSummaries() {
  return [...mockChatSessions]
    .sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1))
    .map(({ entries, ...rest }) => ({
      ...rest,
      message_count: entries.length,
    }));
}

const SIMULATED_LATENCY_MS = 50;
const MAX_DESCRIPTOR_GAP_LIMIT = 5000;
const MAX_ATTACHMENT_LABEL_LENGTH = 200;

function mockUrlDisplayLabel(rawUrl?: string) {
  if (!rawUrl) return "Link attachment";
  try {
    const parsed = new URL(rawUrl);
    const host = parsed.hostname.replace(/^www\./i, "");
    if (host === "docs.google.com") {
      if (parsed.pathname.startsWith("/document/d/")) return "Google Doc";
      if (parsed.pathname.startsWith("/spreadsheets/d/")) return "Google Sheet";
      if (parsed.pathname.startsWith("/presentation/d/")) return "Google Slides deck";
      return "Google Workspace link";
    }
    if (host === "drive.google.com") return "Google Drive link";
    const pathParts = parsed.pathname.split("/").filter(Boolean);
    const slug = pathParts.at(-1)?.replace(/\.[a-z0-9]{2,5}$/i, "");
    if (slug && !/^[a-f0-9-]{16,}$/i.test(slug)) {
      return `${host} - ${decodeURIComponent(slug).replace(/[-_+]+/g, " ")}`;
    }
    return host || "Link attachment";
  } catch {
    return "Link attachment";
  }
}

function mockAttachmentDisplayLabel(attachment: {
  attachment_type: "file" | "url";
  label?: string | null;
  original_filename?: string;
  url?: string;
}) {
  const label = attachment.label?.trim();
  if (label) return label;
  if (attachment.attachment_type === "url") return mockUrlDisplayLabel(attachment.url);
  return attachment.original_filename || "File attachment";
}

const cloneMockProfiles = () => ({
  activeWorkspaceId: MOCK_PROFILES.activeWorkspaceId,
  activeProfileId: MOCK_PROFILES.activeProfileId,
  workspaces: MOCK_PROFILES.workspaces.map((workspace) => ({
    ...workspace,
    profiles: workspace.profiles.map((profile) => ({ ...profile })),
  })),
});

let mockProfilesSnapshot = cloneMockProfiles();

type MockAttachment = {
  id: string;
  transaction_id: string;
  attachment_type: "file" | "url";
  label?: string | null;
  display_label: string;
  original_filename?: string;
  url?: string;
  media_type?: string;
  size_bytes?: number | null;
  sha256?: string;
  stored_relpath?: string;
  copied_from_attachment_id?: string;
  copied_from_transaction_id?: string;
  exists?: boolean | null;
  created_at: string;
};

type MockTransactionHistoryField = {
  id: string;
  field: string;
  label: string;
  family: string;
  before_value: unknown;
  after_value: unknown;
  before_label: string;
  after_label: string;
  diff: Record<string, unknown>;
  redacted?: boolean;
};

type MockTransactionHistoryEvent = {
  id: string;
  transaction_id: string;
  transaction_external_id: string;
  transaction_occurred_at: string;
  wallet_id: string;
  wallet_label: string;
  source: string;
  source_label: string;
  reason: string;
  changed_at: string;
  summary: string;
  families: string[];
  report_anchor: Record<string, unknown>;
  transaction: Record<string, unknown>;
  fields: MockTransactionHistoryField[];
};

let mockAttachments: MockAttachment[] = [
  {
    id: "att-tx2-1",
    transaction_id: "tx2",
    attachment_type: "url",
    label: "Board approval reference",
    display_label: "Board approval reference",
    url: "https://docs.example.com/board/approval",
    media_type: "text/uri-list",
    exists: null,
    created_at: "2026-03-31T09:00:00Z",
  },
  {
    id: "att-tx1-1",
    transaction_id: "tx1",
    attachment_type: "file",
    label: "invoice-2026-04-18.pdf",
    display_label: "invoice-2026-04-18.pdf",
    original_filename: "invoice-2026-04-18.pdf",
    media_type: "application/pdf",
    size_bytes: 248_000,
    sha256: "a91f000000000000000000000000000000000000000000000000000000007c",
    stored_relpath: "mock/invoice-2026-04-18.pdf",
    exists: true,
    created_at: "2026-04-18T14:22:00Z",
  },
  {
    id: "att-tx1-2",
    transaction_id: "tx1",
    attachment_type: "url",
    label: null,
    display_label: "btcpay.example.com - abc123",
    url: "https://btcpay.example.com/invoices/abc123",
    media_type: "text/uri-list",
    exists: null,
    created_at: "2026-04-18T14:23:00Z",
  },
];

let mockTransactionHistory: MockTransactionHistoryEvent[] = [
  {
    id: "edit-mock-2",
    transaction_id: "tx2",
    transaction_external_id: "tx2",
    transaction_occurred_at: "2026-04-17T09:08:00Z",
    wallet_id: "wallet-home-node",
    wallet_label: "Home Node (CLN)",
    source: "ai_tool",
    source_label: "Assistant",
    reason: "Suggested hosting classification",
    changed_at: "2026-04-18T08:12:00Z",
    summary: "Updated Review status, Taxable",
    families: ["tax"],
    report_anchor: { stale_for_reports: true, journal_input_version_after: 8 },
    transaction: {
      id: "tx2",
      external_id: "tx2",
      occurred_at: "2026-04-17T09:08:00Z",
      direction: "outbound",
      asset: "BTC",
      amount: 0.00120431,
      amount_msat: 120_431_000,
      fee: 0,
      fee_msat: 0,
      counterparty: "Server rental · Hetzner",
    },
    fields: [
      {
        id: "edit-field-mock-3",
        field: "review_status",
        label: "Review status",
        family: "tax",
        before_value: "review",
        after_value: "completed",
        before_label: "Needs review",
        after_label: "Completed",
        diff: {},
      },
      {
        id: "edit-field-mock-4",
        field: "taxable",
        label: "Taxable",
        family: "tax",
        before_value: true,
        after_value: false,
        before_label: "Taxable",
        after_label: "Not taxable",
        diff: {},
      },
    ],
  },
  {
    id: "edit-mock-1",
    transaction_id: "tx1",
    transaction_external_id: "tx1",
    transaction_occurred_at: "2026-04-18T14:22:00Z",
    wallet_id: "wallet-cold",
    wallet_label: "Cold Storage",
    source: "gui",
    source_label: "Desktop",
    reason: "Matched invoice evidence",
    changed_at: "2026-04-18T07:42:00Z",
    summary: "Pricing provenance updated",
    families: ["pricing", "metadata"],
    report_anchor: { stale_for_reports: true, journal_input_version_after: 7 },
    transaction: {
      id: "tx1",
      external_id: "tx1",
      occurred_at: "2026-04-18T14:22:00Z",
      direction: "inbound",
      asset: "BTC",
      amount: 0.0245,
      amount_msat: 2_450_000_000,
      fee: 0,
      fee_msat: 0,
      counterparty: "Invoice · ACME GmbH",
    },
    fields: [
      {
        id: "edit-field-mock-1",
        field: "tags",
        label: "Tags",
        family: "metadata",
        before_value: ["Revenue"],
        after_value: ["Invoice", "Revenue"],
        before_label: "Revenue",
        after_label: "Invoice, Revenue",
        diff: { added: ["Invoice"], removed: [], before: ["Revenue"], after: ["Invoice", "Revenue"] },
      },
      {
        id: "edit-field-mock-2",
        field: "pricing_external_ref",
        label: "Pricing evidence reference",
        family: "pricing",
        before_value: null,
        after_value: "invoice=ACME-42 secret=[redacted]",
        before_label: "Empty",
        after_label: "invoice=ACME-42 secret=[redacted]",
        diff: {},
        redacted: true,
      },
    ],
  },
];
let mockAttachmentCounter = 0;

function mockAuditEvidenceSummary(transactionId: string) {
  const direct = mockAttachments
    .filter((attachment) => !transactionId || attachment.transaction_id === transactionId)
    .map((attachment) => ({
      id: attachment.id,
      attachment_type: attachment.attachment_type,
      label: attachment.label,
      media_type: attachment.media_type ?? "",
      size_bytes: attachment.size_bytes ?? null,
      sha256: attachment.sha256 ?? "",
      exists: attachment.exists ?? null,
      copied_from_attachment_id: attachment.copied_from_attachment_id ?? "",
      copied_from_transaction_id: attachment.copied_from_transaction_id ?? "",
      url_host:
        attachment.attachment_type === "url" && attachment.url
          ? new URL(attachment.url).host
          : "",
    }));
  const status = direct.length ? "warning" : "blocked";
  return {
    schema_version: 1,
    workspace: { id: "mock-ws", label: "Mock workspace" },
    profile: { id: "mock-profile", label: "Demo book" },
    scope: { type: transactionId ? "transactions" : "active_profile" },
    journal_freshness: {
      status: "current",
      needs_processing: false,
      reason: "mock journals are current",
    },
    transactions: [
      {
        transaction: {
          id: transactionId || "tx1",
          external_id: transactionId || "tx1",
          asset: "BTC",
        },
        readiness: {
          status,
          warnings: [
            ...(direct.length
              ? []
              : [
                  {
                    code: "receipt_missing",
                    severity: "blocker",
                    message: "No direct receipt, note, file, or URL reference is attached to this transaction.",
                    action: "Attach a local receipt file or a URL reference from transaction detail.",
                  },
                ]),
            {
              code: "source_link_unreviewed",
              severity: "blocker",
              message: "At least one source-of-funds suggestion is still unreviewed.",
              action: "Accept, edit, or reject the suggested link.",
            },
            {
              code: "sensitive_material_excluded",
              severity: "info",
              message: "Descriptors, xpubs, backend URLs, credentials, wallet files, logs, AI settings, and technical wallet evidence are excluded from this audit surface.",
            },
          ],
        },
        direct_attachments: direct,
        source_funds_links: [
          {
            id: "mock-sof-link-1",
            link_type: "manual_source",
            state: "suggested",
            confidence: "strong",
            method: "mock_fixture",
            asset: "BTC",
            allocation_amount: 0.01,
            allocation_policy: "explicit",
            explanation: "Mock source-funds link for browser preview.",
            attachments: [],
            from_source: {
              id: "mock-source-1",
              source_type: "fiat_purchase",
              label: "Exchange statement",
              review_state: "reviewed",
              attachments: [],
            },
          },
        ],
      },
    ],
    summary: {
      transaction_count: 1,
      ready_count: 0,
      blocked_count: status === "blocked" ? 1 : 0,
      warning_count: status === "warning" ? 1 : 0,
    },
    excluded_sensitive_material: [
      "wallet descriptors",
      "xpubs",
      "backend credentials",
      "backend URLs",
      "raw wallet files",
      "environment files",
      "logs",
      "AI settings",
      "unrelated books",
      "technical wallet evidence",
    ],
  };
}

type MockConnection = {
  id: string;
  label: string;
  kind?: string;
  syncMode?: string;
  syncSource?: string;
  chain?: string;
  network?: string;
  paymentMethodId?: string;
  gap?: number;
  lightningCapabilities?: LightningCapabilities;
  /** balance in BTC (float) — present on overview-snapshot connection rows. */
  balance?: number;
};

const mockOverviewSnapshot = () =>
  fixtures["ui.overview.snapshot"] as {
    connections: MockConnection[];
  };

type MockBackendSettingsRow = {
  name: string;
  display_name?: string;
  kind: string;
  chain: string;
  network: string;
  url: string;
  source: string;
  is_default?: boolean;
  has_url?: boolean;
  has_auth_header?: boolean;
  has_token?: boolean;
  has_username?: boolean;
  has_password?: boolean;
  has_commando_peer_id?: boolean;
  has_lightning_dir?: boolean;
  has_rpc_file?: boolean;
  insecure?: boolean;
  tor_proxy?: string;
  infrastructure_owner?: string;
};

let mockBackendSettingsRows: MockBackendSettingsRow[] = [
  {
    name: "mempool",
    kind: "esplora",
    chain: "bitcoin",
    network: "main",
    url: "https://mempool.bitcoin-austria.at/api",
    source: "mock",
    is_default: true,
    has_url: true,
    display_name: "mempool.bitcoin-austria.at",
  },
  {
    name: "liquid",
    kind: "electrum",
    chain: "liquid",
    network: "liquidv1",
    url: "ssl://les.bullbitcoin.com:995",
    source: "mock",
    has_url: true,
    display_name: "BullBitcoin Liquid Electrum",
  },
  {
    name: "liquid-blockstream",
    kind: "electrum",
    chain: "liquid",
    network: "liquidv1",
    url: "ssl://blockstream.info:995",
    source: "mock",
    has_url: true,
    display_name: "Blockstream Liquid Electrum",
  },
  {
    name: "fulcrum-onion-long",
    kind: "electrum",
    chain: "bitcoin",
    network: "main",
    url: "tcp://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50001",
    source: "mock",
    has_url: true,
    display_name: "Very Long Onion Fulcrum",
    tor_proxy: "127.0.0.1:9050",
  },
];

const mockBackendSettingsPayload = () => ({
  backends: mockBackendSettingsRows.map((row) => ({ ...row })),
  summary: {
    count: mockBackendSettingsRows.length,
    default_backend:
      mockBackendSettingsRows.find((row) => row.is_default)?.name ?? null,
  },
});

const mockBackendPublicDefaultsPayload = () => {
  const backends = mockBackendSettingsRows
    .filter((row) =>
      ["electrum", "esplora", "liquid-esplora"].includes(row.kind),
    )
    .map((row) => ({
      name: row.name,
      kind: row.kind,
      chain: row.chain,
      network: row.network,
      url: row.url,
      source: row.source,
      is_default: row.is_default,
    }));
  return {
    backends,
    summary: {
      count: backends.length,
      default_backend:
        mockBackendSettingsRows.find((row) => row.is_default)?.name ?? null,
    },
  };
};

function mockBackendRowFromArgs(
  args: Record<string, unknown>,
  existing?: MockBackendSettingsRow,
): MockBackendSettingsRow {
  const clear = new Set(
    Array.isArray(args.clear)
      ? args.clear.filter((item): item is string => typeof item === "string")
      : [],
  );
  const config =
    args.config && typeof args.config === "object" && !Array.isArray(args.config)
      ? (args.config as Record<string, unknown>)
      : {};
  const row: MockBackendSettingsRow = {
    name:
      typeof args.name === "string" && args.name.trim()
        ? args.name.trim()
        : existing?.name ?? "backend",
    kind:
      typeof args.kind === "string" && args.kind.trim()
        ? args.kind.trim()
        : existing?.kind ?? "esplora",
    chain:
      typeof args.chain === "string" && args.chain.trim()
        ? args.chain.trim()
        : existing?.chain ?? "bitcoin",
    network:
      typeof args.network === "string" && args.network.trim()
        ? args.network.trim()
        : existing?.network ?? "main",
    url:
      typeof args.url === "string" && args.url.trim()
        ? args.url.trim()
        : existing?.url ?? "https://example.invalid/api",
    source: "mock",
    display_name:
      typeof config.display_name === "string" && config.display_name.trim()
        ? config.display_name.trim()
        : existing?.display_name,
    is_default: existing?.is_default,
    has_url: true,
    has_auth_header: clear.has("auth_header") || clear.has("auth-header")
      ? false
      : typeof args.auth_header === "string" && args.auth_header.trim()
        ? true
        : existing?.has_auth_header,
    has_token: clear.has("token")
      ? false
      : typeof args.token === "string" && args.token.trim()
        ? true
        : existing?.has_token,
    has_username: clear.has("username")
      ? false
      : typeof config.username === "string" && config.username.trim()
        ? true
        : existing?.has_username,
    has_password: clear.has("password")
      ? false
      : typeof config.password === "string" && config.password.trim()
        ? true
        : existing?.has_password,
    has_commando_peer_id: clear.has("commando_peer_id")
      ? false
      : typeof config.commando_peer_id === "string" &&
          config.commando_peer_id.trim()
        ? true
        : existing?.has_commando_peer_id,
    has_lightning_dir: clear.has("lightning_dir")
      ? false
      : typeof config.lightning_dir === "string" && config.lightning_dir.trim()
        ? true
        : existing?.has_lightning_dir,
    has_rpc_file: clear.has("rpc_file")
      ? false
      : typeof config.rpc_file === "string" && config.rpc_file.trim()
        ? true
        : existing?.has_rpc_file,
    insecure:
      typeof config.insecure === "boolean"
        ? config.insecure
        : existing?.insecure,
    tor_proxy: clear.has("tor_proxy")
      ? undefined
      : typeof args.tor_proxy === "string" && args.tor_proxy.trim()
        ? args.tor_proxy.trim()
        : existing?.tor_proxy,
    infrastructure_owner:
      typeof config.infrastructure_owner === "string" &&
      config.infrastructure_owner.trim()
        ? config.infrastructure_owner.trim()
        : existing?.infrastructure_owner,
  };
  return row;
}

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

    if (req.kind === "ui.chat.sessions.list") {
      return {
        kind: "ui.chat.sessions.list",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          sessions: mockChatSessionSummaries(),
          history_mode: mockChatHistoryMode,
          history_enabled: mockChatHistoryEnabled(),
        } as T,
      };
    }

    if (req.kind === "ui.chat.sessions.get") {
      const sessionId = (req.args ?? ({} as Record<string, unknown>))
        .session_id as string | undefined;
      const session = mockChatSessions.find((row) => row.id === sessionId);
      if (!session) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: "chat session not found for the active profile",
          },
        } as DaemonEnvelope<T>;
      }
      const { entries, ...rest } = session;
      return {
        kind: "ui.chat.sessions.get",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          ...rest,
          message_count: entries.length,
          messages: entries.map((entry, index) => ({
            id: `${session.id}-m${index}`,
            seq: index,
            role: entry.role,
            content: entry.content,
          })),
        } as T,
      };
    }

    if (req.kind === "ui.chat.sessions.delete") {
      const sessionId = (req.args ?? ({} as Record<string, unknown>))
        .session_id as string | undefined;
      const index = mockChatSessions.findIndex((row) => row.id === sessionId);
      if (index < 0) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: "chat session not found for the active profile",
          },
        } as DaemonEnvelope<T>;
      }
      mockChatSessions.splice(index, 1);
      return {
        kind: "ui.chat.sessions.delete",
        schema_version: 1,
        request_id: req.request_id,
        data: { deleted: sessionId } as T,
      };
    }

    if (req.kind === "ui.chat.sessions.clear") {
      const deleted = mockChatSessions.length;
      mockChatSessions.length = 0;
      return {
        kind: "ui.chat.sessions.clear",
        schema_version: 1,
        request_id: req.request_id,
        data: { deleted } as T,
      };
    }

    if (req.kind === "ui.chat.history.configure") {
      const history = (req.args ?? ({} as Record<string, unknown>))
        .history as string | undefined;
      if (history === "auto" || history === "on" || history === "off") {
        mockChatHistoryMode = history;
      } else if (history !== undefined) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "chat history mode must be one of auto, on, off",
          },
        } as DaemonEnvelope<T>;
      }
      return {
        kind: "ui.chat.history.configure",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          history: mockChatHistoryMode,
          history_enabled: mockChatHistoryEnabled(),
          database_encrypted: true,
        } as T,
      };
    }

    if (req.kind === "ui.wallets.ledger_preview") {
      return {
        kind: "ui.wallets.ledger_preview",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          rows_read: 3,
          mapped: 2,
          errors: 1,
          truncated: false,
          confident: true,
          detected: [
            { column: "Date", field: "date" },
            { column: "Received BTC", field: "received" },
            { column: "Sent BTC", field: "sent" },
            { column: "Price", field: "fiat_rate" },
          ],
          problems: [{ row: 3, message: "Ledger row 3: unknown Type 'Frobnicate'" }],
          preview: [
            {
              occurred_at: "2026-01-15",
              direction: "inbound",
              asset: "BTC",
              amount: "0.05000000",
              fee: "0",
              kind: "buy",
              fiat_currency: "EUR",
              fiat_value: "3000.00",
              description: "Bought",
            },
            {
              occurred_at: "2026-02-10",
              direction: "outbound",
              asset: "BTC",
              amount: "0.01000000",
              fee: "0.00001000",
              kind: "sell",
              fiat_currency: "EUR",
              fiat_value: "720.00",
              description: "Sold",
            },
          ],
        } as T,
      };
    }

    if (req.kind === "ui.wallets.document_import.preview") {
      const args = (req.args ?? {}) as { document_token?: unknown };
      const documentToken =
        typeof args.document_token === "string" && args.document_token
          ? args.document_token
          : "mock-document-session";
      return {
        kind: "ui.wallets.document_import.preview",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          document_token: documentToken,
          source: {
            filename: "receipt.png",
            media_type: "image/png",
            sha256: "mock",
          },
          model: "glm-ocr",
          confidence_threshold: 0.78,
          recommendations: [
            { id: "glm-ocr", command: "ollama pull glm-ocr" },
            { id: "qwen3-vl:8b", command: "ollama pull qwen3-vl:8b" },
          ],
          rows: [
            {
              id: "docrow-001",
              status: "ready",
              flags: [],
              confidence: 0.93,
              confidence_threshold: 0.78,
              cell_confidences: {
                occurred_at: 0.98,
                direction: 0.97,
                asset: 0.99,
                amount_btc: 0.96,
                fee_btc: 0.95,
                fiat_currency: 0.94,
                fiat_value: 0.93,
                fiat_rate: 0.92,
                counterparty: 0.91,
                description: 0.9,
              },
              source_region: { page: 1 },
              evidence_text: "2026-01-15 BTC 0.01000000 EUR 620 OTC Desk",
              record: {
                occurred_at: "2026-01-15",
                direction: "inbound",
                asset: "BTC",
                amount_btc: "0.01000000",
                fee_btc: "0",
                fiat_currency: "EUR",
                fiat_value: "620.00",
                fiat_rate: "62000.00",
                counterparty: "OTC Desk",
                description: "Receipt row",
              },
              import_record: {
                id: "docrow-001",
                occurred_at: "2026-01-15",
                direction: "inbound",
                asset: "BTC",
                amount: "0.01000000",
                fee: "0",
                fiat_currency: "EUR",
                fiat_value: "620.00",
                fiat_rate: "62000.00",
                counterparty: "OTC Desk",
                description: "Receipt row",
              },
            },
            {
              id: "docrow-002",
              status: "quarantined",
              flags: ["missing_direction", "low_row_confidence"],
              confidence: 0.55,
              evidence_text: "handwritten 0.002 BTC",
              record: {
                occurred_at: "2026-01-16",
                direction: null,
                asset: "BTC",
                amount_btc: "0.00200000",
              },
              import_record: null,
            },
          ],
          summary: {
            rows: 2,
            ready: 1,
            quarantined: 1,
            has_importable_rows: true,
          },
          created_at: new Date().toISOString(),
        } as T,
      };
    }

    if (req.kind === "ui.wallets.document_import.import") {
      const args = (req.args ?? {}) as { selected_row_ids?: unknown };
      const selected =
        Array.isArray(args.selected_row_ids) ? args.selected_row_ids.length : 0;
      return {
        kind: "ui.wallets.document_import.import",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          wallet: "Multisig Vault",
          source: { filename: "receipt.png", sha256: "mock" },
          imported: selected,
          skipped: 0,
          unchanged: 0,
          draft_rows_imported: selected,
          quarantined_skipped: 1,
          attached_evidence: [
            {
              transaction_id: "mock-document-import-tx",
              attachment_id: "mock-document-import-attachment",
            },
          ],
        } as T,
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
            network:
              connection.label.toLowerCase().includes("liquid") ||
              connection.label.toLowerCase().includes("l-btc")
                ? "liquidv1"
                : "main",
            backend: {
              name:
                connection.label.toLowerCase().includes("liquid") ||
                connection.label.toLowerCase().includes("l-btc")
                  ? "liquid"
                  : "mempool",
              source: "explicit",
              kind:
                connection.label.toLowerCase().includes("liquid") ||
                connection.label.toLowerCase().includes("l-btc")
                  ? "electrum"
                  : "esplora",
            },
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

    if (req.kind === "ui.wallets.utxos") {
      const overview = mockOverviewSnapshot();
      const args = (req.args ?? {}) as { wallet?: unknown; connection?: unknown };
      const walletRef =
        typeof args.wallet === "string"
          ? args.wallet
          : typeof args.connection === "string"
            ? args.connection
            : "";
      const connection =
        overview.connections.find(
          (item) => item.id === walletRef || item.label === walletRef,
        ) ?? overview.connections[0];
      const payload = JSON.parse(
        JSON.stringify(fixtures["ui.wallets.utxos"]),
      ) as {
        wallet: { id: string; label: string };
        utxos: unknown[];
        totals: unknown[];
        support: {
          supported: boolean;
          status: string;
          reason: string;
          message: string;
        };
        freshness: { status: string; stale: boolean; active_count: number };
        summary: { count: number };
      };
      payload.wallet = {
        id: connection?.id ?? "mock-wallet",
        label: connection?.label ?? "Mock wallet",
      };
      const chainBacked =
        connection?.kind === "xpub" ||
        connection?.kind === "descriptor" ||
        connection?.kind === "address";
      if (!chainBacked) {
        payload.utxos = [];
        payload.totals = [];
        payload.support = {
          supported: false,
          status: "unsupported_source",
          reason: "not_chain_backed",
          message: "This source is not a chain-backed watch-only wallet.",
        };
        payload.freshness = {
          status: "unsupported_source",
          stale: false,
          active_count: 0,
        };
        payload.summary.count = 0;
      } else {
        // Keep the on-chain inventory total consistent with the connection's
        // imported-transaction balance so the Wallet Detail balance
        // reconciliation renders its healthy (reconciled) state by default.
        const balanceSat = Math.round((connection?.balance ?? 0) * 1e8);
        payload.totals = [
          {
            asset: "BTC",
            amount: (connection?.balance ?? 0).toFixed(8),
            amount_sat: balanceSat,
            amount_msat: balanceSat * 1000,
          },
        ];
      }
      return {
        kind: "ui.wallets.utxos",
        schema_version: 1,
        request_id: req.request_id,
        data: payload as T,
      };
    }

    if (
      req.kind === "ui.wallets.identify" ||
      req.kind === "ui.wallets.identify_onchain"
    ) {
      const verified = req.kind === "ui.wallets.identify_onchain";
      const overview = mockOverviewSnapshot();
      const args = (req.args ?? {}) as {
        text?: unknown;
        addresses?: unknown;
        txids?: unknown;
        csv_text?: unknown;
      };
      const tokens: string[] = [];
      if (typeof args.text === "string") {
        tokens.push(...args.text.split(/\r?\n/));
      }
      if (Array.isArray(args.addresses)) {
        tokens.push(...args.addresses.map((value) => String(value)));
      }
      if (Array.isArray(args.txids)) {
        tokens.push(...args.txids.map((value) => String(value)));
      }
      if (typeof args.csv_text === "string") {
        // Smart harvest (mock approximation of the daemon harvester): split into
        // cells and keep only address/txid-looking tokens, ignoring
        // headers/amounts/dates. The mock can't checksum-validate base58, so it
        // bounds the length (26-35) to limit false positives on long ids.
        for (const cell of args.csv_text.split(/[\s,;|\t]+/)) {
          const token = cell.trim();
          if (
            /^[0-9a-fA-F]{64}$/.test(token) ||
            /^(bc1|tb1|bcrt1|lq1|tlq1|ex1|tex1|el1|ert1)/i.test(token) ||
            (/^[13mn2][0-9A-Za-z]{25,34}$/.test(token))
          ) {
            tokens.push(token);
          }
        }
      }
      // De-duplicate in first-seen order to match the real daemon (which dedups
      // in extract_candidates_from_csv and again in parse_tokens).
      const cleaned = Array.from(
        new Set(
          tokens
            .map((token) => token.trim())
            .filter((token) => token.length > 0 && !token.startsWith("#")),
        ),
      );
      const ownerWallet = overview.connections[0]?.label ?? "Cold Storage";
      // Deterministic mock: a 64-hex string is a txid, otherwise an address;
      // tokens containing "own"/"mine"/"demo" demo an owned hit. The cache-only
      // kind mirrors the real read surface (owned txid -> touches_wallet, else
      // unknown); the on-chain kind upgrades txids to a per-leg verdict.
      let receiveIndex = 0;
      const results = cleaned.map((token) => {
        const isTxid = /^[0-9a-fA-F]{64}$/.test(token);
        const owned = /own|mine|demo/i.test(token);
        // A txid can't contain "demo" (non-hex), so use a hex-valid sentinel to
        // demo the owned-txid path (touches_wallet cache-only, self_transfer
        // once verified); any other 64-hex txid stays unknown/external.
        const ownedTxid = isTxid && token.toLowerCase().startsWith("dead");
        if (isTxid) {
          if (ownedTxid) {
            return verified
              ? {
                  input: token,
                  type: "txid",
                  chain: "bitcoin",
                  status: "owned",
                  classification: "self_transfer",
                  wallets: [ownerWallet],
                  owned_inputs: 1,
                  owned_outputs: 2,
                  external_outputs: 0,
                  legs: [
                    { side: "input", outpoint: `${token}:0`, owned: true, wallet: ownerWallet },
                    { side: "output", n: 0, owned: true, wallet: ownerWallet, branch: "change" },
                  ],
                  match_source: "chain",
                  note: "Self-transfer/consolidation: all outputs return to owned addresses.",
                }
              : {
                  input: token,
                  type: "txid",
                  chain: "",
                  status: "owned",
                  classification: "touches_wallet",
                  wallets: [ownerWallet],
                  owned_inputs: null,
                  owned_outputs: null,
                  external_outputs: null,
                  legs: [],
                  match_source: "inventory",
                  note: `Recorded against '${ownerWallet}'; per-leg breakdown needs on-chain verification.`,
                };
          }
          return verified
            ? {
                input: token,
                type: "txid",
                chain: "bitcoin",
                status: "external",
                classification: "external",
                wallets: [],
                owned_inputs: 0,
                owned_outputs: 0,
                external_outputs: 1,
                legs: [],
                match_source: "chain",
                note: "No inputs or outputs belong to this profile.",
              }
            : {
                input: token,
                type: "txid",
                chain: "",
                status: "unknown",
                classification: "unknown",
                wallets: [],
                owned_inputs: null,
                owned_outputs: null,
                external_outputs: null,
                legs: [],
                match_source: "none",
                note: "Not in this profile's synced/imported history; on-chain verification is needed for a verdict.",
              };
        }
        if (owned) {
          const index = receiveIndex++;
          return {
            input: token,
            type: "address",
            chain: "bitcoin",
            status: "owned",
            classification: "owned_address",
            matches: [
              {
                wallet: ownerWallet,
                account: "treasury",
                chain: "bitcoin",
                network: "main",
                branch: "receive",
                address_index: index,
                derivation_path: `m/84'/0'/0'/0/${index}`,
                match_source: "derived",
              },
            ],
            note: `Owned by '${ownerWallet}' (receive #${index}).`,
          };
        }
        return {
          input: token,
          type: "address",
          chain: "bitcoin",
          status: "external",
          classification: "external_address",
          matches: [],
          note: "Not derived from or seen by any wallet in this profile.",
        };
      });
      const counts = { owned: 0, external: 0, unknown: 0, invalid: 0 };
      for (const result of results) {
        if (result.status in counts) {
          counts[result.status as keyof typeof counts] += 1;
        }
      }
      return {
        kind: req.kind,
        schema_version: 1,
        request_id: req.request_id,
        data: {
          results,
          summary: {
            total: results.length,
            ...counts,
            wallets_scanned: overview.connections.length,
            scan_to_index: 500,
            verified_on_chain: verified,
          },
          warnings: [],
          context: { workspace: "Demo", profile: "Default" },
        } as T,
      };
    }

    if (req.kind === "ui.reports.balance_history") {
      const overview = mockOverviewSnapshot();
      const args = (req.args ?? {}) as {
        wallet?: unknown;
        interval?: unknown;
        limit?: unknown;
      };
      const walletRef = typeof args.wallet === "string" ? args.wallet : "";
      const connection = walletRef
        ? overview.connections.find(
            (item) => item.id === walletRef || item.label === walletRef,
          )
        : undefined;
      // Wallet-scoped history ramps up to the connection's current balance so
      // the Wallet Detail sparkline reflects that wallet rather than the book
      // total. Unscoped requests fall back to the static fixture.
      if (walletRef && connection) {
        const target = connection.balance ?? 0;
        const months: Array<[number, number]> = [];
        let year = 2025;
        let month = 7;
        for (let i = 0; i < 12; i += 1) {
          months.push([year, month]);
          month += 1;
          if (month > 12) {
            month = 1;
            year += 1;
          }
        }
        const rows = months.map(([y, m], index) => {
          const progress = (index + 1) / months.length;
          const wobble = 0.82 + 0.36 * ((index % 3) / 3);
          const quantity =
            index === months.length - 1
              ? target
              : Number((target * progress * wobble).toFixed(8));
          // Mirror the real report_balance_history row shape (period_start, not
          // a `bucket` field) so the preview exercises the production code path.
          return {
            period_start: `${y}-${String(m).padStart(2, "0")}-01T00:00:00Z`,
            asset: "BTC",
            quantity,
          };
        });
        return {
          kind: "ui.reports.balance_history",
          schema_version: 1,
          request_id: req.request_id,
          data: {
            rows,
            filters: { interval: "month", limit: 120, wallet: walletRef },
            summary: {
              row_count: rows.length,
              total_row_count: rows.length,
              truncated: false,
            },
          } as T,
        };
      }
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

    if (req.kind === "ui.secrets.forget_cli_unlock") {
      return {
        kind: "ui.secrets.forget_cli_unlock",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          cli_marker_cleared: true,
          cli_credential_deleted: true,
          legacy_credential_deleted: true,
          remembered_unlock: {
            platform: "unsupported",
            access_policy: "unsupported",
            available: false,
            configured: false,
            cli_enabled: false,
            legacy_quarantined: false,
          },
        } as T,
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

    if (req.kind === "ui.profiles.reset_data") {
      const clearSharedRates = req.args?.clear_shared_rates === true;
      return {
        kind: "ui.profiles.reset_data",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          reset: true,
          workspace: { id: "mock-workspace", label: "My Books" },
          profile: { id: "mock-profile", label: "local books" },
          preserved: {
            workspaces: 1,
            profiles: 1,
            accounts: 2,
            wallets: 4,
            backends: 2,
            rates_cache: clearSharedRates ? 0 : 128,
            rates_checked_minutes: clearSharedRates ? 0 : 128,
          },
          rates_scope: clearSharedRates ? "global" : "preserved",
          shared_rates_cleared: clearSharedRates,
          removed: {
            transaction_tags: 5,
            transactions: 24,
            journal_entries: 24,
            journal_quarantines: 0,
            transaction_pairs: 3,
            transaction_pair_dismissals: 1,
            swap_matching_rules: 1,
            saved_views: 2,
            bip329_labels: 8,
            attachments: 2,
            tags: 5,
            source_funds_sources: 1,
            source_funds_links: 1,
            source_funds_link_attachments: 1,
            source_funds_source_attachments: 1,
            source_funds_cases: 1,
            source_funds_snapshots: 1,
            source_funds_recipients: 1,
            rates_cache: clearSharedRates ? 128 : 0,
            rates_checked_minutes: clearSharedRates ? 128 : 0,
            attachment_files: 2,
          },
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

    if (req.kind === "ui.workspace.overview.snapshot") {
      const args = (req.args ?? {}) as { workspace_id?: unknown };
      const workspaceId =
        typeof args.workspace_id === "string" && args.workspace_id.trim()
          ? args.workspace_id.trim()
          : mockProfilesSnapshot.activeWorkspaceId;
      return {
        kind: "ui.workspace.overview.snapshot",
        schema_version: 1,
        request_id: req.request_id,
        data: mockWorkspaceOverviewSnapshot(workspaceId) as T,
      };
    }

    if (req.kind === "ui.workspace.freshness.run") {
      const args = (req.args ?? {}) as { workspace_id?: unknown };
      const workspaceId =
        typeof args.workspace_id === "string" && args.workspace_id.trim()
          ? args.workspace_id.trim()
          : mockProfilesSnapshot.activeWorkspaceId;
      const overview = mockWorkspaceOverviewSnapshot(workspaceId);
      const books = overview.books.map((book) => ({
        profile: { id: book.profile.id, label: book.profile.label },
        results: book.connections.map((connection) => ({
          wallet: connection.label,
          status: "synced",
          inserted: 0,
          updated: 0,
        })),
        recovered: [],
        enqueued: [],
        completed: [
          {
            job_type: "journal_refresh",
            source_label: "Journals",
            source_type: "journals",
            status: book.readiness.ready ? "done" : "rate_limited",
          },
        ],
        attention: {
          blockedReports: !book.readiness.ready,
          rateLimited: !book.readiness.ready,
          errors: 0,
        },
        sources: [],
        jobs: [],
        summary: {
          sources: book.connections.length,
          active_jobs: 0,
          blocking_reports: book.readiness.ready ? 0 : 1,
          rate_limited: book.readiness.ready ? 0 : 1,
        },
      }));
      return {
        kind: "ui.workspace.freshness.run",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          workspace: overview.workspace,
          books,
          summary: {
            books: books.length,
            enqueued: 0,
            completed: books.length,
            errors: 0,
            rate_limited: books.filter((book) => book.attention.rateLimited).length,
            blocked_books: books.filter((book) => book.attention.blockedReports).length,
            synced_books: books.filter((book) => !book.attention.blockedReports).length,
            ok: books.every((book) => !book.attention.blockedReports),
            reports_blocked: books.filter((book) => book.attention.blockedReports).length,
          },
        } as T,
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

    if (req.kind === "ui.onboarding.complete") {
      const args = (req.args ?? {}) as {
        workspace_label?: unknown;
        profile_label?: unknown;
        tax_country?: unknown;
        fiat_currency?: unknown;
        tax_long_term_days?: unknown;
        gains_algorithm?: unknown;
      };
      const workspaceName =
        typeof args.workspace_label === "string"
          ? args.workspace_label.trim()
          : "";
      const profileName =
        typeof args.profile_label === "string" ? args.profile_label.trim() : "";
      if (!workspaceName || !profileName) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Books set and book name are required.",
            retryable: false,
          },
        };
      }
      const taxCountry: ProfileTaxCountry =
        args.tax_country === "at" ? "at" : "generic";
      const fiatCurrency =
        typeof args.fiat_currency === "string" ? args.fiat_currency : "EUR";
      const taxLongTermDays =
        typeof args.tax_long_term_days === "number"
          ? args.tax_long_term_days
          : taxCountry === "at"
            ? 0
            : 365;
      const rawGainsAlgorithm =
        typeof args.gains_algorithm === "string" ? args.gains_algorithm : "";
      const gainsAlgorithm: ProfileGainsAlgorithm =
        rawGainsAlgorithm === "FIFO" ||
        rawGainsAlgorithm === "LIFO" ||
        rawGainsAlgorithm === "HIFO" ||
        rawGainsAlgorithm === "LOFO" ||
        rawGainsAlgorithm === "MOVING_AVERAGE" ||
        rawGainsAlgorithm === "MOVING_AVERAGE_AT"
          ? rawGainsAlgorithm
          : taxCountry === "at"
            ? "MOVING_AVERAGE_AT"
            : "FIFO";
      const workspace: Workspace = {
        id: `mock-workspace-${Date.now()}`,
        name: workspaceName,
        currency: fiatCurrency,
        jurisdiction: taxCountry === "at" ? "Austria" : "Generic",
        created: new Date().toISOString().slice(0, 10),
        profiles: [
          {
            id: `mock-profile-${Date.now()}`,
            name: profileName,
            taxPolicy:
              taxCountry === "at"
                ? mockTaxPolicyLabel("at", gainsAlgorithm, fiatCurrency)
                : "Generic defaults",
            fiatCurrency,
            taxCountry,
            taxLongTermDays,
            gainsAlgorithm,
            accounts: 1,
            wallets: 0,
            lastOpened: "Just now",
            active: true,
          },
        ],
      };
      mockProfilesSnapshot = {
        activeWorkspaceId: workspace.id,
        activeProfileId: workspace.profiles[0].id,
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
        kind: "ui.onboarding.complete",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          workspace: { id: workspace.id, name: workspace.name },
          profile: {
            id: workspace.profiles[0].id,
            name: workspace.profiles[0].name,
          },
          profiles: mockProfilesSnapshot,
        } as T,
      };
    }

    if (req.kind === "ui.profiles.create") {
      const args = (req.args ?? {}) as {
        workspace_id?: unknown;
        label?: unknown;
        source_profile_id?: unknown;
        tax_country?: unknown;
        gains_algorithm?: unknown;
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
      // The "New book" dialog can pick a region + method explicitly; copying from
      // a source inherits its settings verbatim instead. Mirror the real daemon.
      const requestedCountry: ProfileTaxCountry | null =
        args.tax_country === "at"
          ? "at"
          : args.tax_country === "generic"
            ? "generic"
            : null;
      const requestedAlgorithm =
        typeof args.gains_algorithm === "string" && args.gains_algorithm.trim()
          ? (args.gains_algorithm.trim() as ProfileGainsAlgorithm)
          : null;
      const baseCountry: ProfileTaxCountry =
        sourceProfile?.taxCountry ??
        firstProfile?.taxCountry ??
        (workspace.jurisdiction === "Austria" ? "at" : "generic");
      const nextCountry: ProfileTaxCountry = sourceProfile
        ? baseCountry
        : (requestedCountry ?? baseCountry);
      const baseAlgorithm: ProfileGainsAlgorithm =
        sourceProfile?.gainsAlgorithm ??
        firstProfile?.gainsAlgorithm ??
        (baseCountry === "at" ? "MOVING_AVERAGE_AT" : "FIFO");
      // Austrian books default to moving-average but accept any requested
      // method (no coercion); copy-from-source inherits the source's method.
      const nextAlgorithm: ProfileGainsAlgorithm = sourceProfile
        ? baseAlgorithm
        : (requestedAlgorithm ??
          (nextCountry === "at" ? "MOVING_AVERAGE_AT" : baseAlgorithm));
      const nextFiat =
        nextCountry === "at"
          ? "EUR"
          : (sourceProfile?.fiatCurrency ??
            firstProfile?.fiatCurrency ??
            workspace.currency);
      const nextLongTermDays =
        nextCountry === "at"
          ? 0
          : !sourceProfile && nextCountry !== baseCountry
            ? 365
            : (sourceProfile?.taxLongTermDays ??
              firstProfile?.taxLongTermDays ??
              365);
      const profile = {
        id: `mock-profile-${Date.now()}`,
        name: label,
        taxPolicy: mockTaxPolicyLabel(nextCountry, nextAlgorithm, nextFiat),
        fiatCurrency: nextFiat,
        taxCountry: nextCountry,
        taxLongTermDays: nextLongTermDays,
        gainsAlgorithm: nextAlgorithm,
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

    if (req.kind === "ui.profiles.update") {
      const args = (req.args ?? {}) as {
        profile_id?: unknown;
        gains_algorithm?: unknown;
        tax_country?: unknown;
      };
      const profileId =
        typeof args.profile_id === "string" ? args.profile_id : "";
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
      const requested =
        typeof args.gains_algorithm === "string"
          ? args.gains_algorithm.trim()
          : "";
      // Mirror the real daemon's validation (daemon.py: "Accounting method is
      // required."); the method-change dialog always sends a non-empty method.
      if (!requested) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Accounting method is required.",
            retryable: false,
          },
        };
      }
      // Region is optional: the book-settings dialog only sends it on an
      // explicit switch, always paired with a region-valid method.
      const nextCountry: ProfileTaxCountry =
        args.tax_country === "at"
          ? "at"
          : args.tax_country === "generic"
            ? "generic"
            : (profile.taxCountry ?? "generic");
      // No per-country coercion: Austrian books accept any requested method
      // (moving-average remains the default the dialog offers).
      const nextAlgorithm = requested as ProfileGainsAlgorithm;
      const nextTaxPolicy = mockTaxPolicyLabel(nextCountry, nextAlgorithm, "EUR");
      mockProfilesSnapshot = {
        ...mockProfilesSnapshot,
        workspaces: mockProfilesSnapshot.workspaces.map((candidate) => ({
          ...candidate,
          profiles: candidate.profiles.map((existing) =>
            existing.id === profileId
              ? {
                  ...existing,
                  gainsAlgorithm: nextAlgorithm,
                  taxCountry: nextCountry,
                  taxPolicy: nextTaxPolicy,
                }
              : existing,
          ),
        })),
      };
      return {
        kind: "ui.profiles.update",
        schema_version: 1,
        request_id: req.request_id,
        data: { id: profileId } as T,
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
        gap_limit?: unknown;
        backend?: unknown;
        clear?: unknown;
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
          args.source_file.trim().length > 0) ||
        typeof args.gap_limit === "number" ||
        (typeof args.backend === "string" && args.backend.trim().length > 0) ||
        (Array.isArray(args.clear) && args.clear.length > 0);
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
      if (
        typeof args.gap_limit === "number" &&
        (args.gap_limit <= 0 || args.gap_limit > MAX_DESCRIPTOR_GAP_LIMIT)
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: `gap_limit must be between 1 and ${MAX_DESCRIPTOR_GAP_LIMIT}`,
            retryable: false,
          },
        };
      }
      if (label) connection.label = label;
      if (typeof args.gap_limit === "number") connection.gap = args.gap_limit;
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
        gap_limit?: unknown;
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
      if (
        typeof args.gap_limit === "number" &&
        (args.gap_limit <= 0 || args.gap_limit > MAX_DESCRIPTOR_GAP_LIMIT)
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: `gap_limit must be between 1 and ${MAX_DESCRIPTOR_GAP_LIMIT}`,
            retryable: false,
          },
        };
      }
      const overview = mockOverviewSnapshot();
      // A descriptor / xpub wallet (incl. multi-script xpub) is a backend-synced
      // wallet: surface its gap limit + descriptor sync mode, and start at zero
      // transactions so the empty recent-tx state and the count badge agree.
      const isDescriptorKind = kind === "descriptor" || kind === "xpub";
      const connection = {
        id: `mock-wallet-${Date.now()}`,
        label,
        kind,
        last: "just now",
        balance: 0,
        status: "idle",
        transactionCount: 0,
        ...(typeof args.gap_limit === "number"
          ? { gap: args.gap_limit }
          : isDescriptorKind
            ? { gap: 40 }
            : {}),
        ...(sourceFormat
          ? {
              syncMode: "file_import",
              syncSource: sourceFormat,
              sourceFormat,
            }
          : isDescriptorKind
            ? { syncMode: "backend_descriptor" }
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
        paymentMethodId,
        chain: paymentMethodId.includes("LBTC") ? "liquid" : "bitcoin",
        network: paymentMethodId.includes("LBTC") ? "liquidv1" : "main",
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

    if (req.kind === "ui.connections.bullbitcoin_wallet.create") {
      const args = (req.args ?? {}) as {
        label?: unknown;
        source_file?: unknown;
        networks?: unknown;
        mode?: unknown;
        routes?: unknown;
      };
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const sourceFile =
        typeof args.source_file === "string" ? args.source_file.trim() : "";
      if (!label || !sourceFile) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Bull Bitcoin wallet setup requires a label and export file",
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
          kind: "ui.connections.bullbitcoin_wallet.create",
          schema_version: 1,
          request_id: req.request_id,
          data: {
            mode: "existing_wallets",
            wallet: mapped[0],
            wallets: mapped,
            routes,
          } as T,
        };
      }
      const rawNetworks = Array.isArray(args.networks)
        ? args.networks
        : ["bitcoin", "liquid", "lightning"];
      const networks = rawNetworks
        .filter((network): network is string =>
          typeof network === "string" && Boolean(network.trim()),
        )
        .map((network) => network.trim());
      const connections = networks.map((network, index) => ({
        id: `mock-bull-wallet-${Date.now()}-${index}`,
        label:
          networks.length === 1
            ? label
            : `${label} - ${network[0].toUpperCase()}${network.slice(1)}`,
        kind: "bullbitcoin",
        syncMode: "file_import",
        syncSource: "bullbitcoin_wallet_csv",
        chain: network === "liquid" ? "liquid" : "bitcoin",
        network: network === "liquid" ? "liquidv1" : network,
      }));
      overview.connections = [...overview.connections, ...connections];
      return {
        kind: "ui.connections.bullbitcoin_wallet.create",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          mode: "wallet_sources",
          wallet: connections[0],
          wallets: connections,
          networks,
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

    if (req.kind === "ui.metadata.bip329.preview") {
      return {
        kind: "ui.metadata.bip329.preview",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          file: String((req.args as { file?: string } | undefined)?.file ?? ""),
          records: 3,
          counts: {
            exact: 1,
            ambiguous: 1,
            unmatched: 1,
            preserved: 0,
            conflicts: 0,
            duplicate_refs: 0,
            duplicate_records: 0,
            tag_additions: 1,
            tag_unchanged: 0,
            tag_skipped_ambiguous: 1,
            tag_skipped_duplicate: 0,
            tag_skipped_label_too_long: 0,
          },
          warnings: [],
          apply_policy: "exact_only",
          rows: [
            {
              line: 1,
              type: "tx",
              ref: "mock-txid",
              ref_preview: "mock-txid",
              ref_redacted: false,
              label: "merchant",
              match_status: "exact",
              wallets: ["Treasury"],
              conflicts: [],
              duplicate: false,
              tag_effects: [{ action: "add" }],
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
          preview: {
            counts: {
              exact: 1,
              ambiguous: 0,
              unmatched: 0,
              preserved: 0,
              conflicts: 0,
              duplicate_refs: 0,
              duplicate_records: 0,
              tag_additions: 1,
              tag_unchanged: 0,
              tag_skipped_ambiguous: 0,
              tag_skipped_duplicate: 0,
              tag_skipped_label_too_long: 0,
            },
            apply_policy: "exact_only",
          },
        } as T,
      };
    }

    if (req.kind === "ui.metadata.bip329.export") {
      return {
        kind: "ui.metadata.bip329.export",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          file: "/mock/exports/kassiber-bip329-labels.jsonl",
          filename: "kassiber-bip329-labels.jsonl",
          exported: 3,
          exported_stored: 2,
          exported_synthesized: 1,
          mode: String((req.args as { mode?: string } | undefined)?.mode ?? "stored"),
          wallet: String((req.args as { wallet?: string } | undefined)?.wallet ?? ""),
          format: "jsonl",
          scope: "bip329",
        } as T,
      };
    }

    if (req.kind === "ui.transactions.metadata.update") {
      const args = (req.args ?? {}) as {
        transaction?: unknown;
        note?: unknown;
        tags?: unknown;
        excluded?: unknown;
        fiat_currency?: unknown;
        fiat_rate?: unknown;
        fiat_value?: unknown;
        pricing_source_kind?: unknown;
        pricing_quality?: unknown;
        pricing_external_ref?: unknown;
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
      if ("fiat_currency" in args) {
        tx.fiatCurrency =
          typeof args.fiat_currency === "string" ? args.fiat_currency : null;
      }
      if ("fiat_rate" in args) {
        tx.rate =
          typeof args.fiat_rate === "string"
            ? Number(args.fiat_rate)
            : typeof args.fiat_rate === "number"
              ? args.fiat_rate
              : null;
      }
      if ("fiat_value" in args) {
        tx.eur =
          typeof args.fiat_value === "string"
            ? Number(args.fiat_value)
            : typeof args.fiat_value === "number"
              ? args.fiat_value
              : null;
      }
      if ("pricing_source_kind" in args) {
        tx.pricingSourceKind =
          typeof args.pricing_source_kind === "string"
            ? args.pricing_source_kind
            : null;
      }
      if ("pricing_quality" in args) {
        tx.pricingQuality =
          typeof args.pricing_quality === "string" ? args.pricing_quality : null;
      }
      if ("pricing_external_ref" in args) {
        tx.pricingExternalRef =
          typeof args.pricing_external_ref === "string"
            ? args.pricing_external_ref
            : null;
      }
      if ("review_status" in args) {
        tx.reviewStatus =
          typeof args.review_status === "string" ? args.review_status : null;
      }
      if ("taxable" in args) {
        tx.taxable = typeof args.taxable === "boolean" ? args.taxable : null;
      }
      if ("at_regime" in args) {
        tx.atRegime =
          typeof args.at_regime === "string" ? args.at_regime : null;
      }
      if ("at_category" in args) {
        tx.atCategory =
          typeof args.at_category === "string" ? args.at_category : null;
      }
      return {
        kind: "ui.transactions.metadata.update",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          transaction_id: transactionId,
          note: typeof args.note === "string" ? args.note : "",
          tags: (tags ?? []).map((tag) => ({ code: tag.toLowerCase(), label: tag })),
          excluded: args.excluded === true,
          fiat_currency: tx.fiatCurrency ?? null,
          fiat_rate: tx.rate ?? null,
          fiat_value: tx.eur ?? null,
          pricing_source_kind: tx.pricingSourceKind ?? null,
          pricing_quality: tx.pricingQuality ?? null,
          pricing_external_ref: tx.pricingExternalRef ?? null,
          review_status: tx.reviewStatus ?? null,
          taxable: tx.taxable ?? null,
          at_regime: tx.atRegime ?? null,
          at_category: tx.atCategory ?? null,
          updated: true,
        } as T,
      };
    }

    if (req.kind === "ui.transactions.commercial_context") {
      const args = (req.args ?? {}) as { transaction?: unknown };
      const transactionId = typeof args.transaction === "string" ? args.transaction : "";
      return {
        kind: "ui.transactions.commercial_context",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          transaction_id: transactionId,
          transaction_external_id: transactionId === "tx1" ? "mock-btcpay-txid" : "",
          links:
            transactionId === "tx1"
              ? [
                  {
                    id: "commercial-link-1",
                    invoice_id: "inv-demo-1",
                    payment_id: "pay-demo-1",
                    document_id: "doc-demo-1",
                    document_label: "Invoice demo-1",
                    link_type: "btcpay_payment_transaction",
                    state: "reviewed",
                    confidence: "exact",
                    reconciliation_state: "matched",
                    commercial_kind: "income",
                    reviewed_at: "2026-04-18T14:24:00Z",
                  },
                ]
              : [],
          btcpay:
            transactionId === "tx1"
              ? [
                  {
                    link: {
                      id: "commercial-link-1",
                      invoice_id: "inv-demo-1",
                      payment_id: "pay-demo-1",
                      document_id: "doc-demo-1",
                      document_label: "Invoice demo-1",
                      link_type: "btcpay_payment_transaction",
                      state: "reviewed",
                      confidence: "exact",
                      reconciliation_state: "matched",
                      commercial_kind: "income",
                      reviewed_at: "2026-04-18T14:24:00Z",
                    },
                    payment: {
                      id: "btcpay-payment-1",
                      record_type: "payment",
                      invoice_id: "inv-demo-1",
                      payment_id: "pay-demo-1",
                      order_id: "order-demo-1",
                      status: "Settled",
                      occurred_at: "2026-04-18T14:22:00Z",
                      asset: "BTC",
                      amount_msat: 125000000,
                      amount: 0.00125,
                      payment_request_id: "pr-demo-1",
                      origin_kind: "pos",
                      origin_app_id: "",
                      origin_label: "Demo checkout",
                      origin_url: "https://btcpay.example/apps/pos/demo",
                      fiat_currency: "EUR",
                      fiat_value_exact: "75.00",
                      fiat_rate_exact: "60000.00",
                      pricing_timestamp: "2026-04-18T14:22:00Z",
                      updated_at: "2026-04-18T14:24:00Z",
                    },
                    invoice: {
                      id: "btcpay-invoice-1",
                      record_type: "invoice",
                      invoice_id: "inv-demo-1",
                      payment_id: "",
                      order_id: "order-demo-1",
                      status: "Settled",
                      occurred_at: "2026-04-18T14:20:00Z",
                      asset: "",
                      amount_msat: null,
                      amount: null,
                      payment_request_id: "pr-demo-1",
                      origin_kind: "pos",
                      origin_app_id: "",
                      origin_label: "Demo checkout",
                      origin_url: "https://btcpay.example/apps/pos/demo",
                      fiat_currency: "EUR",
                      fiat_value_exact: "75.00",
                      fiat_rate_exact: "",
                      pricing_timestamp: "2026-04-18T14:20:00Z",
                      updated_at: "2026-04-18T14:24:00Z",
                    },
                    payment_request: {
                      id: "pr-demo-1",
                      label: "Demo checkout",
                      status: "Settled",
                      url: "https://btcpay.example/apps/pos/demo",
                    },
                    origin: {
                      kind: "pos",
                      app_id: "",
                      label: "Demo checkout",
                      url: "https://btcpay.example/apps/pos/demo",
                    },
                  },
                ]
              : [],
          documents:
            transactionId === "tx1"
              ? [
                  {
                    id: "doc-demo-1",
                    document_type: "invoice",
                    label: "Invoice demo-1",
                    external_ref: "inv-demo-1",
                    review_state: "reviewed",
                  },
                ]
              : [],
        } as T,
      };
    }

    if (req.kind === "ui.transactions.history" || req.kind === "ui.activity.history") {
      const args = (req.args ?? {}) as Record<string, unknown>;
      const transaction =
        typeof args.transaction === "string" ? args.transaction : "";
      const source = typeof args.source === "string" ? args.source : "";
      const family =
        typeof args.field_family === "string" ? args.field_family : "";
      const wallet = typeof args.wallet === "string" ? args.wallet : "";
      const pricingOnly = args.pricing_only === true;
      const aiOnly = args.ai_only === true;
      const staleOnly = args.stale_only === true;
      const includeStale = args.include_stale !== false;
      const events = mockTransactionHistory.filter((event) => {
        if (transaction && event.transaction_id !== transaction && event.transaction_external_id !== transaction) {
          return false;
        }
        if (source && event.source !== source) return false;
        if (aiOnly && event.source !== "ai_tool") return false;
        if (wallet && event.wallet_label !== wallet && event.wallet_id !== wallet) return false;
        if (family && !event.families.includes(family)) return false;
        if (pricingOnly && !event.families.includes("pricing")) return false;
        if (staleOnly && !event.report_anchor?.stale_for_reports) return false;
        return true;
      });
      return {
        kind: req.kind,
        schema_version: 1,
        request_id: req.request_id,
        data: {
          events,
          next_cursor: null,
          has_more: false,
          limit: typeof args.limit === "number" ? args.limit : 50,
          ...(includeStale
            ? {
                stale: {
                  edit_count: mockTransactionHistory.filter(
                    (event) => event.report_anchor?.stale_for_reports,
                  ).length,
                  latest_changed_at: mockTransactionHistory[0]?.changed_at ?? null,
                  source_counts: { ai_tool: 1, gui: 1 },
                  family_counts: { metadata: 1, pricing: 1, tax: 2 },
                  field_counts: {
                    pricing_external_ref: 1,
                    review_status: 1,
                    tags: 1,
                    taxable: 1,
                  },
                  last_processed_at: "2026-04-17T22:00:00Z",
                  last_processed_input_version: 6,
                },
              }
            : {}),
        } as T,
      };
    }

    if (req.kind === "ui.activity.stale") {
      return {
        kind: "ui.activity.stale",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          edit_count: mockTransactionHistory.filter(
            (event) => event.report_anchor?.stale_for_reports,
          ).length,
          latest_changed_at: mockTransactionHistory[0]?.changed_at ?? null,
          source_counts: { ai_tool: 1, gui: 1 },
          family_counts: { metadata: 1, pricing: 1, tax: 2 },
          field_counts: {
            pricing_external_ref: 1,
            review_status: 1,
            tags: 1,
            taxable: 1,
          },
          last_processed_at: "2026-04-17T22:00:00Z",
          last_processed_input_version: 6,
        } as T,
      };
    }

    if (req.kind === "ui.transactions.history.revert") {
      const args = (req.args ?? {}) as Record<string, unknown>;
      const transactionId = typeof args.transaction === "string" ? args.transaction : "tx1";
      const eventId = typeof args.event === "string" ? args.event : "";
      const fieldName = typeof args.field === "string" ? args.field : "";
      const sourceEvent =
        mockTransactionHistory.find((event) => event.id === eventId) ??
        mockTransactionHistory.find((event) => event.transaction_id === transactionId);
      const sourceField = fieldName
        ? sourceEvent?.fields.find((field) => field.field === fieldName)
        : undefined;
      const fields = sourceField ? [sourceField] : sourceEvent?.fields ?? [];
      const revertedFields = fields.map((field) => field.field);
      const newEvent = {
        id: `edit-mock-revert-${Date.now()}`,
        transaction_id: transactionId,
        transaction_external_id: transactionId,
        transaction_occurred_at: sourceEvent?.transaction_occurred_at ?? "",
        wallet_id: sourceEvent?.wallet_id ?? "",
        wallet_label: sourceEvent?.wallet_label ?? "",
        source: "gui",
        source_label: "Desktop",
        reason: typeof args.reason === "string" ? args.reason : "Reverted edit history event",
        changed_at: new Date().toISOString(),
        summary: sourceField ? `Updated ${sourceField.label}` : "Reverted edit history event",
        families: Array.from(new Set(fields.map((field) => field.family))),
        report_anchor: { stale_for_reports: true, journal_input_version_after: 9 },
        transaction: sourceEvent?.transaction ?? { id: transactionId },
        fields: fields.map((field) => ({
          ...field,
          id: `${field.id}-revert`,
          before_value: field.after_value,
          after_value: field.before_value,
          before_label: field.after_label,
          after_label: field.before_label,
        })),
      };
      mockTransactionHistory = [newEvent, ...mockTransactionHistory];
      return {
        kind: "ui.transactions.history.revert",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          updated: true,
          reverted_event_id: eventId,
          history_event_id: newEvent.id,
          reverted_fields: revertedFields,
          transaction: { transaction_id: transactionId },
        } as T,
      };
    }

    if (req.kind === "ui.attachments.list") {
      const args = (req.args ?? {}) as { transaction?: unknown };
      const tx = typeof args.transaction === "string" ? args.transaction : "";
      return {
        kind: "ui.attachments.list",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          attachments: mockAttachments.filter(
            (attachment) => !tx || attachment.transaction_id === tx,
          ),
        } as T,
      };
    }

    if (req.kind === "ui.attachments.add") {
      const args = (req.args ?? {}) as {
        transaction?: unknown;
        file_path?: unknown;
        url?: unknown;
        label?: unknown;
      };
      const transactionId =
        typeof args.transaction === "string" ? args.transaction : "";
      const isUrl = typeof args.url === "string" && args.url.length > 0;
      const source = isUrl
        ? args.url as string
        : typeof args.file_path === "string"
          ? args.file_path
          : "";
      if (!transactionId || !source) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "ui.attachments.add requires transaction and file_path or url",
            retryable: false,
          },
        };
      }
      const label =
        typeof args.label === "string" && args.label.trim()
          ? args.label.trim()
          : isUrl
            ? null
            : source.split(/[\\/]/).pop() || "attachment.bin";
      if (label && label.length > MAX_ATTACHMENT_LABEL_LENGTH) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: `Attachment label must be ${MAX_ATTACHMENT_LABEL_LENGTH} characters or fewer`,
            retryable: false,
          },
        };
      }
      const attachment: MockAttachment = {
        id: `att-mock-${(mockAttachmentCounter += 1)}`,
        transaction_id: transactionId,
        attachment_type: isUrl ? "url" : "file",
        label,
        display_label: isUrl
          ? label || mockUrlDisplayLabel(source)
          : label || "attachment.bin",
        original_filename: isUrl ? undefined : label || "attachment.bin",
        url: isUrl ? source : undefined,
        media_type: isUrl ? "text/uri-list" : "application/octet-stream",
        size_bytes: isUrl ? null : 1024,
        sha256: isUrl ? "" : "mock",
        stored_relpath: isUrl ? "" : `mock/${label || "attachment.bin"}`,
        exists: isUrl ? null : true,
        created_at: new Date().toISOString(),
      };
      mockAttachments = [attachment, ...mockAttachments];
      return {
        kind: "ui.attachments.add",
        schema_version: 1,
        request_id: req.request_id,
        data: attachment as T,
      };
    }

    if (req.kind === "ui.attachments.copy") {
      const args = (req.args ?? {}) as {
        transaction?: unknown;
        target_transaction?: unknown;
        source_transaction?: unknown;
        attachments?: unknown;
        attachment_ids?: unknown;
      };
      const transactionId =
        typeof args.transaction === "string"
          ? args.transaction
          : typeof args.target_transaction === "string"
            ? args.target_transaction
            : "";
      const sourceTransactionId =
        typeof args.source_transaction === "string"
          ? args.source_transaction
          : "";
      const attachmentIds = Array.isArray(args.attachments)
        ? args.attachments
        : Array.isArray(args.attachment_ids)
          ? args.attachment_ids
          : [];
      const sourceAttachments = attachmentIds
        .filter((id): id is string => typeof id === "string")
        .map((id) => mockAttachments.find((attachment) => attachment.id === id))
        .filter((attachment): attachment is MockAttachment => Boolean(attachment));
      const copied = sourceAttachments.map((attachment) => {
        const id = `att-mock-${(mockAttachmentCounter += 1)}`;
        return {
          ...attachment,
          id,
          transaction_id: transactionId,
          stored_relpath:
            attachment.attachment_type === "file"
              ? `mock/${id}-${attachment.original_filename || attachment.label}`
              : "",
          copied_from_attachment_id: attachment.id,
          copied_from_transaction_id:
            sourceTransactionId || attachment.transaction_id,
          created_at: new Date().toISOString(),
        };
      });
      mockAttachments = [...copied, ...mockAttachments];
      return {
        kind: "ui.attachments.copy",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          copied: copied.length,
          attachments: copied,
          source_transaction_id: sourceTransactionId,
          target_transaction_id: transactionId,
        } as T,
      };
    }

    if (req.kind === "ui.attachments.rename") {
      const args = (req.args ?? {}) as {
        attachment?: unknown;
        attachment_id?: unknown;
        label?: unknown;
      };
      const attachmentId =
        typeof args.attachment === "string"
          ? args.attachment
          : typeof args.attachment_id === "string"
            ? args.attachment_id
            : "";
      const label = typeof args.label === "string" ? args.label.trim() : "";
      const attachment = mockAttachments.find((item) => item.id === attachmentId);
      if (
        !attachment ||
        !label ||
        label.length > MAX_ATTACHMENT_LABEL_LENGTH ||
        attachment.attachment_type !== "url"
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: attachment ? "validation" : "not_found",
            message: !attachment
              ? `Attachment '${attachmentId}' not found`
              : attachment.attachment_type !== "url"
                ? "Only URL attachment link text can be renamed"
                : label.length > MAX_ATTACHMENT_LABEL_LENGTH
                  ? `Attachment label must be ${MAX_ATTACHMENT_LABEL_LENGTH} characters or fewer`
                  : "ui.attachments.rename requires label",
            retryable: false,
          },
        };
      }
      attachment.label = label;
      attachment.display_label = mockAttachmentDisplayLabel(attachment);
      return {
        kind: "ui.attachments.rename",
        schema_version: 1,
        request_id: req.request_id,
        data: attachment as T,
      };
    }

    if (req.kind === "ui.attachments.remove") {
      const args = (req.args ?? {}) as { attachment?: unknown };
      const attachmentId =
        typeof args.attachment === "string" ? args.attachment : "";
      const attachment = mockAttachments.find((item) => item.id === attachmentId);
      mockAttachments = mockAttachments.filter((item) => item.id !== attachmentId);
      return {
        kind: "ui.attachments.remove",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          ...(attachment ?? { id: attachmentId }),
          removed: Boolean(attachment),
        } as T,
      };
    }

    if (req.kind === "ui.attachments.open") {
      const args = (req.args ?? {}) as { attachment?: unknown };
      const attachmentId =
        typeof args.attachment === "string" ? args.attachment : "";
      const attachment = mockAttachments.find((item) => item.id === attachmentId);
      if (!attachment) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Attachment '${attachmentId}' not found`,
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.attachments.open",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          target_type: attachment.attachment_type === "url" ? "url" : "file",
          url: attachment.url,
          path: attachment.attachment_type === "file" ? `/tmp/${attachment.label}` : undefined,
          attachment,
        } as T,
      };
    }

    if (req.kind === "ui.source_funds.links.list") {
      return {
        kind: "ui.source_funds.links.list",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          links: mockAuditEvidenceSummary(String(req.args?.target_transaction ?? "tx1"))
            .transactions[0].source_funds_links,
        } as T,
      };
    }

    if (req.kind === "ui.source_funds.cases.list") {
      return {
        kind: "ui.source_funds.cases.list",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          cases: [
            {
              id: "mock-source-funds-case",
              label: "Exchange sale handoff",
              target_external_id: "tx1",
              status: "blocked",
              created_at: "2026-04-18T15:00:00Z",
            },
          ],
        } as T,
      };
    }

    if (req.kind === "ui.audit.evidence.summary") {
      const transaction =
        typeof req.args?.transaction === "string" ? req.args.transaction : "tx1";
      return {
        kind: "ui.audit.evidence.summary",
        schema_version: 1,
        request_id: req.request_id,
        data: mockAuditEvidenceSummary(transaction) as T,
      };
    }

    if (req.kind === "ui.reports.export_audit_package") {
      return {
        kind: "ui.reports.export_audit_package",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          dir: "/tmp/kassiber-audit-package-mock",
          manifest: "/tmp/kassiber-audit-package-mock/manifest.json",
          format: "directory",
          scope: "audit_package",
          filename: "kassiber-audit-package-mock",
          transaction_count: 1,
          ready_count: 0,
          blocked_count: 1,
          evidence_file_count: mockAttachments.filter(
            (attachment) => attachment.attachment_type === "file",
          ).length,
          url_reference_count: mockAttachments.filter(
            (attachment) => attachment.attachment_type === "url",
          ).length,
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
            { kind: "bullbitcoin", summary: "Bull Bitcoin CSV importer." },
            { kind: "coinfinity", summary: "Coinfinity CSV importer." },
            { kind: "21bitcoin", summary: "21bitcoin CSV importer." },
            { kind: "pocketbitcoin", summary: "Pocket Bitcoin CSV importer." },
            { kind: "strike", summary: "Strike CSV importer." },
            { kind: "ledgerlive", summary: "Ledger Live CSV importer." },
            { kind: "kraken", summary: "Kraken API/CSV importer." },
            { kind: "coinbase", summary: "Coinbase API importer." },
            { kind: "binance", summary: "Binance API/CSV importer." },
            { kind: "wasabi", summary: "Wasabi Wallet sanitized bundle importer." },
          ],
          source_formats: [
            "btcpay_csv",
            "btcpay_json",
            "csv",
            "json",
            "phoenix_csv",
            "river_csv",
            "bullbitcoin_csv",
            "bullbitcoin_wallet_csv",
            "coinfinity_csv",
            "21bitcoin_csv",
            "pocketbitcoin_csv",
            "strike_csv",
            "ledgerlive_csv",
            "binance_supplemental_csv",
            "wasabi_bundle",
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

    if (req.kind === "ui.wallets.detect_script_types") {
      const args = (req.args ?? {}) as { wallet_material?: unknown };
      const material =
        typeof args.wallet_material === "string"
          ? args.wallet_material.trim()
          : "";
      if (!material.startsWith("xpub") && !material.startsWith("tpub")) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Script-type detection only applies to a bare xpub/tpub",
            retryable: false,
          },
        };
      }
      // Mock a mixed wallet: history on Native SegWit + Taproot, none on the
      // legacy/nested chains. Lets the add flow exercise the multi-active path.
      const detected = [
        { script_type: "p2pkh", has_history: false },
        { script_type: "p2sh-p2wpkh", has_history: false },
        { script_type: "p2wpkh", has_history: true },
        { script_type: "p2tr", has_history: true },
      ];
      return {
        kind: "ui.wallets.detect_script_types",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          probed: true,
          detected,
          active: ["p2wpkh", "p2tr"],
          fallback_used: false,
          reason: null,
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

    if (req.kind === "ui.connections.node.snapshot") {
      const args = (req.args ?? {}) as { connection?: unknown };
      const ref = typeof args.connection === "string" ? args.connection : "";
      if (!ref) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Lightning node snapshot requires `connection`.",
            hint: "Pass the wallet id or label of an LND/CLN connection.",
            retryable: false,
          },
        };
      }
      const connection = mockOverviewSnapshot().connections.find(
        (item) => item.id === ref || item.label === ref,
      );
      if (!connection) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Lightning connection '${ref}' not found.`,
            retryable: false,
          },
        };
      }
      const lightningKinds = new Set(["core-ln", "coreln", "lnd", "nwc"]);
      if (!connection.kind || !lightningKinds.has(connection.kind)) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: `Connection '${connection.label}' is not a Lightning node.`,
            retryable: false,
          },
        };
      }
      if (
        !connectionSupportsLightningCapability(connection, "nodeSnapshot")
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "lightning_capability_unsupported",
            message: `Lightning adapter for '${connection.label}' does not support node snapshots.`,
            retryable: false,
          },
        };
      }
      const node = (connection as { node?: unknown }).node;
      if (!node) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "lightning_adapter_unavailable",
            message: `No mock node snapshot is seeded for '${connection.label}'.`,
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.connections.node.snapshot",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          ...(node as Record<string, unknown>),
          connection: {
            id: connection.id,
            label: connection.label,
            kind: connection.kind,
            lightningCapabilities: connection.lightningCapabilities,
          },
          capabilities: connection.lightningCapabilities,
        } as T,
      };
    }

    if (req.kind === "ui.reports.lightning_profitability") {
      const args = (req.args ?? {}) as { connection?: unknown };
      const ref = typeof args.connection === "string" ? args.connection : "";
      if (!ref) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Lightning profitability requires `connection`.",
            hint: "Pass the wallet id or label of an LND/CLN connection.",
            retryable: false,
          },
        };
      }
      const connection = mockOverviewSnapshot().connections.find(
        (item) => item.id === ref || item.label === ref,
      );
      if (!connection) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Lightning connection '${ref}' not found.`,
            retryable: false,
          },
        };
      }
      if (
        !connectionSupportsLightningCapability(
          connection,
          "routingProfitability",
        )
      ) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "lightning_capability_unsupported",
            message: `Lightning adapter for '${connection.label}' does not support routing profitability.`,
            retryable: false,
          },
        };
      }
      const node = (connection as {
        node?: {
          routing?: {
            windowLabel: string;
            routingRevenueSat: number;
            paymentCostSat: number;
            rebalanceCostSat: number;
            onchainCostSat: number;
            netProfitSat: number;
            forwardCount: number;
            paymentCount: number;
            rebalanceCount: number;
          };
          channels?: Array<{
            id: string;
            peerAlias: string;
            capacitySat: number;
            earnedRoutingSat?: number | null;
          }>;
        };
      }).node;
      const routing = node?.routing ?? null;
      const channels = node?.channels ?? [];
      const summary = routing
        ? {
            routingRevenueSat: routing.routingRevenueSat,
            paymentCostSat: routing.paymentCostSat,
            rebalanceCostSat: routing.rebalanceCostSat,
            onchainCostSat: routing.onchainCostSat,
            netProfitSat: routing.netProfitSat,
            forwardCount: routing.forwardCount,
            paymentCount: routing.paymentCount,
            rebalanceCount: routing.rebalanceCount,
          }
        : {
            routingRevenueSat: 0,
            paymentCostSat: 0,
            rebalanceCostSat: 0,
            onchainCostSat: 0,
            netProfitSat: 0,
            forwardCount: 0,
            paymentCount: 0,
            rebalanceCount: 0,
          };
      const channelBreakEvens = channels.map((channel) => ({
        channelId: channel.id,
        peerAlias: channel.peerAlias,
        capacitySat: channel.capacitySat,
        earnedRoutingSat: channel.earnedRoutingSat ?? 0,
        openCostSat: DEFAULT_OPEN_COST_SAT,
        coversOpenCost:
          (channel.earnedRoutingSat ?? 0) >= DEFAULT_OPEN_COST_SAT,
      }));
      return {
        kind: "ui.reports.lightning_profitability",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          connection: {
            id: connection.id,
            label: connection.label,
            kind: connection.kind,
            lightningCapabilities: connection.lightningCapabilities,
          },
          windowLabel: routing?.windowLabel ?? "No routing window reported",
          summary,
          channels: channelBreakEvens,
        } as T,
      };
    }

    if (req.kind === "ui.reports.exit_tax_preview") {
      const args = (req.args ?? {}) as {
        departure_date?: unknown;
        destination?: unknown;
      };
      const departureDate =
        typeof args.departure_date === "string" ? args.departure_date : "2026-06-16";
      const destination: ExitTaxDestination =
        args.destination === "third_country" ? "third_country" : "eu_eea";
      return {
        kind: "ui.reports.exit_tax_preview",
        schema_version: 1,
        request_id: req.request_id,
        data: buildExitTaxFixture(departureDate, destination) as T,
      };
    }

    if (
      req.kind === "ui.reports.export_exit_tax_pdf" ||
      req.kind === "ui.reports.export_exit_tax_xlsx"
    ) {
      const args = (req.args ?? {}) as {
        departure_date?: unknown;
        destination?: unknown;
      };
      const departureDate =
        typeof args.departure_date === "string" ? args.departure_date : "2026-06-16";
      const destination: ExitTaxDestination =
        args.destination === "third_country" ? "third_country" : "eu_eea";
      const format = req.kind === "ui.reports.export_exit_tax_pdf" ? "pdf" : "xlsx";
      return {
        kind: req.kind,
        schema_version: 1,
        request_id: req.request_id,
        data: {
          file: `/mock/exports/kassiber-exit-tax-${departureDate}.${format}`,
          filename: `kassiber-exit-tax-${departureDate}.${format}`,
          bytes: format === "pdf" ? 2516 : 9260,
          format,
          scope: "exit_tax",
          departure_date: departureDate,
          destination,
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
            `Preview mode: simulated Electrum test for ${url}`,
            "No network request was made.",
            trustSelfSigned
              ? "Certificate verification: self-signed certificate would be trusted for this test."
              : certificate
                ? `Certificate verification: would use pinned certificate ${certificate}.`
                : "Certificate verification: would use system trust store.",
            proxy ? `Proxy: ${proxy}.` : "Proxy: disabled.",
            "Simulated result: connected.",
            "Simulated server version: Fulcrum 2.0 on protocol version 1.4.2",
            "Simulated server banner: Connected to a Fulcrum 2.0 server",
          ],
        } as T,
      };
    }

    if (req.kind === "ui.backends.http.test") {
      const args = (req.args ?? {}) as { url?: unknown; proxy?: unknown };
      const url = typeof args.url === "string" ? args.url.trim() : "";
      const proxy = typeof args.proxy === "string" ? args.proxy.trim() : "";
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
            `Preview mode: simulated HTTP test for ${url}`,
            "No network request was made.",
            proxy ? `Proxy: ${proxy}.` : "Proxy: disabled.",
            "Simulated response: HTTP 200 OK",
            "Simulated content-type: application/json",
            "Simulated body: 256 bytes sampled",
          ],
        } as T,
      };
    }

    if (req.kind === "ui.backends.bitcoinrpc.test") {
      const args = (req.args ?? {}) as { url?: unknown; backend?: unknown };
      const url =
        typeof args.url === "string"
          ? args.url.trim()
          : typeof args.backend === "string"
            ? `saved:${args.backend}`
            : "";
      if (!url) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Bitcoin Core test requires url or backend",
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.backends.bitcoinrpc.test",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          reachable: true,
          chain: "main",
          network: "main",
          blocks: 850000,
          headers: 850000,
          peers: 8,
          status: "synchronized",
          pruned: false,
          pruneheight: null,
          version: 270000,
          ibd: false,
          wallet_rpc: {
            available: true,
            loaded_wallet_count: 1,
          },
          block_filters: {
            available: true,
            type: "basic",
          },
          warnings: [],
        } as T,
      };
    }

    if (req.kind === "ui.backends.detect_core") {
      return {
        kind: "ui.backends.detect_core",
        schema_version: 1,
        request_id: req.request_id,
        data: { candidates: [] } as T,
      };
    }

    if (req.kind === "ui.backends.options") {
      return {
        kind: "ui.backends.options",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          backends: mockBackendSettingsRows.map((row) => ({
            name: row.name,
            display_name: row.display_name,
            kind: row.kind,
            chain: row.chain,
            network: row.network,
            is_default: row.is_default,
            has_url: row.has_url,
          })),
          summary: mockBackendSettingsPayload().summary,
        } as T,
      };
    }

    if (req.kind === "ui.backends.settings.list") {
      return {
        kind: "ui.backends.settings.list",
        schema_version: 1,
        request_id: req.request_id,
        data: mockBackendSettingsPayload() as T,
      };
    }
    if (req.kind === "ui.backends.public_defaults") {
      return {
        kind: "ui.backends.public_defaults",
        schema_version: 1,
        request_id: req.request_id,
        data: mockBackendPublicDefaultsPayload() as T,
      };
    }

    if (req.kind === "ui.backends.create") {
      const args = (req.args ?? {}) as Record<string, unknown>;
      const name = typeof args.name === "string" ? args.name.trim() : "";
      if (!name) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "validation",
            message: "Backend name is required",
            retryable: false,
          },
        };
      }
      if (mockBackendSettingsRows.some((row) => row.name === name)) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "conflict",
            message: `Backend '${name}' already exists`,
            retryable: false,
          },
        };
      }
      const row = mockBackendRowFromArgs(args);
      mockBackendSettingsRows = [...mockBackendSettingsRows, row];
      return {
        kind: "ui.backends.create",
        schema_version: 1,
        request_id: req.request_id,
        data: { ...row } as T,
      };
    }

    if (req.kind === "ui.backends.update") {
      const args = (req.args ?? {}) as Record<string, unknown>;
      const name = typeof args.name === "string" ? args.name.trim() : "";
      const existing = mockBackendSettingsRows.find((row) => row.name === name);
      if (!existing) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Backend '${name || "backend"}' not found`,
            retryable: false,
          },
        };
      }
      const row = mockBackendRowFromArgs(args, existing);
      mockBackendSettingsRows = mockBackendSettingsRows.map((item) =>
        item.name === name ? row : item,
      );
      return {
        kind: "ui.backends.update",
        schema_version: 1,
        request_id: req.request_id,
        data: { ...row } as T,
      };
    }

    if (req.kind === "ui.backends.delete") {
      const args = (req.args ?? {}) as { name?: unknown };
      const name = typeof args.name === "string" ? args.name.trim() : "";
      const before = mockBackendSettingsRows.length;
      mockBackendSettingsRows = mockBackendSettingsRows.filter(
        (row) => row.name !== name,
      );
      if (mockBackendSettingsRows.length === before) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Backend '${name || "backend"}' not found`,
            retryable: false,
          },
        };
      }
      return {
        kind: "ui.backends.delete",
        schema_version: 1,
        request_id: req.request_id,
        data: { name, deleted: true } as T,
      };
    }

    if (req.kind === "ui.backends.set_default") {
      const args = (req.args ?? {}) as { name?: unknown };
      const name = typeof args.name === "string" ? args.name.trim() : "";
      if (!mockBackendSettingsRows.some((row) => row.name === name)) {
        return {
          kind: "error",
          schema_version: 1,
          request_id: req.request_id,
          error: {
            code: "not_found",
            message: `Backend '${name || "backend"}' not found`,
            retryable: false,
          },
        };
      }
      mockBackendSettingsRows = mockBackendSettingsRows.map((row) => ({
        ...row,
        is_default: row.name === name,
      }));
      return {
        kind: "ui.backends.set_default",
        schema_version: 1,
        request_id: req.request_id,
        data: { default_backend: name } as T,
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
      const args = (req.args ?? {}) as { pair?: unknown };
      const overview = fixtures["ui.overview.snapshot"] as {
        marketRate?: {
          rate?: number | null;
          pair?: string | null;
          timestamp?: string | null;
          fetchedAt?: string | null;
          source?: string | null;
        };
      };
      if (overview.marketRate) {
        const now = new Date().toISOString();
        overview.marketRate.pair =
          typeof args.pair === "string" && args.pair.trim()
            ? args.pair.trim().toUpperCase()
            : overview.marketRate.pair;
        overview.marketRate.timestamp = now;
        overview.marketRate.fetchedAt = now;
        overview.marketRate.source = "coinbase-exchange";
      }
      return {
        kind: "ui.rates.rebuild",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          source: "coinbase-exchange",
          pair:
            typeof args.pair === "string" && args.pair.trim()
              ? args.pair.trim().toUpperCase()
              : null,
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
          reprice: {
            auto_priced: 8,
          },
          journals: {
            ok: true,
            result: {
              entries_created: 12,
              quarantined: 0,
              auto_priced: 8,
            },
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

    if (req.kind === "ui.rates.latest") {
      const args = (req.args ?? {}) as { pair?: unknown };
      const overview = fixtures["ui.overview.snapshot"] as {
        marketRate?: {
          asset?: "BTC";
          fiatCurrency?: string;
          pair?: string | null;
          rate?: number | null;
          timestamp?: string | null;
          fetchedAt?: string | null;
          source?: string | null;
          granularity?: string | null;
          method?: string | null;
        };
        priceEur?: number;
        priceUsd?: number;
        fiat?: {
          eurBalance?: number;
          eurUnrealized?: number;
        };
      };
      const pair =
        typeof args.pair === "string" && args.pair.trim()
          ? args.pair.trim().toUpperCase()
          : overview.marketRate?.pair ?? "BTC-EUR";
      const now = new Date().toISOString();
      const previousRate = Number(overview.marketRate?.rate ?? 71_420.18);
      const nextRate = Number((previousRate + 125.25).toFixed(2));
      if (overview.marketRate) {
        overview.marketRate.asset = "BTC";
        overview.marketRate.pair = pair;
        overview.marketRate.fiatCurrency = pair.includes("-")
          ? pair.split("-")[1] ?? overview.marketRate.fiatCurrency ?? "EUR"
          : overview.marketRate.fiatCurrency ?? "EUR";
        overview.marketRate.rate = nextRate;
        overview.marketRate.timestamp = now;
        overview.marketRate.fetchedAt = now;
        overview.marketRate.source = "coinbase-exchange";
        overview.marketRate.granularity = "minute";
        overview.marketRate.method = "product_candles";
      }
      if (pair === "BTC-EUR") overview.priceEur = nextRate;
      if (pair === "BTC-USD") overview.priceUsd = nextRate;
      if (overview.fiat?.eurBalance != null) {
        const btcBalance =
          previousRate > 0 ? overview.fiat.eurBalance / previousRate : 0;
        const nextBalance = btcBalance * nextRate;
        const delta = nextBalance - overview.fiat.eurBalance;
        overview.fiat.eurBalance = nextBalance;
        overview.fiat.eurUnrealized = (overview.fiat.eurUnrealized ?? 0) + delta;
      }
      return {
        kind: "ui.rates.latest",
        schema_version: 1,
        request_id: req.request_id,
        data: {
          source: "coinbase-exchange",
          pair,
          latest: [
            {
              pair,
              source: "coinbase-exchange",
              samples: 1,
              granularity: "minute",
              method: "product_candles",
              mode: "latest_quote",
              lookback_minutes: 5,
              timestamp: now,
              fetched_at: now,
            },
          ],
          marketRate: overview.marketRate ?? null,
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

    if (req.kind === "ui.transactions.resolve") {
      const query =
        typeof req.args?.query === "string" ? req.args.query.trim().toLowerCase() : "";
      const transactionList = fixtures["ui.transactions.list"] as {
        txs?: Array<{
          id?: string;
          externalId?: string;
          explorerId?: string;
        }>;
      };
      const transaction =
        transactionList.txs?.find((tx) =>
          [tx.id, tx.externalId, tx.explorerId]
            .filter(Boolean)
            .some((value) => value?.toLowerCase() === query),
        ) ?? null;
      return {
        kind: "ui.transactions.resolve",
        schema_version: 1,
        request_id: req.request_id,
        data: { transaction, query } as T,
      };
    }

    if (req.kind === "ui.transactions.list") {
      const args = (req.args ?? {}) as { wallet?: unknown };
      const wallet =
        typeof args.wallet === "string" && args.wallet.trim()
          ? args.wallet.trim()
          : null;
      if (wallet) {
        // Mirror the daemon's server-side wallet scoping so the preview's
        // wallet deep links return that wallet's rows (leg-aware, so a
        // transfer "Cold Storage -> Vault" is included for "Cold Storage").
        const base = fixtures["ui.transactions.list"] as {
          txs: Array<{ account?: string }>;
          nextCursor: unknown;
          hasMore: boolean;
        };
        const txs = base.txs.filter((tx) =>
          accountMatchesLabel(tx.account, wallet),
        );
        return {
          kind: "ui.transactions.list",
          schema_version: 1,
          request_id: req.request_id,
          data: { ...base, txs, nextCursor: null, hasMore: false } as T,
        };
      }
    }

    if (req.kind === "ui.source_funds.preview") {
      // Echo target_amount / reveal_mode so the planned-sale amount field has a
      // visible effect in the mock preview (the real daemon recomputes the
      // report from these args).
      const base = fixtures["ui.source_funds.preview"] as Record<string, unknown>;
      const reqArgs = (req.args ?? {}) as {
        target_amount?: unknown;
        reveal_mode?: unknown;
        report_options?: { reveal_overrides?: Record<string, string> };
      };
      const parsedAmount =
        typeof reqArgs.target_amount === "number"
          ? reqArgs.target_amount
          : typeof reqArgs.target_amount === "string" && reqArgs.target_amount.trim()
            ? Number(reqArgs.target_amount)
            : null;
      const clone = structuredClone(base) as Record<string, unknown>;
      if (parsedAmount != null && Number.isFinite(parsedAmount) && parsedAmount > 0) {
        clone.target = {
          ...(clone.target as Record<string, unknown>),
          required_amount: parsedAmount,
        };
        clone.overview = {
          ...(clone.overview as Record<string, unknown>),
          target_amount: parsedAmount,
        };
      }
      if (typeof reqArgs.reveal_mode === "string" && reqArgs.reveal_mode) {
        clone.reveal_mode = reqArgs.reveal_mode;
      }
      // Apply per-node reveal overrides so the disclosure preview flips live.
      const overrides = reqArgs.report_options?.reveal_overrides ?? {};
      if (Object.keys(overrides).length > 0) {
        const graph = clone.graph as { nodes?: Record<string, unknown>[] } | undefined;
        const hidden = new Set<string>();
        for (const node of graph?.nodes ?? []) {
          const id = String(node.transaction_id ?? "");
          if (overrides[id] === "hide") {
            if (node.external_id) hidden.add(String(node.external_id));
            node.external_id = "";
          }
        }
        if (hidden.size > 0) {
          const preview = clone.disclosure_preview as
            | { txids?: string[] }
            | undefined;
          if (preview && Array.isArray(preview.txids)) {
            preview.txids = preview.txids.filter((txid) => !hidden.has(txid));
          }
        }
      }
      return {
        kind: req.kind,
        schema_version: 1,
        request_id: req.request_id,
        data: clone as T,
      };
    }

    if (req.kind === "ui.transactions.graph") {
      const args = (req.args ?? {}) as { transaction?: unknown };
      const transactionId =
        typeof args.transaction === "string" && args.transaction.trim()
          ? args.transaction.trim()
          : "tx19";
      const graph =
        MOCK_TRANSACTION_GRAPHS[transactionId] ??
        mockGraphlessTransactionGraph(transactionId);
      return {
        kind: "ui.transactions.graph",
        schema_version: 1,
        request_id: req.request_id,
        data: graph as T,
      };
    }

    if (req.kind === "ui.transfers.components.bulk_resolve") {
      const args = (req.args ?? {}) as {
        components?: unknown;
        activate?: unknown;
        dry_run?: unknown;
      };
      const specs = Array.isArray(args.components) ? args.components : [];
      const activate = args.activate !== false;
      const base = structuredClone(
        fixtures["ui.transfers.components.bulk_resolve"],
      ) as {
        components: Array<Record<string, unknown>>;
      };
      const template = base.components[0] ?? {};
      const components = specs.map((_, index) => ({
        ...structuredClone(template),
        id: `custody:mock-bulk-${index + 1}`,
        lineage_id: `custody:mock-bulk-${index + 1}`,
        state: activate ? "active" : "draft",
        effective_state: activate ? "active" : "draft",
      }));
      return {
        kind: req.kind,
        schema_version: 1,
        request_id: req.request_id,
        data: {
          fingerprint: "a".repeat(64),
          components,
          summary: {
            count: components.length,
            active: activate ? components.length : 0,
            draft: activate ? 0 : components.length,
          },
          ...(args.dry_run === true ? { dry_run: true } : {}),
        } as T,
      };
    }

    if (
      req.kind === "ui.custody.gaps.residual.preview" ||
      req.kind === "ui.custody.gaps.residual.classify"
    ) {
      const args = (req.args ?? {}) as { classification?: unknown };
      const classification =
        typeof args.classification === "string"
          ? args.classification
          : "suspense_continuation";
      const custodyState = [
        "external_payment",
        "external_disposal",
        "external_gift",
        "external_loss",
      ].includes(classification)
        ? "external_confirmed"
        : classification === "retained_custody"
          ? "internal_reviewed"
          : "custody_suspense";
      const fixture = structuredClone(fixtures[req.kind]) as Record<string, unknown>;
      return {
        kind: req.kind,
        schema_version: 1,
        request_id: req.request_id,
        data: {
          ...fixture,
          classification,
          custody_state: custodyState,
          country_tax_meaning: "not_assigned",
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
    if (req.kind === "ui.freshness.run") {
      return mockFreshnessRunStream<T, R>(req, options);
    }
    if (req.kind === "ui.workspace.freshness.run") {
      return mockWorkspaceFreshnessRunStream<T, R>(req, options);
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

async function mockFreshnessRunStream<T, R>(
  req: DaemonRequest,
  options?: DaemonStreamOptions<R>,
): Promise<DaemonEnvelope<T>> {
  const requestId =
    req.request_id ?? `mock-freshness-${Math.random().toString(36).slice(2)}`;
  const steps = [
    {
      phase: "discovery",
      source_label: "Treasury watch-only",
      source_type: "onchain_wallet",
    },
    {
      phase: "backend_fetch",
      source_label: "Treasury watch-only",
      source_type: "onchain_wallet",
      processed: 400,
      total: 1200,
    },
    {
      phase: "rate_coverage",
      source_label: "Market-rate coverage",
      source_type: "market_rates",
    },
    {
      phase: "journal_refresh",
      source_label: "Journals",
      source_type: "journals",
    },
  ];
  for (const data of steps) {
    if (options?.signal?.aborted) break;
    await new Promise((resolve) => setTimeout(resolve, 60));
    options?.onRecord?.({
      kind: "ui.freshness.run.progress",
      schema_version: 1,
      request_id: requestId,
      data: data as R,
    });
  }
  return mockDaemon.invoke<T>({ ...req, request_id: requestId });
}

async function mockWorkspaceFreshnessRunStream<T, R>(
  req: DaemonRequest,
  options?: DaemonStreamOptions<R>,
): Promise<DaemonEnvelope<T>> {
  const requestId =
    req.request_id ??
    `mock-workspace-freshness-${Math.random().toString(36).slice(2)}`;
  const args = (req.args ?? {}) as { workspace_id?: unknown };
  const workspaceId =
    typeof args.workspace_id === "string" && args.workspace_id.trim()
      ? args.workspace_id.trim()
      : mockProfilesSnapshot.activeWorkspaceId;
  const overview = mockWorkspaceOverviewSnapshot(workspaceId);
  for (const book of overview.books) {
    const steps = [
      {
        workspace: overview.workspace,
        profile: { id: book.profile.id, label: book.profile.label },
        phase: "discovery",
        source_label: `${book.profile.label} sources`,
        source_type: "workspace_book",
      },
      {
        workspace: overview.workspace,
        profile: { id: book.profile.id, label: book.profile.label },
        phase: "journal_refresh",
        source_label: "Journals",
        source_type: "journals",
      },
    ];
    for (const data of steps) {
      if (options?.signal?.aborted) break;
      await new Promise((resolve) => setTimeout(resolve, 40));
      options?.onRecord?.({
        kind: "ui.workspace.freshness.run.progress",
        schema_version: 1,
        request_id: requestId,
        data: data as R,
      });
    }
  }
  return mockDaemon.invoke<T>({ ...req, request_id: requestId });
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
    messages?: { role?: string; content?: string }[];
    persist?: boolean | "auto";
    session_id?: string;
    seed_history?: boolean;
    screen_context?: { route?: string };
  };
  if (
    typeof args.session_id === "string" &&
    args.persist !== false &&
    !mockChatSessions.some((row) => row.id === args.session_id)
  ) {
    // Mirror the real daemon: unknown session ids fail before streaming.
    return {
      kind: "error",
      schema_version: 1,
      request_id: requestId,
      error: {
        code: "not_found",
        message: "chat session not found for the active profile",
      },
    } as DaemonEnvelope<T>;
  }
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
  const assistantContent = MOCK_AI_CHAT_STREAM.map(
    (chunk) => chunk.content ?? "",
  ).join("");
  let sessionId: string | null = null;
  const optedIn = args.persist === true || args.persist === "auto" ||
    typeof args.session_id === "string";
  if (args.persist !== false && optedIn && mockChatHistoryEnabled()) {
    const lastUser = [...(args.messages ?? [])]
      .reverse()
      .find((message) => message.role === "user");
    const userContent =
      typeof lastUser?.content === "string" ? lastUser.content : "";
    if (userContent) {
      const now = new Date().toISOString();
      let session = mockChatSessions.find((row) => row.id === args.session_id);
      if (!session) {
        session = {
          id: `mock-chat-session-${Math.random().toString(36).slice(2, 8)}`,
          title: userContent.slice(0, 80),
          provider: args.provider ?? "ollama",
          model: args.model ?? "mock-model",
          created_at: now,
          updated_at: now,
          entries: [],
        };
        // Backfill a branched/edited seed only when the fork explicitly asked
        // for it (seed_history) — mirrors the daemon. A bare new session (e.g.
        // history re-enabled, or a deleted session with messages on screen)
        // must not have its prior turns written in.
        const messages = args.seed_history ? (args.messages ?? []) : [];
        let lastUser = -1;
        messages.forEach((message, index) => {
          if (message.role === "user") lastUser = index;
        });
        if (lastUser > 0) {
          for (const message of messages.slice(0, lastUser)) {
            if (
              (message.role === "user" || message.role === "assistant") &&
              typeof message.content === "string" &&
              message.content
            ) {
              session.entries.push({
                role: message.role,
                content: message.content,
              });
            }
          }
        }
        mockChatSessions.push(session);
      }
      session.entries.push(
        { role: "user", content: userContent },
        { role: "assistant", content: assistantContent },
      );
      session.updated_at = now;
      sessionId = session.id;
    }
  }
  return {
    kind: "ai.chat",
    schema_version: 1,
    request_id: requestId,
    data: {
      provider: args.provider ?? "ollama",
      model: args.model ?? "mock-model",
      finish_reason: cancelled ? "cancelled" : "stop",
      session_id: sessionId,
      provenance: {
        generated_at: new Date().toISOString(),
        provider: args.provider ?? "ollama",
        model: args.model ?? "mock-model",
        tools_used: args.tools_enabled ? ["ui.workspace.health"] : [],
        privacy_receipt: {
          provider_kind: "local",
          remote_provider: false,
          screen_route: args.screen_context?.route ?? null,
          advertised_tool_count: args.tools_enabled ? 8 : 0,
          tools_executed: args.tools_enabled ? 1 : 0,
          egress_records: 1,
          egress_endpoints: 1,
          egress_bytes_out: 512,
          egress_subsystems: ["ai"],
          egress_gap: false,
          history_intent: args.persist ?? null,
          hostnames_disclosed_to_model: false,
        },
      },
    } as T,
  };
}
