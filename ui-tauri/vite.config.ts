import {
  execFileSync,
  spawn,
  type ChildProcessWithoutNullStreams,
} from "node:child_process";
import type { IncomingMessage, ServerResponse } from "node:http";
import path from "node:path";
import { createInterface } from "node:readline";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const DAEMON_BRIDGE_PATH = "/__kassiber__/daemon";
const DAEMON_BRIDGE_STREAM_PATH = "/__kassiber__/daemon/stream";
const BRIDGE_REQUEST_TIMEOUT_MS = 10 * 60 * 1000;
const ALLOWED_BRIDGE_KINDS = new Set([
  "status",
  "ui.overview.snapshot",
  "ui.transactions.list",
  "ui.wallets.list",
  "ui.backends.list",
  "ui.profiles.snapshot",
  "ui.reports.capital_gains",
  "ui.journals.snapshot",
  "ui.journals.quarantine",
  "ui.journals.transfers.list",
  "ui.rates.summary",
  "ui.workspace.health",
  "ui.workspace.delete",
  "ui.secrets.init",
  "ui.secrets.change_passphrase",
  "ui.next_actions",
  "ui.wallets.sync",
  "daemon.lock",
  "daemon.unlock",
  // AI provider config and chat kinds. The bridge keeps one daemon process so
  // ai.chat, cancel, and consent all route through the same active registry.
  "ai.providers.list",
  "ai.providers.get",
  "ai.providers.create",
  "ai.providers.update",
  "ai.providers.delete",
  "ai.providers.set_default",
  "ai.providers.clear_default",
  "ai.providers.acknowledge",
  "ai.list_models",
  "ai.test_connection",
  "ai.chat",
  "ai.chat.cancel",
  "ai.tool_call.consent",
]);
const STREAMING_BRIDGE_KINDS = new Set(["ai.chat"]);

function makeBridgeRequestId(prefix = "bridge") {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function isLoopbackHost(hostHeader: string | string[] | undefined) {
  const rawHost = Array.isArray(hostHeader) ? hostHeader[0] : hostHeader;
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
    const child = spawn("uv", ["run", "python", "-m", "kassiber", "daemon"], {
      cwd: repoRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child = child;
    this.stderrTail = "";

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      this.stderrTail = (this.stderrTail + chunk).slice(-8_000);
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
      return;
    }
    const pending = this.pending.get(String(requestId));
    if (!pending) {
      return;
    }

    const kind = typeof payload.kind === "string" ? payload.kind : "";
    if (kind === pending.kind || kind === "error") {
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
        next();
      });
    },
  };
}

async function handleBridgeInvoke(
  req: IncomingMessage,
  res: ServerResponse,
  supervisor: DaemonBridgeSupervisor,
) {
  const request = await readBridgeRequest(req, res);
  if (!request) return;

  const kind = typeof request.kind === "string" ? request.kind : "";
  if (STREAMING_BRIDGE_KINDS.has(kind)) {
    writeJsonError(
      res,
      400,
      "bridge_stream_required",
      "use the bridge stream endpoint for ai.chat",
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
      error instanceof Error ? error.message : String(error),
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
  if (!STREAMING_BRIDGE_KINDS.has(kind)) {
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
    if (completed) return;
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
          message: error instanceof Error ? error.message : String(error),
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

export default defineConfig({
  define: {
    __APP_COMMIT__: JSON.stringify(resolveAppCommit()),
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
  },
});
