import {
  execFile,
  execFileSync,
  spawn,
  type ChildProcessWithoutNullStreams,
} from "node:child_process";
import {
  existsSync,
  readFileSync,
  realpathSync,
} from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import path from "node:path";
import { createInterface } from "node:readline";
// `vitest/config` re-exports vite's `defineConfig` and adds the `test` field;
// it is a superset, so the Vite build reads this config unchanged.
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { inspectImportProjectDirectory } from "./vite/importProject";

const DAEMON_BRIDGE_PATH = "/__kassiber__/daemon";
const DAEMON_BRIDGE_STREAM_PATH = "/__kassiber__/daemon/stream";
const FILE_PICKER_BRIDGE_PATH = "/__kassiber__/pick-file";
const IMPORT_PROJECT_BRIDGE_PATH = "/__kassiber__/import-project";
const BRIDGE_REQUEST_TIMEOUT_MS = 10 * 60 * 1000;
const UI_ROOT = __dirname;
const NODE_MODULES_REALPATH = (() => {
  const nodeModulesPath = path.resolve(UI_ROOT, "node_modules");
  return existsSync(nodeModulesPath)
    ? realpathSync(nodeModulesPath)
    : nodeModulesPath;
})();
const ALLOWED_BRIDGE_KINDS = new Set([
  "status",
  "ui.logs.snapshot",
  "ui.overview.snapshot",
  "ui.workspace.overview.snapshot",
  "ui.transactions.list",
  "ui.transactions.metadata.update",
  "ui.transactions.resolve",
  "ui.transactions.history",
  "ui.transactions.history.revert",
  "ui.activity.history",
  "ui.activity.stale",
  "ui.attachments.list",
  "ui.attachments.add",
  "ui.attachments.copy",
  "ui.attachments.rename",
  "ui.attachments.remove",
  "ui.attachments.open",
  "ui.wallets.list",
  "ui.backends.list",
  "ui.backends.options",
  "ui.backends.public_defaults",
  "ui.backends.settings.list",
  "ui.backends.create",
  "ui.backends.update",
  "ui.backends.delete",
  "ui.backends.set_default",
  "ui.backends.electrum.test",
  "ui.backends.http.test",
  "ui.profiles.snapshot",
  "ui.onboarding.complete",
  "ui.profiles.create",
  "ui.profiles.rename",
  "ui.profiles.update",
  "ui.profiles.switch",
  "ui.profiles.reset_data",
  "ui.reports.capital_gains",
  "ui.reports.summary",
  "ui.reports.balance_sheet",
  "ui.reports.portfolio_summary",
  "ui.reports.balance_history",
  "ui.reports.tax_summary",
  "ui.reports.exit_tax_preview",
  "ui.reports.export_exit_tax_pdf",
  "ui.reports.export_exit_tax_xlsx",
  "ui.reports.export_pdf",
  "ui.reports.export_summary_pdf",
  "ui.reports.export_csv",
  "ui.reports.export_xlsx",
  "ui.reports.export_capital_gains_csv",
  "ui.reports.export_austrian_e1kv_pdf",
  "ui.reports.export_austrian_e1kv_xlsx",
  "ui.reports.export_austrian_e1kv_csv",
  "ui.reports.export_audit_package",
  "ui.transactions.export_csv",
  "ui.transactions.export_xlsx",
  "ui.journals.snapshot",
  "ui.journals.events.list",
  "ui.journals.quarantine",
  "ui.journals.transfers.list",
  "ui.journals.process",
  "ui.transfers.suggest",
  "ui.transfers.list",
  "ui.transfers.payouts.list",
  "ui.transfers.payouts.create",
  "ui.transfers.payouts.delete",
  "ui.transfers.pair",
  "ui.transfers.unpair",
  "ui.transfers.bulk_pair",
  "ui.transfers.dismiss",
  "ui.transfers.rules.list",
  "ui.transfers.rules.create",
  "ui.transfers.rules.delete",
  "ui.transfers.rules.set_enabled",
  "ui.transfers.rules.apply",
  "ui.saved_views.list",
  "ui.saved_views.create",
  "ui.saved_views.delete",
  "ui.rates.summary",
  "ui.rates.coverage",
  "ui.rates.kraken_csv.import",
  "ui.rates.latest",
  "ui.rates.rebuild",
  "ui.maintenance.settings",
  "ui.maintenance.configure",
  "ui.maintenance.run",
  "ui.workspace.health",
  "ui.workspace.freshness.run",
  "ui.workspace.create",
  "ui.workspace.rename",
  "ui.workspace.delete",
  "ui.secrets.init",
  "ui.secrets.change_passphrase",
  "ui.next_actions",
  "ui.wallets.utxos",
  "ui.wallets.create",
  "ui.wallets.import_file",
  "ui.wallets.import_samourai",
  "ui.wallets.preview_descriptor",
  "ui.wallets.identify",
  "ui.wallets.identify_onchain",
  "ui.connections.sources",
  "ui.connections.btcpay.create",
  "ui.connections.btcpay.discover",
  "ui.connections.btcpay.test",
  "ui.connections.node.snapshot",
  "ui.reports.lightning_profitability",
  "ui.metadata.bip329.import",
  "ui.wallets.update",
  "ui.wallets.delete",
  "ui.wallets.sync",
  "ui.freshness.status",
  "ui.freshness.configure",
  "ui.freshness.run",
  "ui.freshness.cancel",
  "ui.freshness.pause",
  "ui.freshness.resume",
  "daemon.lock",
  "daemon.unlock",
  // AI provider config and chat kinds. The bridge keeps one daemon process so
  // ai.chat, cancel, and consent all route through the same active registry.
  "ai.providers.list",
  "ai.providers.get",
  "ai.providers.create",
  "ai.providers.update",
  "ai.providers.set_api_key",
  "ai.providers.move_api_key",
  "ai.providers.delete",
  "ai.providers.set_default",
  "ai.providers.clear_default",
  "ai.providers.acknowledge",
  "ai.list_models",
  "ai.test_connection",
  "ai.chat",
  "ai.chat.cancel",
  "ai.tool_call.consent",
  "ui.chat.sessions.list",
  "ui.chat.sessions.get",
  "ui.chat.sessions.delete",
  "ui.chat.sessions.clear",
  "ui.chat.history.configure",
  "ui.source_funds.preview",
  "ui.source_funds.cases.save",
  "ui.source_funds.cases.list",
  "ui.source_funds.sources.list",
  "ui.source_funds.sources.create",
  "ui.source_funds.sources.attach",
  "ui.source_funds.links.list",
  "ui.source_funds.links.create",
  "ui.source_funds.links.review",
  "ui.source_funds.links.bulk_review",
  "ui.source_funds.links.attach",
  "ui.source_funds.suggest",
  "ui.source_funds.evidence.list",
  "ui.source_funds.export_pdf",
  "ui.source_funds.coverage",
  "ui.source_funds.recipients.list",
  "ui.source_funds.recipients.create",
  "ui.source_funds.recipients.update",
  "ui.source_funds.recipients.delete",
  "ui.audit.evidence.summary",
  "ui.btcpay.provenance.sync",
  "ui.btcpay.provenance.list",
  "ui.btcpay.provenance.suggest",
  "ui.btcpay.provenance.links",
  "ui.btcpay.provenance.review",
  "ui.transactions.commercial_context",
  "ui.documents.list",
  "ui.documents.create",
  "ui.documents.attach",
]);
/**
 * Kinds the bridge accepts on the stream endpoint. AI chat is here because
 * it MUST stream (the daemon spawns a worker thread); wallet sync and
 * freshness runs are here because they CAN stream progress envelopes when the
 * caller wants them. The invoke endpoint also accepts them — it just drops the
 * interleaved progress envelopes — so existing useDaemonMutation callers keep
 * working.
 */
