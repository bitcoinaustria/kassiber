import { randomUUID } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { createConnection, createServer, type Server as NetServer } from "node:net";
import { join } from "node:path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import type { BrokerToolDefinition, BrokerToolResult } from "./protocol.js";
import { writeEvent } from "./protocol.js";

const MAX_BRIDGE_MESSAGE_BYTES = 2_000_000;

type BridgeRequest = {
  nonce: string;
  call_id: string;
  name: string;
  arguments: Record<string, unknown>;
};

type PendingTool = {
  resolve: (output: string) => void;
  reject: (error: Error) => void;
};

export class NativeToolBridge {
  private readonly pending = new Map<string, PendingTool>();
  private readonly allowedNames: Set<string>;
  private constructor(
    private readonly server: NetServer,
    readonly port: number,
    readonly nonce: string,
    tools: BrokerToolDefinition[],
  ) {
    this.allowedNames = new Set(tools.map((tool) => tool.name));
  }

  static async start(tools: BrokerToolDefinition[]): Promise<NativeToolBridge> {
    const nonce = randomUUID();
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
        if (request.nonce !== nonce) {
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
      server.listen(0, "127.0.0.1", resolve);
    });
    const address = server.address();
    if (!address || typeof address === "string") {
      server.close();
      throw new Error("Kassiber tool bridge did not bind a loopback port.");
    }
    const bridge = new NativeToolBridge(server, address.port, nonce, tools);
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
  return [
    process.execPath,
    script,
    "mcp",
    String(bridge.port),
    bridge.nonce,
    manifest,
  ];
}

async function forwardToolCall(
  port: number,
  nonce: string,
  name: string,
  args: Record<string, unknown>,
): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    let body = "";
    socket.setEncoding("utf8");
    socket.once("connect", () => {
      socket.write(
        `${JSON.stringify({ nonce, call_id: randomUUID(), name, arguments: args })}\n`,
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
  const port = Number(args[0]);
  const nonce = args[1];
  const manifest = args[2];
  if (!Number.isInteger(port) || port < 1 || !nonce || !manifest) {
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
              port,
              nonce,
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
