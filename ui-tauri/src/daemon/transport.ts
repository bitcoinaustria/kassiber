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
import {
  emitAppLog,
  type AppLogField,
  type AppLogLevel,
} from "@/lib/appLogs";

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

export interface TouchIdPassphraseStatus {
  platform: "macos" | "windows" | "linux" | "unsupported";
  available: boolean;
  configured: boolean;
  reason?: string | null;
}

export interface TouchIdPassphraseUnlock {
  passphraseSecret: string;
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

function recordDaemonLog(
  level: AppLogLevel,
  source: string,
  message: string,
  fields: Record<string, AppLogField>,
) {
  emitAppLog({
    level,
    module: source,
    file: "daemon/transport.ts",
    // Wrapper events are logical transport records; a fixed source line would drift.
    line: 0,
    msg: message,
    fields,
  });
}

function envelopeLogLevel(envelope: DaemonEnvelope): "info" | "warning" | "error" {
  if (envelope.kind === "error" || envelope.error) return "error";
  if (envelope.kind === "auth_required") return "warning";
  return "info";
}

function textField(value: unknown): AppLogField {
  return { type: "text", value: String(value ?? "") };
}

function numberField(value: unknown): AppLogField {
  return { type: "number", value: typeof value === "number" ? value : 0 };
}

function booleanField(value: unknown): AppLogField {
  return { type: "boolean", value: Boolean(value) };
}

function summarizeRequestFields(req: DaemonRequest): Record<string, AppLogField> {
  const args = req.args ?? {};
  return {
    kind: textField(req.kind),
    request_id: textField(req.request_id ?? ""),
    arg_keys: textField(Object.keys(args).sort().join(",")),
  };
}

function summarizeEnvelopeFields(
  envelope: DaemonEnvelope,
): Record<string, AppLogField> {
  const summary: Record<string, AppLogField> = {
    kind: textField(envelope.kind),
    schema_version: numberField(envelope.schema_version),
    request_id: textField(envelope.request_id ?? ""),
  };
  if (envelope.error) {
    summary.error_code = textField(envelope.error.code);
    summary.error_message = {
      type: "label",
      value: envelope.error.message,
    };
    summary.retryable = booleanField(envelope.error.retryable);
    if (envelope.error.hint) {
      summary.error_hint = { type: "label", value: envelope.error.hint };
    }
  } else if (envelope.data && typeof envelope.data === "object") {
    summary.data_keys = textField(
      Object.keys(envelope.data as Record<string, unknown>).sort().join(","),
    );
  } else if (envelope.data !== undefined) {
    summary.data_type = textField(typeof envelope.data);
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
        "Daemon invoke started",
        summarizeRequestFields(req),
      );
      try {
        const envelope = await transport.invoke<T>(req);
        recordDaemonLog(
          envelopeLogLevel(envelope),
          source,
          "Daemon invoke finished",
          summarizeEnvelopeFields(envelope),
        );
        return envelope;
      } catch (error) {
        recordDaemonLog(
          "error",
          source,
          "Daemon invoke threw",
          {
            kind: textField(req.kind),
            error_message: {
              type: "label",
              value: error instanceof Error ? error.message : String(error),
            },
          },
        );
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
        "Daemon stream started",
        summarizeRequestFields(req),
      );
      try {
        const envelope = await transport.stream<T, R>(req, {
          ...options,
          onRecord: (record) => {
            recordDaemonLog(
              "trace",
              source,
              "Daemon stream record",
              summarizeEnvelopeFields(record),
            );
            options?.onRecord?.(record);
          },
        });
        recordDaemonLog(
          envelopeLogLevel(envelope),
          source,
          "Daemon stream finished",
          summarizeEnvelopeFields(envelope),
        );
        return envelope;
      } catch (error) {
        recordDaemonLog(
          "error",
          source,
          "Daemon stream threw",
          {
            kind: textField(req.kind),
            error_message: {
              type: "label",
              value: error instanceof Error ? error.message : String(error),
            },
          },
        );
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

export async function saveExportedFileAs(
  sourcePath: string,
  destinationPath: string,
): Promise<string> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Saving exported files is available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("save_exported_file_as", {
    sourcePath,
    destinationPath,
  });
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

export function canSaveExportedFiles(): boolean {
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

export function isImportProjectActive(dataRoot: string): boolean {
  return activeImportProjectSelection?.dataRoot === dataRoot;
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

export function canUseTouchIdPassphraseUnlock(): boolean {
  if (DAEMON_MODE !== "tauri" || typeof navigator === "undefined") {
    return false;
  }
  return navigator.platform.toLowerCase().includes("mac");
}

const TOUCH_ID_UNAVAILABLE: TouchIdPassphraseStatus = {
  platform: "unsupported",
  available: false,
  configured: false,
  reason: "desktop_only",
};

export async function touchIdPassphraseStatus(
  dataRoot?: string | null,
): Promise<TouchIdPassphraseStatus> {
  if (DAEMON_MODE !== "tauri") {
    return TOUCH_ID_UNAVAILABLE;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TouchIdPassphraseStatus>("touch_id_passphrase_status_command", {
    dataRoot: dataRoot ?? null,
  });
}

export async function storeTouchIdPassphrase(
  passphraseSecret: string,
  dataRoot?: string | null,
): Promise<TouchIdPassphraseStatus> {
  if (DAEMON_MODE !== "tauri") {
    return TOUCH_ID_UNAVAILABLE;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TouchIdPassphraseStatus>("touch_id_store_passphrase_command", {
    dataRoot: dataRoot ?? null,
    passphraseSecret,
  });
}

export async function unlockTouchIdPassphrase(
  dataRoot?: string | null,
): Promise<TouchIdPassphraseUnlock | null> {
  if (DAEMON_MODE !== "tauri") {
    return null;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TouchIdPassphraseUnlock | null>(
    "touch_id_unlock_passphrase_command",
    {
      dataRoot: dataRoot ?? null,
    },
  );
}

export async function forgetTouchIdPassphrase(
  dataRoot?: string | null,
): Promise<TouchIdPassphraseStatus> {
  if (DAEMON_MODE !== "tauri") {
    return TOUCH_ID_UNAVAILABLE;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TouchIdPassphraseStatus>("touch_id_forget_passphrase_command", {
    dataRoot: dataRoot ?? null,
  });
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