const STREAM_CAPABLE_BRIDGE_KINDS = new Set([
  "ai.chat",
  "ui.wallets.sync",
  "ui.freshness.run",
  "ui.workspace.freshness.run",
]);
const STREAM_ONLY_BRIDGE_KINDS = new Set(["ai.chat"]);

function makeBridgeRequestId(prefix = "bridge") {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function redactBridgeText(value: string): string {
  const words: string[] = [];
  let redactNext = false;
  for (const word of value.split(/\s+/).filter(Boolean)) {
    if (redactNext) {
      words.push("[redacted]");
      redactNext = false;
      continue;
    }
    const lowered = word.toLowerCase();
    if (lowered.startsWith("sk-")) {
      words.push("[redacted]");
      continue;
    }
    if (
      /\b(?:xpub|ypub|zpub|tpub|upub|vpub|xprv|yprv|zprv|tprv|uprv|vprv)[A-Za-z0-9]{20,}\b/i.test(
        word,
      ) ||
      /\b(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)/i.test(word)
    ) {
      words.push("[redacted]");
      continue;
    }
    if (lowered === "bearer") {
      words.push("Bearer");
      redactNext = true;
      continue;
    }
    const redacted = word.replace(
      /\b(api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase(?:_secret)?|password|secret|token)\b([:=])([^\s,;"']+)/gi,
      "$1$2[redacted]",
    );
    words.push(redacted);
  }
  return words.join(" ");
}

function firstHeaderValue(header: string | string[] | undefined) {
  return Array.isArray(header) ? header[0] : header;
}

export function isLoopbackHost(hostHeader: string | string[] | undefined) {
  const rawHost = firstHeaderValue(hostHeader);
  if (!rawHost) return false;

  const host = rawHost.startsWith("[")
    ? rawHost.slice(1, rawHost.indexOf("]"))
    : rawHost.split(":")[0];

  return (
    host === "localhost" ||
    host === "::1" ||
    host === "0:0:0:0:0:0:0:1" ||
    /^127(?:\.\d{1,3}){3}$/.test(host)
  );
}

export function isAllowedBridgeOrigin(
  originHeader: string | string[] | undefined,
  hostHeader: string | string[] | undefined,
) {
  const rawOrigin = firstHeaderValue(originHeader);
  const rawHost = firstHeaderValue(hostHeader);
  if (!rawOrigin || !rawHost) return false;

  let origin: URL;
  try {
    origin = new URL(rawOrigin);
  } catch {
    return false;
  }

  return (
    isLoopbackHost(origin.host) &&
    origin.host.toLowerCase() === rawHost.toLowerCase()
  );
}

function readBody(req: IncomingMessage) {
  return new Promise<string>((resolve, reject) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
      if (body.length > 1_000_000) {
        reject(new Error("request body too large"));
        req.destroy();
      }
    });
    req.on("end", () => resolve(body));
    req.on("error", reject);
  });
}

