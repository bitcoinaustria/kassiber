/**
 * Daemon transport selector.
 *
 * Three runtime modes per docs/plan/04-desktop-ui.md §2.6:
 *   - "mock"   — fixture responses, no Python required (default dev mode)
 *   - "bridge" — Vite dev-server bridge to the Python daemon, dev-only
 *   - "tauri"  — JSONL over stdin/stdout via Rust supervisor (production)
 *
 * The Tauri mode calls the Rust shell command boundary, which forwards
 * whitelisted requests to the local Python daemon.
 */

import { mockDaemon, mockStream } from "./mock";
import { useUiStore, type DataMode } from "@/store/ui";

export type DaemonMode = "mock" | "bridge" | "tauri";

function defaultDaemonMode(): DaemonMode {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    return "tauri";
  }
  if (import.meta.env.DEV) {
    return "bridge";
  }
  return "mock";
}

const RAW_MODE = (import.meta.env.VITE_DAEMON ?? defaultDaemonMode()) as string;

if (!["mock", "bridge", "tauri"].includes(RAW_MODE)) {
  throw new Error(
    `VITE_DAEMON must be one of mock|bridge|tauri (got ${RAW_MODE})`,
  );
}

export const DAEMON_MODE = RAW_MODE as DaemonMode;

export interface DaemonRequest {
  kind: string;
  args?: Record<string, unknown>;
  /**
   * Optional client-allocated request_id. Streaming transports allocate
   * one automatically so they can filter `daemon://stream` records as
   * they arrive instead of buffering them until the terminal envelope.
   */
  request_id?: string;
}

export interface DaemonEnvelope<T = unknown> {
  kind: string;
  schema_version: number;
  request_id?: string | number | null;
  data?: T;
  error?: {
    code: string;
    message: string;
    hint?: string | null;
    details?: unknown;
    retryable?: boolean;
  };
}

/**
 * Stream record forwarded by the Rust supervisor while a streaming kind
 * (e.g. `ai.chat`) is in flight. Mid-stream records have a kind shaped
 * like `<request_kind>.delta`; the terminal record matches the request
 * kind exactly and is delivered as the resolved value of `stream()`.
 */
export interface DaemonStreamRecord<T = unknown> {
  kind: string;
  schema_version: number;
  request_id?: string | number | null;
  data?: T;
}

export interface DaemonStreamOptions<T = unknown> {
  /** Receive each mid-stream record (kind = `<request_kind>.delta` etc.). */
  onRecord?: (record: DaemonStreamRecord<T>) => void;
  /** Optional abort signal; transports that support cancellation watch this. */
  signal?: AbortSignal;
}

export interface DaemonTransport {
  invoke<T = unknown>(req: DaemonRequest): Promise<DaemonEnvelope<T>>;
  /** Streaming variant; resolves with the terminal envelope. */
  stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>>;
}

export interface ImportProjectSelection {
  stateRoot: string;
  dataRoot: string;
  database: string;
  encrypted: boolean;
}

let activeImportProjectSelection: ImportProjectSelection | null = null;
let activeImportProjectActivation:
  | {
      dataRoot: string;
      generation: number;
      promise: Promise<ImportProjectSelection>;
    }
  | null = null;
let importProjectActivationGeneration = 0;

function isTerminalEnvelopeKind(kind: string, requestKind: string): boolean {
  return kind === requestKind || kind === "error" || kind === "auth_required";
}

export async function readBridgeNdjsonStream<T = unknown, R = unknown>(
  response: Response,
  requestKind: string,
  requestId: string,
  options?: DaemonStreamOptions<R>,
): Promise<DaemonEnvelope<T>> {
  if (!response.ok) {
    throw new Error(`bridge stream failed with HTTP ${response.status}`);
  }
  if (!response.body) {
    throw new Error("bridge stream response did not include a body");
  }

  const decoder = new TextDecoder();
  const reader = response.body.getReader();
  let buffer = "";
  let terminal: DaemonEnvelope<T> | null = null;

  const handleLine = (line: string) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const record = JSON.parse(trimmed) as DaemonStreamRecord<R>;
    if (record.request_id !== requestId) {
      return;
    }
    if (isTerminalEnvelopeKind(record.kind, requestKind)) {
      terminal = record as DaemonEnvelope<T>;
      return;
    }
    if (!options?.signal?.aborted) {
      options?.onRecord?.(record);
    }
  };

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let lineEnd = buffer.indexOf("\n");
    while (lineEnd >= 0) {
      handleLine(buffer.slice(0, lineEnd));
      buffer = buffer.slice(lineEnd + 1);
      lineEnd = buffer.indexOf("\n");
    }
  }

  buffer += decoder.decode();
  handleLine(buffer);

  if (!terminal) {
    throw new Error("bridge stream ended without a terminal daemon envelope");
  }
  return terminal;
}

