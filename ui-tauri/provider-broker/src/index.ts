import { stdin } from "node:process";
import { createInterface } from "node:readline";
import { codexChat, codexStatus } from "./codex.js";
import { claudeChat, claudeStatus } from "./claude.js";
import { NativeToolBridge, runMcpServer } from "./native-tools.js";
import { openCodeChat, openCodeStatus } from "./opencode.js";
import type {
  BrokerRequest,
  BrokerToolResult,
  ChatRequest,
  ProviderId,
  ProviderStatus,
} from "./protocol.js";
import { safeErrorMessage, writeEvent } from "./protocol.js";
import { withWorkingDirectory } from "./working-directory.js";

type ProviderAdapter = {
  status: (cwd: string) => Promise<ProviderStatus>;
  chat: (
    request: ChatRequest,
    cwd: string,
    bridge?: NativeToolBridge,
  ) => Promise<void>;
};

const PROVIDERS = {
  codex: { status: codexStatus, chat: codexChat },
  claude: { status: () => claudeStatus(), chat: claudeChat },
  opencode: { status: openCodeStatus, chat: openCodeChat },
} satisfies Record<ProviderId, ProviderAdapter>;

async function status(provider: ProviderId): Promise<ProviderStatus> {
  return withWorkingDirectory(provider, (cwd) => PROVIDERS[provider].status(cwd));
}

function errorCode(message: string) {
  if (/not installed|not found|enoent/i.test(message)) return "missing_executable" as const;
  if (/auth|login|unauthor|credential/i.test(message)) return "authentication_required" as const;
  return "provider_error" as const;
}

async function main(): Promise<void> {
  const lines = createInterface({ input: stdin });
  const input = lines[Symbol.asyncIterator]();
  const first = await input.next();
  if (first.done) throw new Error("Kassiber provider broker received no request.");
  const request = JSON.parse(first.value) as BrokerRequest;
  if (request.command === "status") {
    writeEvent({
      type: "result",
      data: await Promise.all((Object.keys(PROVIDERS) as ProviderId[]).map(status)),
    });
    return;
  }
  if (request.command === "models") {
    const snapshot = await status(request.provider);
    writeEvent({ type: "result", data: snapshot.models });
    return;
  }
  const tools = request.tools ?? [];
  const bridge = tools.length ? await NativeToolBridge.start(tools) : undefined;
  const replies = bridge
    ? (async () => {
        for await (const line of input) {
          let message: { command?: string; results?: BrokerToolResult[] };
          try {
            message = JSON.parse(line) as typeof message;
          } catch {
            continue;
          }
          if (message.command === "tool_results" && Array.isArray(message.results)) {
            bridge.resolve(message.results);
          }
        }
      })()
    : undefined;
  try {
    await withWorkingDirectory(request.provider, async (cwd) => {
      await PROVIDERS[request.provider].chat(request, cwd, bridge);
    });
  } finally {
    await bridge?.close();
    lines.close();
    await replies;
  }
}

const work =
  process.argv[2] === "mcp"
    ? runMcpServer(process.argv.slice(3))
    : main();

work.catch((error) => {
  if (process.argv[2] === "mcp") {
    process.stderr.write(`${safeErrorMessage(error)}\n`);
    process.exitCode = 1;
    return;
  }
  const message = safeErrorMessage(error);
  const code = errorCode(message);
  writeEvent({
    type: "error",
    code,
    message:
      code === "authentication_required"
        ? "The provider requires authentication."
        : message,
    hint:
      code === "authentication_required"
        ? "Run the provider's normal login command outside Kassiber."
        : undefined,
  });
  process.exitCode = 1;
});