interface PendingBridgeRequest {
  kind: string;
  onRecord?: (payload: Record<string, unknown>) => void;
  resolve: (payload: Record<string, unknown>) => void;
  reject: (error: Error) => void;
  timeout: NodeJS.Timeout;
}

class DaemonBridgeSupervisor {
  private child: ChildProcessWithoutNullStreams | null = null;
  private dataRoot: string | null = null;
  private readonly pending = new Map<string, PendingBridgeRequest>();
  private stderrTail = "";

  invoke(request: Record<string, unknown>) {
    return this.dispatch(request);
  }

  stream(
    request: Record<string, unknown>,
    onRecord: (payload: Record<string, unknown>) => void,
  ) {
    return this.dispatch(request, onRecord);
  }

  cancelAiChat(targetRequestId: string) {
    return this.invoke({
      kind: "ai.chat.cancel",
      request_id: makeBridgeRequestId("bridge-cancel"),
      args: { target_request_id: targetRequestId },
    });
  }

  setDataRoot(dataRoot: string | null) {
    if (this.dataRoot === dataRoot) return;
    this.dataRoot = dataRoot;
    this.shutdown();
  }

  shutdown() {
    const child = this.child;
    this.child = null;
    if (!child || child.killed) return;
    const requestId = `bridge-shutdown-${Date.now()}`;
    try {
      child.stdin.write(
        `${JSON.stringify({
          request_id: requestId,
          kind: "daemon.shutdown",
        })}\n`,
      );
    } catch {
      // Best-effort cleanup for a dev-only bridge.
    }
    setTimeout(() => {
      if (!child.killed) {
        child.kill();
      }
    }, 500).unref();
  }