export function makeDaemonRequestId(): string {
  if (
    typeof globalThis.crypto !== "undefined" &&
    typeof globalThis.crypto.randomUUID === "function"
  ) {
    return globalThis.crypto.randomUUID();
  }
  return `req-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

const SENSITIVE_LOG_KEYS = [
  "api_key",
  "auth_response",
  "auth_header",
  "cookie",
  "descriptor",
  "new_passphrase_secret",
  "passphrase",
  "passphrase_secret",
  "private",
  "secret",
  "token",
  "xprv",
];

const SENSITIVE_LOG_PATTERNS: Array<[RegExp, string]> = [
  [
    /\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b/g,
    "[redacted-private-key]",
  ],
  [
    /\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b/g,
    "[redacted-extended-key]",
  ],
  [
    /\b(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)/gi,
    "[redacted-descriptor]",
  ],
  [/\b[Bb]earer\s+[A-Za-z0-9._~+/-]+=*/g, "Bearer [redacted]"],
  [
    /\b(api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase|password|secret|token)\b(\s*[:=]\s*)([^\s,;"']+)/gi,
    "$1$2[redacted]",
  ],
];

export function redactForLog(value: unknown, depth = 0): unknown {
  if (depth > 8) return "[truncated]";
  if (Array.isArray(value)) {
    return value.map((item) => redactForLog(item, depth + 1));
  }
  if (typeof value === "string") {
    return redactStringForLog(value);
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value).map(([key, child]) => {
      const normalized = key.toLowerCase();
      if (
        SENSITIVE_LOG_KEYS.some((sensitive) =>
          normalized.includes(sensitive),
        )
      ) {
        return [key, "[redacted]"];
      }
      return [key, redactForLog(child, depth + 1)];
    }),
  );
}

function redactStringForLog(value: string): string {
  return SENSITIVE_LOG_PATTERNS.reduce(
    (current, [pattern, replacement]) => current.replace(pattern, replacement),
    value,
  );
}

function summarizeRequestForLog(req: DaemonRequest): unknown {
  const args = req.args ?? {};
  return {
    request_id: req.request_id ?? null,
    arg_keys: Object.keys(args),
  };
}

function recordDaemonLog(
  level: "debug" | "info" | "warning" | "error",
  source: string,
  message: string,
  details?: unknown,
) {
  useUiStore.getState().addLogEntry({
    level,
    source,
    message,
    details: redactForLog(details),
  });
}

function envelopeLogLevel(envelope: DaemonEnvelope): "info" | "warning" | "error" {
  if (envelope.kind === "error" || envelope.error) return "error";
  if (envelope.kind === "auth_required") return "warning";
  return "info";
}

function summarizeEnvelopeForLog(envelope: DaemonEnvelope): unknown {
  const summary: Record<string, unknown> = {
    kind: envelope.kind,
    schema_version: envelope.schema_version,
    request_id: envelope.request_id ?? null,
  };
  if (envelope.error) {
    summary.error = envelope.error;
  } else if (envelope.data && typeof envelope.data === "object") {
    summary.data_keys = Object.keys(envelope.data as Record<string, unknown>);
  } else if (envelope.data !== undefined) {
    summary.data_type = typeof envelope.data;
  }
  return summary;
}

function withDaemonLogging(
  transport: DaemonTransport,
  source: string,
): DaemonTransport {
  return {
    async invoke<T = unknown>(
      req: DaemonRequest,
    ): Promise<DaemonEnvelope<T>> {
      recordDaemonLog(
        "debug",
        source,
        `invoke ${req.kind}`,
        summarizeRequestForLog(req),
      );
      try {
        const envelope = await transport.invoke<T>(req);
        recordDaemonLog(
          envelopeLogLevel(envelope),
          source,
          `terminal ${envelope.kind}`,
          summarizeEnvelopeForLog(envelope),
        );
        return envelope;
      } catch (error) {
        recordDaemonLog("error", source, `invoke ${req.kind} threw`, {
          message: error instanceof Error ? error.message : String(error),
        });
        throw error;
      }
    },
    async stream<T = unknown, R = unknown>(
      req: DaemonRequest,
      options?: DaemonStreamOptions<R>,
    ): Promise<DaemonEnvelope<T>> {
      recordDaemonLog(
        "debug",
        source,
        `stream ${req.kind}`,
        summarizeRequestForLog(req),
      );
      try {
        const envelope = await transport.stream<T, R>(req, options);
        recordDaemonLog(
          envelopeLogLevel(envelope),
          source,
          `terminal ${envelope.kind}`,
          summarizeEnvelopeForLog(envelope),
        );
        return envelope;
      } catch (error) {
        recordDaemonLog("error", source, `stream ${req.kind} threw`, {
          message: error instanceof Error ? error.message : String(error),
        });
        throw error;
      }
    },
  };
}

export async function openExportedFile(path: string): Promise<void> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Opening exported files is available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_exported_file", { path });
}

export function normalizeExternalBrowserUrl(url: string): string {
  const trimmed = url.trim();
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error("Only absolute HTTP or HTTPS explorer URLs can be opened.");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("Only HTTP or HTTPS explorer URLs can be opened.");
  }
  if (!parsed.host) {
    throw new Error("Explorer URLs must include a host.");
  }
  if (parsed.username || parsed.password) {
    throw new Error("Explorer URLs with embedded credentials cannot be opened.");
  }
  return parsed.toString();
}

export async function openExternalUrl(url: string): Promise<void> {
  const normalized = normalizeExternalBrowserUrl(url);
  if (DAEMON_MODE !== "tauri") {
    if (typeof window === "undefined") {
      throw new Error("Opening explorer URLs requires a browser window.");
    }
    window.open(normalized, "_blank", "noopener,noreferrer");
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_external_url", { url: normalized });
}

export function canOpenExportedFiles(): boolean {
  return DAEMON_MODE === "tauri";
}

export async function selectImportProjectDirectory(): Promise<ImportProjectSelection | null> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Project import is available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ImportProjectSelection | null>("select_import_project_directory");
}

export async function activateImportProject(
  dataRoot: string,
): Promise<ImportProjectSelection> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Project import is available in the desktop app.");
  }
  if (activeImportProjectSelection?.dataRoot === dataRoot) {
    return activeImportProjectSelection;
  }
  if (activeImportProjectActivation?.dataRoot === dataRoot) {
    return activeImportProjectActivation.promise;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  const generation = ++importProjectActivationGeneration;
  const promise = invoke<ImportProjectSelection>("activate_import_project", {
    dataRoot,
  });
  activeImportProjectActivation = { dataRoot, generation, promise };
  try {
    const selection = await promise;
    if (generation === importProjectActivationGeneration) {
      if (activeImportProjectSelection?.dataRoot !== selection.dataRoot) {
        useUiStore.getState().bumpDaemonSession();
      }
      activeImportProjectSelection = selection;
    }
    return selection;
  } finally {
    if (activeImportProjectActivation?.promise === promise) {
      activeImportProjectActivation = null;
    }
  }
}

export async function clearImportProject(): Promise<void> {
  if (DAEMON_MODE !== "tauri") {
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  importProjectActivationGeneration += 1;
  activeImportProjectActivation = null;
  await invoke("clear_import_project");
  if (activeImportProjectSelection !== null) {
    activeImportProjectSelection = null;
    useUiStore.getState().bumpDaemonSession();
  }
}

export function canImportProjects(): boolean {
  return DAEMON_MODE === "tauri";
}

const tauriDaemon: DaemonTransport = {
  async invoke<T = unknown>(
    req: DaemonRequest,
  ): Promise<DaemonEnvelope<T>> {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<DaemonEnvelope<T>>("daemon_invoke", { request: req });
  },
  async stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>> {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");

    // Allocate the request_id client-side and pass it through; the
    // supervisor honors a String request_id when supplied, so we can
    // filter `daemon://stream` records as they arrive instead of
    // buffering them until the terminal envelope returns.
    const requestId = req.request_id ?? makeDaemonRequestId();

    const unlisten = await listen<DaemonStreamRecord<R>>(
      "daemon://stream",
      (event) => {
        if (options?.signal?.aborted) return;
        if (event.payload.request_id === requestId) {
          options?.onRecord?.(event.payload);
        }
      },
    );

    try {
      return await invoke<DaemonEnvelope<T>>("daemon_invoke", {
        request: { ...req, request_id: requestId },
      });
    } finally {
      unlisten();
    }
  },
};

