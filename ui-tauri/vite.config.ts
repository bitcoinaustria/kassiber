import { spawn } from "node:child_process";
import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const DAEMON_BRIDGE_PATH = "/__kassiber__/daemon";
const ALLOWED_BRIDGE_KINDS = new Set([
  "status",
  "ui.overview.snapshot",
  "ui.transactions.list",
  "ui.profiles.snapshot",
  "ui.reports.capital_gains",
  "ui.journals.snapshot",
  "ui.wallets.sync",
]);

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

function readBody(req: import("node:http").IncomingMessage) {
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

function daemonBridgePlugin() {
  return {
    name: "kassiber-daemon-bridge",
    configureServer(server: import("vite").ViteDevServer) {
      server.middlewares.use(DAEMON_BRIDGE_PATH, async (req, res) => {
        if (!isLoopbackHost(req.headers.host)) {
          res.statusCode = 403;
          res.setHeader("content-type", "application/json");
          res.end(
            JSON.stringify({
              kind: "error",
              schema_version: 1,
              error: {
                code: "bridge_forbidden_host",
                message: "daemon bridge only accepts loopback hosts",
                retryable: false,
              },
            }),
          );
          return;
        }

        if (req.method !== "POST") {
          res.statusCode = 405;
          res.setHeader("content-type", "application/json");
          res.end(JSON.stringify({ error: "method_not_allowed" }));
          return;
        }

        let request: Record<string, unknown>;
        try {
          request = JSON.parse(await readBody(req));
        } catch (error) {
          res.statusCode = 400;
          res.setHeader("content-type", "application/json");
          res.end(
            JSON.stringify({
              kind: "error",
              schema_version: 1,
              error: {
                code: "invalid_bridge_request",
                message: error instanceof Error ? error.message : String(error),
                retryable: false,
              },
            }),
          );
          return;
        }

        const kind = typeof request.kind === "string" ? request.kind : "";
        if (!ALLOWED_BRIDGE_KINDS.has(kind)) {
          res.statusCode = 403;
          res.setHeader("content-type", "application/json");
          res.end(
            JSON.stringify({
              kind: "error",
              schema_version: 1,
              error: {
                code: "kind_not_allowed",
                message: `daemon kind ${JSON.stringify(kind)} is not allowed by the dev bridge`,
                retryable: false,
              },
            }),
          );
          return;
        }

        try {
          const response = await invokeDaemon(request);
          res.statusCode = 200;
          res.setHeader("content-type", "application/json");
          res.end(JSON.stringify(response));
        } catch (error) {
          res.statusCode = 500;
          res.setHeader("content-type", "application/json");
          res.end(
            JSON.stringify({
              kind: "error",
              schema_version: 1,
              error: {
                code: "bridge_daemon_failed",
                message: error instanceof Error ? error.message : String(error),
                retryable: true,
              },
            }),
          );
        }
      });
    },
  };
}

function invokeDaemon(request: Record<string, unknown>) {
  return new Promise<unknown>((resolve, reject) => {
    const repoRoot = path.resolve(__dirname, "..");
    const requestId =
      typeof request.request_id === "string" && request.request_id
        ? request.request_id
        : `bridge-${Date.now()}`;
    const child = spawn("uv", ["run", "python", "-m", "kassiber", "daemon"], {
      cwd: repoRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("close", () => {
      const lines = stdout
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean);
      const response = lines
        .map((line) => JSON.parse(line) as Record<string, unknown>)
        .find((payload) => payload.request_id === requestId);
      if (response) {
        resolve(response);
        return;
      }
      reject(
        new Error(
          stderr.trim() ||
            "daemon exited without returning a matching request_id response",
        ),
      );
    });
    child.stdin.end(
      `${JSON.stringify({ ...request, request_id: requestId })}\n${JSON.stringify({
        request_id: `${requestId}-shutdown`,
        kind: "daemon.shutdown",
      })}\n`,
    );
  });
}

export default defineConfig({
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