  private dispatch(
    request: Record<string, unknown>,
    onRecord?: (payload: Record<string, unknown>) => void,
  ) {
    const child = this.ensureStarted();
    const requestId =
      typeof request.request_id === "string" && request.request_id
        ? request.request_id
        : makeBridgeRequestId();
    const kind = typeof request.kind === "string" ? request.kind : "";

    return new Promise<Record<string, unknown>>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new Error(`daemon bridge request ${requestId} timed out`));
      }, BRIDGE_REQUEST_TIMEOUT_MS);
      timeout.unref();

      this.pending.set(requestId, {
        kind,
        onRecord,
        resolve,
        reject,
        timeout,
      });

      const line = `${JSON.stringify({ ...request, request_id: requestId })}\n`;
      child.stdin.write(line, (error) => {
        if (!error) return;
        this.pending.delete(requestId);
        clearTimeout(timeout);
        reject(error);
      });
    });
  }

  private ensureStarted() {
    if (this.child && !this.child.killed) {
      return this.child;
    }

    const repoRoot = path.resolve(__dirname, "..");
    const args = ["run", "python", "-m", "kassiber"];
    if (this.dataRoot) {
      args.push("--data-root", this.dataRoot);
    }
    args.push("daemon");
    const child = spawn("uv", args, {
      cwd: repoRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child = child;
    this.stderrTail = "";

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      this.stderrTail = redactBridgeText((this.stderrTail + chunk).slice(-8_000));
    });
    child.on("error", (error) => this.failPending(error));
    child.on("close", (code, signal) => {
      this.child = null;
      this.failPending(
        new Error(
          this.stderrTail.trim() ||
            `daemon bridge exited with code ${code ?? "null"} signal ${signal ?? "null"}`,
        ),
      );
    });

    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => this.handleDaemonLine(line));

    return child;
  }

  private handleDaemonLine(line: string) {
    const trimmed = line.trim();
    if (!trimmed) return;

    let payload: Record<string, unknown>;
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        return;
      }
      payload = parsed as Record<string, unknown>;
    } catch {
      return;
    }

    const requestId = payload.request_id;
    if (requestId === null || requestId === undefined) {
      // Unsolicited daemon events (`event: true`, no request_id) have no
      // push channel over the HTTP bridge, so surface them in the dev
      // server terminal instead of dropping them silently. Anything else
      // without a request_id (besides the startup `daemon.ready`) is a
      // daemon bug worth seeing in dev.
      const kind = typeof payload.kind === "string" ? payload.kind : "<unknown>";
      if (payload.event === true) {
        console.info(
          `[kassiber bridge] daemon event ${kind}: ${redactBridgeText(
            JSON.stringify(payload),
          ).slice(0, 2_000)}`,
        );
      } else if (kind !== "daemon.ready") {
        console.warn(
          `[kassiber bridge] dropping daemon record without request_id (kind ${kind})`,
        );
      }
      return;
    }
    const pending = this.pending.get(String(requestId));
    if (!pending) {
      return;
    }

    const kind = typeof payload.kind === "string" ? payload.kind : "";
    if (kind === pending.kind || kind === "error" || kind === "auth_required") {
      this.pending.delete(String(requestId));
      clearTimeout(pending.timeout);
      pending.resolve(payload);
      return;
    }

    pending.onRecord?.(payload);
  }

  private failPending(error: Error) {
    for (const [requestId, pending] of this.pending) {
      clearTimeout(pending.timeout);
      pending.reject(error);
      this.pending.delete(requestId);
    }
  }
}

function daemonBridgePlugin() {
  return {
    name: "kassiber-daemon-bridge",
    configureServer(server: import("vite").ViteDevServer) {
      const supervisor = new DaemonBridgeSupervisor();
      server.httpServer?.once("close", () => supervisor.shutdown());

      server.middlewares.use(async (req, res, next) => {
        const pathname = (req.url ?? "").split("?")[0];
        if (pathname === DAEMON_BRIDGE_STREAM_PATH) {
          await handleBridgeStream(req, res, supervisor);
          return;
        }
        if (pathname === DAEMON_BRIDGE_PATH) {
          await handleBridgeInvoke(req, res, supervisor);
          return;
        }
        if (pathname === FILE_PICKER_BRIDGE_PATH) {
          await handleBridgeFilePicker(req, res);
          return;
        }
        if (pathname === IMPORT_PROJECT_BRIDGE_PATH) {
          await handleBridgeImportProject(req, res, supervisor);
          return;
        }
        next();
      });
    },
  };
}

