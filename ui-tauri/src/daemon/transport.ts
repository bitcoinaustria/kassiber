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
    debug?: string | null;
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

/**
 * Unsolicited daemon→UI event record (`event: true`, never a
 * `request_id`), e.g. `ui.freshness.background` / `ui.freshness.worker`
 * from the background freshness worker. The Rust supervisor forwards
 * these on the `daemon://event` Tauri channel, separate from the
 * per-request `daemon://stream` records.
 */
export interface DaemonEventRecord<T = unknown> {
  kind: string;
  schema_version: number;
  event: true;
  data?: T;
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

export interface TerminalCommandStatus {
  platform: "macos" | "windows" | "linux" | "unsupported";
  available: boolean;
  installed: boolean;
  managed: boolean;
  needsRepair: boolean;
  conflict: boolean;
  pathOnPath: boolean;
  command: string;
  binDir: string;
  commandPath: string;
  targetPath: string;
  pathHint: string;
  message: string;
}

const IMPORT_PROJECT_BRIDGE_PATH = "/__kassiber__/import-project";

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

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function syncErrorRows(data: unknown): Record<string, unknown>[] {
  if (!isRecord(data) || !Array.isArray(data.results)) return [];
  return data.results.filter(
    (row): row is Record<string, unknown> =>
      isRecord(row) && String(row.status ?? "").toLowerCase() === "error",
  );
}

export function envelopeLogLevel(
  envelope: DaemonEnvelope,
): "info" | "warning" | "error" {
  if (envelope.kind === "error" || envelope.error) return "error";
  if (syncErrorRows(envelope.data).length > 0) return "error";
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

function requestWithId(req: DaemonRequest): DaemonRequest {
  return req.request_id ? req : { ...req, request_id: makeDaemonRequestId() };
}

function summarizeRequestFields(req: DaemonRequest): Record<string, AppLogField> {
  const args = req.args ?? {};
  const requestId = req.request_id ?? "";
  return {
    kind: textField(req.kind),
    request_id: textField(requestId),
    trace_id: textField(requestId),
    arg_keys: textField(Object.keys(args).sort().join(",")),
  };
}

export function summarizeEnvelopeFields(
  envelope: DaemonEnvelope,
): Record<string, AppLogField> {
  const requestId = envelope.request_id ?? "";
  const summary: Record<string, AppLogField> = {
    kind: textField(envelope.kind),
    schema_version: numberField(envelope.schema_version),
    request_id: textField(requestId),
    trace_id: textField(requestId),
  };
  if (envelope.error) {
    summary.error_code = textField(envelope.error.code);
    summary.error_message = textField(envelope.error.message);
    summary.retryable = booleanField(envelope.error.retryable);
    if (envelope.error.hint) {
      summary.error_hint = textField(envelope.error.hint);
    }
    // Secret-floor redaction runs at insert in appLogs and operational
    // redaction at render time, so the raw payloads can pass through here.
    const { details, debug } = envelope.error;
    if (details !== undefined && details !== null) {
      summary.error_details = textField(JSON.stringify(details).slice(0, 2048));
    }
    if (typeof debug === "string" && debug) {
      summary.error_debug = textField(debug.slice(0, 2048));
    }
  } else if (envelope.data && typeof envelope.data === "object") {
    addDataSummaryFields(summary, envelope.data as Record<string, unknown>);
  } else if (envelope.data !== undefined) {
    summary.data_type = textField(typeof envelope.data);
  }
  return summary;
}

function addDataSummaryFields(
  summary: Record<string, AppLogField>,
  data: Record<string, unknown>,
): void {
  const keys = Object.keys(data).sort();
  summary.data_keys = textField(keys.join(","));
  const phase = data.phase;
  if (typeof phase === "string") summary.phase = textField(phase);
  const label = data.label;
  if (typeof label === "string") summary.label = { type: "label", value: label };
  const callId = data.call_id;
  if (typeof callId === "string") summary.call_id = textField(callId);
  const name = data.name;
  if (typeof name === "string") summary.tool_name = textField(name);
  const kindClass = data.kind_class;
  if (typeof kindClass === "string") summary.kind_class = textField(kindClass);
  if (typeof data.needs_consent === "boolean") {
    summary.needs_consent = booleanField(data.needs_consent);
  }
  const finishReason = data.finish_reason;
  if (typeof finishReason === "string") {
    summary.finish_reason = textField(finishReason);
  }
  const model = data.model;
  if (typeof model === "string") summary.model = textField(model);
  const args = data.arguments;
  if (args && typeof args === "object" && !Array.isArray(args)) {
    summary.argument_keys = textField(
      Object.keys(args as Record<string, unknown>).sort().join(","),
    );
  }
  if (data.provenance && typeof data.provenance === "object") {
    summary.has_provenance = booleanField(true);
  }
  addSyncErrorSummaryFields(summary, data);
}

function addSyncErrorSummaryFields(
  summary: Record<string, AppLogField>,
  data: Record<string, unknown>,
): void {
  const errors = syncErrorRows(data);
  const first = errors[0];
  if (!first) return;

  summary.sync_error_count = numberField(errors.length);
  const wallet = first.wallet;
  if (typeof wallet === "string") {
    summary.sync_error_wallet = { type: "label", value: wallet };
  }
  const code = first.code;
  if (typeof code === "string") summary.sync_error_code = textField(code);
  const message = first.message;
  if (typeof message === "string") {
    summary.sync_error_message = textField(message);
  }
  const hint = first.hint;
  if (typeof hint === "string" && hint) {
    summary.sync_error_hint = textField(hint);
  }
  if (typeof first.retryable === "boolean") {
    summary.sync_error_retryable = booleanField(first.retryable);
  }

  const details = first.details;
  if (!isRecord(details)) return;
  const phase = details.phase;
  if (typeof phase === "string") summary.sync_error_phase = textField(phase);
  const errorType = details.error_type;
  if (typeof errorType === "string") {
    summary.sync_error_type = textField(errorType);
  }
  const backend = details.backend;
  if (typeof backend === "string") {
    summary.sync_error_backend = { type: "label", value: backend };
  }
  const backendKind = details.backend_kind;
  if (typeof backendKind === "string") {
    summary.sync_error_backend_kind = textField(backendKind);
  }
  const chain = details.chain;
  if (typeof chain === "string") summary.sync_error_chain = textField(chain);
  const network = details.network;
  if (typeof network === "string") {
    summary.sync_error_network = textField(network);
  }
  if (typeof details.has_backend_url === "boolean") {
    summary.sync_error_has_backend_url = booleanField(details.has_backend_url);
  }
}

function withDaemonLogging(
  transport: DaemonTransport,
  source: string,
): DaemonTransport {
  return {
    async invoke<T = unknown>(
      req: DaemonRequest,
    ): Promise<DaemonEnvelope<T>> {
      // The log bridge polls ui.logs.snapshot every few seconds to fold the
      // daemon ring into this one; logging the polls would flood both rings.
      if (req.kind === "ui.logs.snapshot") {
        return transport.invoke<T>(requestWithId(req));
      }
      const request = requestWithId(req);
      recordDaemonLog(
        "debug",
        source,
        "Daemon invoke started",
        summarizeRequestFields(request),
      );
      try {
        const envelope = await transport.invoke<T>(request);
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
            ...summarizeRequestFields(request),
            error_message: textField(
              error instanceof Error ? error.message : String(error),
            ),
          },
        );
        throw error;
      }
    },
    async stream<T = unknown, R = unknown>(
      req: DaemonRequest,
      options?: DaemonStreamOptions<R>,
    ): Promise<DaemonEnvelope<T>> {
      if (req.kind === "ui.logs.snapshot") {
        return transport.stream<T, R>(requestWithId(req), options);
      }
      const request = requestWithId(req);
      recordDaemonLog(
        "debug",
        source,
        "Daemon stream started",
        summarizeRequestFields(request),
      );
      try {
        const envelope = await transport.stream<T, R>(request, {
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
            ...summarizeRequestFields(request),
            error_message: textField(
              error instanceof Error ? error.message : String(error),
            ),
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

export async function openAttachmentFile(path: string): Promise<void> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Opening attachment files is available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_attachment_file", { path });
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

async function requestBridgeImportProject<T>(
  body: Record<string, unknown>,
): Promise<T> {
  const response = await fetch(IMPORT_PROJECT_BRIDGE_PATH, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = (await response.json()) as Record<string, unknown>;
  const error =
    payload.error && typeof payload.error === "object"
      ? (payload.error as { message?: string | null })
      : null;
  if (!response.ok || payload.kind === "error" || error) {
    const message =
      error?.message ?? `Project import failed with HTTP ${response.status}`;
    throw new Error(message || "Project import failed.");
  }
  return payload as T;
}

export async function selectImportProjectDirectory(): Promise<ImportProjectSelection | null> {
  if (DAEMON_MODE === "bridge") {
    const response = await requestBridgeImportProject<{
      selection: ImportProjectSelection | null;
    }>({ action: "select" });
    return response.selection;
  }
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Project import is available in the desktop app or dev bridge.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ImportProjectSelection | null>("select_import_project_directory");
}

async function activateImportProjectViaMode(
  dataRoot: string,
): Promise<ImportProjectSelection> {
  if (DAEMON_MODE === "bridge") {
    const response = await requestBridgeImportProject<{
      selection: ImportProjectSelection;
    }>({ action: "activate", dataRoot });
    return response.selection;
  }
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Project import is available in the desktop app or dev bridge.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ImportProjectSelection>("activate_import_project", {
    dataRoot,
  });
}

export async function activateImportProject(
  dataRoot: string,
): Promise<ImportProjectSelection> {
  if (activeImportProjectSelection?.dataRoot === dataRoot) {
    return activeImportProjectSelection;
  }
  if (activeImportProjectActivation?.dataRoot === dataRoot) {
    return activeImportProjectActivation.promise;
  }
  const generation = ++importProjectActivationGeneration;
  const promise = activateImportProjectViaMode(dataRoot);
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
  if (DAEMON_MODE === "mock") {
    return;
  }
  importProjectActivationGeneration += 1;
  activeImportProjectActivation = null;
  if (DAEMON_MODE === "bridge") {
    await requestBridgeImportProject<{ ok: boolean }>({ action: "clear" });
  } else {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("clear_import_project");
  }
  if (activeImportProjectSelection !== null) {
    activeImportProjectSelection = null;
    useUiStore.getState().bumpDaemonSession();
  }
}

export function canImportProjects(): boolean {
  return DAEMON_MODE === "tauri" || DAEMON_MODE === "bridge";
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
  options?: { requireExistingProject?: boolean },
): Promise<DaemonEnvelope> {
  if (DAEMON_MODE !== "tauri") {
    return {
      kind: "error",
      schema_version: 1,
      error: {
        code: "touch_id_unavailable",
        message: "Touch ID passphrase unlock is only available in the macOS desktop app.",
      },
    };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<DaemonEnvelope>("touch_id_unlock_passphrase_command", {
    dataRoot: dataRoot ?? null,
    requireExistingProject: options?.requireExistingProject ?? false,
  });
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

const TERMINAL_COMMAND_UNAVAILABLE: TerminalCommandStatus = {
  platform: "unsupported",
  available: false,
  installed: false,
  managed: false,
  needsRepair: false,
  conflict: false,
  pathOnPath: false,
  command: "kassiber",
  binDir: "",
  commandPath: "",
  targetPath: "",
  pathHint: "",
  message: "Terminal command installation is available in the desktop app.",
};

export async function terminalCommandStatus(): Promise<TerminalCommandStatus> {
  if (DAEMON_MODE !== "tauri") {
    return TERMINAL_COMMAND_UNAVAILABLE;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TerminalCommandStatus>("terminal_command_status_command");
}

export async function installTerminalCommand(): Promise<TerminalCommandStatus> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Terminal command installation is available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TerminalCommandStatus>("terminal_command_install_command");
}

export async function removeTerminalCommand(): Promise<TerminalCommandStatus> {
  if (DAEMON_MODE !== "tauri") {
    throw new Error("Terminal command removal is available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<TerminalCommandStatus>("terminal_command_remove_command");
}

/**
 * Subscribe to unsolicited daemon events (`daemon://event`). Resolves
 * with an unsubscribe function. The mock transport has no daemon and the
 * dev bridge logs events in the Vite terminal instead of pushing them to
 * the browser, so both return a no-op unsubscribe.
 */
export async function subscribeDaemonEvents<T = unknown>(
  onEvent: (record: DaemonEventRecord<T>) => void,
): Promise<() => void> {
  if (DAEMON_MODE !== "tauri") {
    return () => {};
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<DaemonEventRecord<T>>("daemon://event", (event) => {
    onEvent(event.payload);
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
