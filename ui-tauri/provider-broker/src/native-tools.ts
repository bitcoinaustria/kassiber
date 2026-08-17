import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createConnection, createServer, type Server as NetServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import type { BrokerToolDefinition, BrokerToolResult } from "./protocol.js";
import { writeEvent } from "./protocol.js";

const MAX_BRIDGE_MESSAGE_BYTES = 2_000_000;

type BridgeRequest = {
  call_id: string;
  name: string;
  arguments: Record<string, unknown>;
};

// A loopback TCP port is reachable by every local process, so it would need a
// shared secret — and the only place to hand one to the child MCP process is
// argv, which is world-readable on Linux. The socket lives in a 0700 directory
// instead, so the operating system enforces the boundary and there is no
// secret to leak. Windows gets a named pipe with an unguessable name.
function bridgeSocketPath(directory: string): string {
  return process.platform === "win32"
    ? `\\\\.\\pipe\\kassiber-tool-bridge-${randomUUID()}`
    : join(directory, "bridge.sock");
}

type PendingTool = {
  resolve: (output: string) => void;
  reject: (error: Error) => void;
};

export class NativeToolBridge {
  private readonly pending = new Map<string, PendingTool>();
  private readonly allowedNames: Set<string>;
  private constructor(
    private readonly server: NetServer,
    private readonly directory: string,
    readonly socketPath: string,
    tools: BrokerToolDefinition[],
  ) {
    this.allowedNames = new Set(tools.map((tool) => tool.name));
  }

  static async start(tools: BrokerToolDefinition[]): Promise<NativeToolBridge> {
    const directory = await mkdtemp(join(tmpdir(), "kassiber-ai-bridge-"));
    const socketPath = bridgeSocketPath(directory);
    const server = createServer((socket) => {
      let body = "";
      socket.setEncoding("utf8");
      socket.on("data", (chunk) => {
        body += chunk;
        if (body.length > MAX_BRIDGE_MESSAGE_BYTES) {
          socket.destroy();
          return;
        }
        const newline = body.indexOf("\n");
        if (newline < 0) return;
        const line = body.slice(0, newline);
        body = "";
        let request: BridgeRequest;
        try {
          request = JSON.parse(line) as BridgeRequest;
        } catch {
          socket.destroy();
          return;
        }
        bridge
          .request(request.name, request.arguments, request.call_id)
          .then((output) => socket.end(`${JSON.stringify({ output })}\n`))
          .catch((error: unknown) =>
            socket.end(`${JSON.stringify({ error: String(error) })}\n`),
          );
      });
    });
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(socketPath, resolve);
    });
    const bridge = new NativeToolBridge(server, directory, socketPath, tools);
    return bridge;
  }

  request(
    name: string,
    args: Record<string, unknown>,
    callId: string = randomUUID(),
  ): Promise<string> {
    if (!this.allowedNames.has(name)) {
      return Promise.reject(new Error("Provider requested a tool outside the advertised catalog."));
    }
    if (this.pending.has(callId)) {
      return Promise.reject(new Error("Provider repeated a pending tool call id."));
    }
    writeEvent({ type: "tool_call", call_id: callId, name, arguments: args });
    return new Promise<string>((resolve, reject) => {
      this.pending.set(callId, { resolve, reject });
    });
  }

  resolve(results: BrokerToolResult[]): void {
    for (const result of results) {
      const pending = this.pending.get(result.call_id);
      if (!pending) continue;
      this.pending.delete(result.call_id);
      pending.resolve(result.output);
    }
  }

  async close(): Promise<void> {
    for (const pending of this.pending.values()) {
      pending.reject(new Error("Kassiber closed the tool bridge."));
    }
    this.pending.clear();
    await new Promise<void>((resolve) => this.server.close(() => resolve()));
    await rm(this.directory, { recursive: true, force: true });
  }
}

export async function mcpCommand(
  cwd: string,
  tools: BrokerToolDefinition[],
  bridge: NativeToolBridge,
): Promise<string[]> {
  const manifest = join(cwd, "kassiber-mcp-tools.json");
  await writeFile(manifest, JSON.stringify(tools), { mode: 0o600 });
  const script = process.argv[1];
  if (!script) throw new Error("Kassiber broker script path is unavailable.");
  return [process.execPath, script, "mcp", bridge.socketPath, manifest];
}

async function forwardToolCall(
  socketPath: string,
  name: string,
  args: Record<string, unknown>,
): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const socket = createConnection({ path: socketPath });
    let body = "";
    socket.setEncoding("utf8");
    socket.once("connect", () => {
      socket.write(
        `${JSON.stringify({ call_id: randomUUID(), name, arguments: args })}\n`,
      );
    });
    socket.on("data", (chunk) => {
      body += chunk;
      if (body.length > MAX_BRIDGE_MESSAGE_BYTES) {
        socket.destroy();
        return;
      }
      const newline = body.indexOf("\n");
      if (newline < 0) return;
      let response: { output?: unknown; error?: unknown };
      try {
        response = JSON.parse(body.slice(0, newline)) as typeof response;
      } catch {
        reject(new Error("Kassiber returned an invalid tool result."));
        socket.destroy();
        return;
      }
      if (typeof response.output === "string") resolve(response.output);
      else reject(new Error(String(response.error || "Kassiber tool execution failed.")));
      socket.destroy();
    });
    socket.once("error", reject);
    socket.once("close", () => {
      if (!body.includes("\n")) reject(new Error("Kassiber tool bridge closed early."));
    });
  });
}

export async function runMcpServer(args: string[]): Promise<void> {
  const socketPath = args[0];
  const manifest = args[1];
  if (!socketPath || !manifest) {
    throw new Error("Invalid Kassiber MCP bridge arguments.");
  }
  const tools = JSON.parse(await readFile(manifest, "utf8")) as BrokerToolDefinition[];
  const server = new McpServer(
    { name: "kassiber", version: "1.0.0" },
    { instructions: "Typed, capability-scoped Kassiber accounting tools." },
  );
  for (const tool of tools) {
    const inputSchema = z.fromJSONSchema(tool.parameters);
    server.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema,
        annotations: {
          readOnlyHint: tool.read_only === true,
          destructiveHint: tool.destructive === true,
        },
      },
      async (toolArgs) => ({
        content: [
          {
            type: "text" as const,
            text: await forwardToolCall(
              socketPath,
              tool.name,
              toolArgs as Record<string, unknown>,
            ),
          },
        ],
      }),
    );
  }
  await server.connect(new StdioServerTransport());
}