function execFileText(command: string, args: string[]) {
  return new Promise<string>((resolve, reject) => {
    execFile(command, args, { encoding: "utf8" }, (error, stdout, stderr) => {
      if (error) {
        reject(new Error(stderr.trim() || error.message));
        return;
      }
      resolve(stdout.trim());
    });
  });
}

function appleScriptString(value: string) {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function normalizeFilePickerFilters(filters: unknown) {
  if (!Array.isArray(filters)) return [];
  return filters
    .flatMap((filter) => {
      if (!filter || typeof filter !== "object" || Array.isArray(filter)) return [];
      const extensions = (filter as { extensions?: unknown }).extensions;
      if (!Array.isArray(extensions)) return [];
      return extensions
        .filter((extension): extension is string => typeof extension === "string")
        .map((extension) => extension.trim().replace(/^\./, "").toLowerCase())
        .filter((extension) => /^[a-z0-9]+$/.test(extension));
    })
    .filter((extension, index, all) => all.indexOf(extension) === index);
}

async function pickFileViaNativeBridge(
  request: Record<string, unknown>,
): Promise<string[]> {
  const title =
    typeof request.title === "string" && request.title.trim()
      ? request.title.trim()
      : "Choose a file";
  const directory = request.directory === true;
  const multiple = request.multiple === true;
  if (process.platform === "darwin") {
    const prompt = appleScriptString(title);
    const script = directory
      ? `POSIX path of (choose folder with prompt ${prompt})`
      : (() => {
          const extensions = normalizeFilePickerFilters(request.filters);
          const ofType = extensions.length
            ? ` of type {${extensions.map(appleScriptString).join(", ")}}`
            : "";
          if (multiple) {
            // Emit one POSIX path per line so we can split on the client.
            return `set theChoices to (choose file with prompt ${prompt}${ofType} with multiple selections allowed)
set thePaths to {}
repeat with anItem in theChoices
  set end of thePaths to POSIX path of anItem
end repeat
set AppleScript's text item delimiters to linefeed
return thePaths as text`;
          }
          return `POSIX path of (choose file with prompt ${prompt}${ofType})`;
        })();
    const raw = await execFileText("osascript", ["-e", script]);
    return raw
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
  }

  const zenityArgs = [
    "--file-selection",
    ...(directory ? ["--directory"] : []),
    ...(multiple ? ["--multiple", "--separator=\n"] : []),
    `--title=${title}`,
  ];
  try {
    const raw = await execFileText("zenity", zenityArgs);
    return raw
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
  } catch {
    throw new Error("No supported native file picker is available for the dev bridge.");
  }
}

async function handleBridgeFilePicker(req: IncomingMessage, res: ServerResponse) {
  if (!isLoopbackHost(req.headers.host)) {
    writeJsonError(
      res,
      403,
      "bridge_forbidden_host",
      "file picker bridge only accepts loopback hosts",
    );
    return;
  }
  if (req.method !== "POST") {
    writeJsonError(res, 405, "method_not_allowed", "file picker bridge only accepts POST");
    return;
  }
  if (!isAllowedBridgeOrigin(req.headers.origin, req.headers.host)) {
    writeJsonError(
      res,
      403,
      "bridge_forbidden_origin",
      "file picker bridge only accepts same-origin browser requests",
    );
    return;
  }

  let request: Record<string, unknown>;
  try {
    const parsed = JSON.parse(await readBody(req)) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("request body must be a JSON object");
    }
    request = parsed as Record<string, unknown>;
  } catch (error) {
    writeJsonError(
      res,
      400,
      "invalid_file_picker_request",
      error instanceof Error ? error.message : String(error),
    );
    return;
  }

  try {
    const paths = await pickFileViaNativeBridge(request);
    if (request.multiple === true) {
      writeJson(res, 200, { paths });
    } else {
      writeJson(res, 200, { path: paths[0] ?? null });
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const cancelled =
      /user canceled/i.test(message) ||
      /User cancelled/i.test(message) ||
      /cancel/i.test(message);
    if (cancelled) {
      writeJson(res, 200, request.multiple === true ? { paths: [] } : { path: null });
    } else {
      writeJson(
        res,
        200,
        request.multiple === true
          ? { paths: [], error: message }
          : { path: null, error: message },
      );
    }
  }
}

async function handleBridgeImportProject(
  req: IncomingMessage,
  res: ServerResponse,
  supervisor: DaemonBridgeSupervisor,
) {
  if (!isLoopbackHost(req.headers.host)) {
    writeJsonError(
      res,
      403,
      "bridge_forbidden_host",
      "project import bridge only accepts loopback hosts",
    );
    return;
  }
  if (req.method !== "POST") {
    writeJsonError(
      res,
      405,
      "method_not_allowed",
      "project import bridge only accepts POST",
    );
    return;
  }
  if (!isAllowedBridgeOrigin(req.headers.origin, req.headers.host)) {
    writeJsonError(
      res,
      403,
      "bridge_forbidden_origin",
      "project import bridge only accepts same-origin browser requests",
    );
    return;
  }

  let request: Record<string, unknown>;
  try {
    const parsed = JSON.parse(await readBody(req)) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("request body must be a JSON object");
    }
    request = parsed as Record<string, unknown>;
  } catch (error) {
    writeJsonError(
      res,
      400,
      "invalid_project_import_request",
      error instanceof Error ? error.message : String(error),
    );
    return;
  }

  try {
    const action = request.action;
    if (action === "select") {
      const paths = await pickFileViaNativeBridge({
        title: "Open Kassiber books",
        directory: true,
        multiple: false,
      });
      if (!paths[0]) {
        writeJson(res, 200, { selection: null });
        return;
      }
      writeJson(res, 200, {
        selection: inspectImportProjectDirectory(paths[0]),
      });
      return;
    }
    if (action === "activate") {
      if (typeof request.dataRoot !== "string" || !request.dataRoot.trim()) {
        throw new Error("dataRoot is required.");
      }
      const selection = inspectImportProjectDirectory(request.dataRoot);
      supervisor.setDataRoot(selection.dataRoot);
      writeJson(res, 200, { selection });
      return;
    }
    if (action === "clear") {
      supervisor.setDataRoot(null);
      writeJson(res, 200, { ok: true });
      return;
    }
    writeJsonError(
      res,
      400,
      "invalid_project_import_action",
      "project import action must be select, activate, or clear",
    );
  } catch (error) {
    writeJsonError(
      res,
      500,
      "bridge_project_import_failed",
      redactBridgeText(error instanceof Error ? error.message : String(error)),
      true,
    );
  }
}