const bridgeDaemon: DaemonTransport = {
  async invoke<T = unknown>(
    req: DaemonRequest,
  ): Promise<DaemonEnvelope<T>> {
    const response = await fetch("/__kassiber__/daemon", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(req),
    });
    return response.json() as Promise<DaemonEnvelope<T>>;
  },
  async stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>> {
    const requestId = req.request_id ?? makeDaemonRequestId();
    const response = await fetch("/__kassiber__/daemon/stream", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...req, request_id: requestId }),
    });
    return readBridgeNdjsonStream<T, R>(
      response,
      req.kind,
      requestId,
      options,
    );
  },
};

export function getTransport(dataMode?: DataMode): DaemonTransport {
  if ((dataMode ?? useUiStore.getState().dataMode) === "mock") {
    return withDaemonLogging(
      { invoke: mockDaemon.invoke, stream: mockStream },
      "daemon:mock",
    );
  }

  switch (DAEMON_MODE) {
    case "mock":
      return withDaemonLogging(
        { invoke: mockDaemon.invoke, stream: mockStream },
        "daemon:mock",
      );
    case "bridge":
      return withDaemonLogging(bridgeDaemon, "daemon:bridge");
    case "tauri":
      return withDaemonLogging(tauriDaemon, "daemon:tauri");
  }
}