async function handleBridgeInvoke(
  req: IncomingMessage,
  res: ServerResponse,
  supervisor: DaemonBridgeSupervisor,
) {
  const request = await readBridgeRequest(req, res);
  if (!request) return;

  const kind = typeof request.kind === "string" ? request.kind : "";
  if (STREAM_ONLY_BRIDGE_KINDS.has(kind)) {
    writeJsonError(
      res,
      400,
      "bridge_stream_required",
      `daemon kind ${JSON.stringify(kind)} requires the bridge stream endpoint`,
    );
    return;
  }

  try {
    writeJson(res, 200, await supervisor.invoke(request));
  } catch (error) {
    writeJsonError(
      res,
      500,
      "bridge_daemon_failed",
      redactBridgeText(error instanceof Error ? error.message : String(error)),
      true,
    );
  }
}

async function handleBridgeStream(
  req: IncomingMessage,
  res: ServerResponse,
  supervisor: DaemonBridgeSupervisor,
) {
  const request = await readBridgeRequest(req, res);
  if (!request) return;

  const kind = typeof request.kind === "string" ? request.kind : "";
  if (!STREAM_CAPABLE_BRIDGE_KINDS.has(kind)) {
    writeJsonError(
      res,
      400,
      "bridge_stream_unsupported",
      `daemon kind ${JSON.stringify(kind)} is not streamable`,
    );
    return;
  }

  res.statusCode = 200;
  res.setHeader("content-type", "application/x-ndjson");
  res.setHeader("cache-control", "no-cache");

  const streamRequestId =
    typeof request.request_id === "string" && request.request_id
      ? request.request_id
      : makeBridgeRequestId("bridge-stream");
  request.request_id = streamRequestId;
  let completed = false;
  res.once("close", () => {
    if (completed || kind !== "ai.chat") return;
    void supervisor.cancelAiChat(streamRequestId).catch(() => undefined);
  });

  try {
    const terminal = await supervisor.stream(request, (record) => {
      if (!res.destroyed && !res.writableEnded) {
        writeNdjson(res, record);
      }
    });
    completed = true;
    if (!res.destroyed && !res.writableEnded) {
      writeNdjson(res, terminal);
      res.end();
    }
  } catch (error) {
    completed = true;
    if (!res.destroyed && !res.writableEnded) {
      writeNdjson(res, {
        kind: "error",
        schema_version: 1,
        request_id: request.request_id,
        error: {
          code: "bridge_daemon_failed",
          message: redactBridgeText(error instanceof Error ? error.message : String(error)),
          retryable: true,
        },
      });
      res.end();
    }
  }
}

async function readBridgeRequest(req: IncomingMessage, res: ServerResponse) {
  if (!isLoopbackHost(req.headers.host)) {
    writeJsonError(
      res,
      403,
      "bridge_forbidden_host",
      "daemon bridge only accepts loopback hosts",
    );
    return null;
  }

  if (req.method !== "POST") {
    writeJsonError(
      res,
      405,
      "method_not_allowed",
      "daemon bridge only accepts POST",
    );
    return null;
  }

  if (!isAllowedBridgeOrigin(req.headers.origin, req.headers.host)) {
    writeJsonError(
      res,
      403,
      "bridge_forbidden_origin",
      "daemon bridge only accepts same-origin browser requests",
    );
    return null;
  }

  let request: Record<string, unknown>;
  try {
    const parsed = JSON.parse(await readBody(req)) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("request body must be a JSON object");
    }
    request = parsed as Record<string, unknown>;
  } catch (error) {
    writeJsonError(
      res,
      400,
      "invalid_bridge_request",
      error instanceof Error ? error.message : String(error),
    );
    return null;
  }

  const kind = typeof request.kind === "string" ? request.kind : "";
  if (!ALLOWED_BRIDGE_KINDS.has(kind)) {
    writeJsonError(
      res,
      403,
      "kind_not_allowed",
      `daemon kind ${JSON.stringify(kind)} is not allowed by the dev bridge`,
    );
    return null;
  }
  return request;
}

function writeJson(
  res: ServerResponse,
  statusCode: number,
  payload: Record<string, unknown>,
) {
  res.statusCode = statusCode;
  res.setHeader("content-type", "application/json");
  res.end(JSON.stringify(payload));
}

function writeJsonError(
  res: ServerResponse,
  statusCode: number,
  code: string,
  message: string,
  retryable = false,
) {
  writeJson(res, statusCode, {
    kind: "error",
    schema_version: 1,
    error: {
      code,
      message,
      retryable,
    },
  });
}

function writeNdjson(res: ServerResponse, payload: Record<string, unknown>) {
  res.write(`${JSON.stringify(payload)}\n`);
}

function resolveAppCommit(): string {
  const envCommit = process.env.KASSIBER_BUILD_COMMIT ?? process.env.GITHUB_SHA;
  if (envCommit) {
    return envCommit.slice(0, 12);
  }

  try {
    return execFileSync("git", ["rev-parse", "--short=12", "HEAD"], {
      encoding: "utf8",
    }).trim();
  } catch {
    return "unknown";
  }
}

function resolveAppVersion(): string {
  if (process.env.KASSIBER_BUILD_VERSION) {
    return process.env.KASSIBER_BUILD_VERSION;
  }

  try {
    const pkg = JSON.parse(
      readFileSync(path.resolve(UI_ROOT, "package.json"), "utf8"),
    ) as { version?: unknown };
    return typeof pkg.version === "string" ? pkg.version : "unknown";
  } catch {
    return "unknown";
  }
}

export default defineConfig({
  define: {
    __APP_COMMIT__: JSON.stringify(resolveAppCommit()),
    __APP_VERSION__: JSON.stringify(resolveAppVersion()),
  },
  plugins: [daemonBridgePlugin(), react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    fs: {
      allow: [UI_ROOT, NODE_MODULES_REALPATH],
    },
  },
  test: {
    // Initialize i18next before any test file so `useTranslation` resolves
    // real strings under `renderToStaticMarkup`. Environment stays on the
    // vitest default (node); no test currently needs a DOM.
    setupFiles: ["./vitest.setup.ts"],
  },
});
