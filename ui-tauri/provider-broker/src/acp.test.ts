import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  COPILOT_AGENT,
  GEMINI_AGENT,
  acpChat,
  geminiSystemSettings,
  kassiberToolFromTitle,
  modelsFromConfigOptions,
} from "./acp.js";
import { providerEnvironment } from "./executables.js";
import { NativeToolBridge } from "./native-tools.js";
import type { BrokerEvent, ChatRequest } from "./protocol.js";

const roots: string[] = [];
const previousEnv = { ...process.env };
let events: BrokerEvent[] = [];

beforeEach(() => {
  events = [];
  vi.spyOn(process.stdout, "write").mockImplementation((chunk: string | Uint8Array) => {
    for (const line of String(chunk).split("\n")) {
      if (line.trim()) events.push(JSON.parse(line) as BrokerEvent);
    }
    return true;
  });
});

afterEach(async () => {
  vi.restoreAllMocks();
  process.env = { ...previousEnv };
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

async function tempDir(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}

const STATUS_TOOL = {
  name: "status",
  description: "Return the Kassiber book status.",
  parameters: { type: "object", properties: {}, additionalProperties: false },
  read_only: true,
};

function chatRequest(overrides: Partial<ChatRequest> = {}): ChatRequest {
  return {
    command: "chat",
    request_id: "r1",
    provider: "copilot",
    model: "default",
    messages: [{ role: "user", content: "hi" }],
    tools: [STATUS_TOOL],
    ...overrides,
  };
}

/**
 * Install a fake `copilot` on PATH that speaks just enough ACP. `onPrompt`
 * is JavaScript run inside the fake for `session/prompt`, with `send(msg)`
 * and `reply(result)` in scope; the fake also records its argv.
 */
async function fakeAgent(onPrompt: string): Promise<{ argvFile: string }> {
  const bin = await tempDir("kassiber-acp-bin-");
  const argvFile = join(bin, "argv.json");
  const script = join(bin, "agent.mjs");
  await writeFile(
    script,
    `import { writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
writeFileSync(${JSON.stringify(argvFile)}, JSON.stringify(process.argv.slice(2)));
const send = (message) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\\n");
let nextId = 1000;
for await (const line of createInterface({ input: process.stdin })) {
  const message = JSON.parse(line);
  const reply = (result) => send({ id: message.id, result });
  if (message.method === "initialize") {
    reply({ protocolVersion: 1, agentCapabilities: { loadSession: false }, authMethods: [] });
  } else if (message.method === "session/new") {
    reply({ sessionId: "fake-session" });
  } else if (message.method === "session/prompt") {
    const update = (body) => send({ method: "session/update", params: { sessionId: "fake-session", update: body } });
    const ask = (toolCall) => send({ id: nextId++, method: "session/request_permission", params: { sessionId: "fake-session", toolCall, options: [
      { optionId: "yes", name: "Allow", kind: "allow_once" },
      { optionId: "no", name: "Reject", kind: "reject_once" },
    ] } });
    ${onPrompt}
  }
}
`,
  );
  const executable = join(bin, "copilot");
  await writeFile(executable, `#!/bin/sh\nexec "${process.execPath}" "${script}" "$@"\n`);
  await chmod(executable, 0o755);
  process.env.PATH = `${bin}:${previousEnv.PATH ?? ""}`;
  return { argvFile };
}

async function runChat(request: ChatRequest): Promise<void> {
  const cwd = await tempDir("kassiber-acp-cwd-");
  const bridge = request.tools?.length ? await NativeToolBridge.start(request.tools) : undefined;
  try {
    await acpChat(COPILOT_AGENT, request, cwd, bridge);
  } finally {
    await bridge?.close();
  }
}

describe("ACP tool attribution", () => {
  const advertised = new Set(["status", "ui_transactions_list"]);

  it("recognises the advertised Kassiber tools in each agent's title form", () => {
    expect(kassiberToolFromTitle("kassiber-status", advertised)).toBe("status");
    expect(kassiberToolFromTitle("status (kassiber MCP Server)", advertised)).toBe("status");
    expect(kassiberToolFromTitle("mcp__kassiber__ui_transactions_list", advertised)).toBe(
      "ui_transactions_list",
    );
    expect(kassiberToolFromTitle('kassiber_status: {"a":1}', advertised)).toBe("status");
  });

  it("treats everything else as a native tool", () => {
    expect(kassiberToolFromTitle("Read file", advertised)).toBeUndefined();
    expect(kassiberToolFromTitle("status", advertised)).toBeUndefined();
    expect(kassiberToolFromTitle("bash: cat ~/.ssh/id_rsa", advertised)).toBeUndefined();
    expect(kassiberToolFromTitle("kassiber-ui_wallets_sync", advertised)).toBeUndefined();
    expect(kassiberToolFromTitle("run kassiber-status for me", advertised)).toBeUndefined();
    expect(kassiberToolFromTitle("other-status", advertised)).toBeUndefined();
    expect(kassiberToolFromTitle(undefined, advertised)).toBeUndefined();
  });
});

describe("ACP agent lockdown", () => {
  it("removes every Gemini built-in tool and allows only the Kassiber server", () => {
    const settings = geminiSystemSettings("kassiber") as {
      tools: { core: unknown[]; exclude: string[] };
      mcp: { allowed: string[] };
      hooksConfig: { enabled: boolean };
    };
    expect(settings.tools.core).toEqual([]);
    expect(settings.tools.exclude).toContain("run_shell_command");
    expect(settings.tools.exclude).toContain("read_file");
    expect(settings.tools.exclude).toContain("web_fetch");
    expect(settings.mcp.allowed).toEqual(["kassiber"]);
    expect(settings.hooksConfig.enabled).toBe(false);
  });

  it("points Gemini at Kassiber's settings even if the user set their own", async () => {
    process.env.GEMINI_CLI_SYSTEM_SETTINGS_PATH = "/tmp/user-settings.json";
    process.env.GEMINI_API_KEY = "gemini-secret";
    const cwd = await tempDir("kassiber-acp-gemini-");
    const launch = await GEMINI_AGENT.launch({ cwd, request: chatRequest({ provider: "gemini" }) });
    expect(launch.env.GEMINI_CLI_SYSTEM_SETTINGS_PATH).toBe(
      join(cwd, "kassiber-gemini-system-settings.json"),
    );
    expect(launch.env.GEMINI_API_KEY).toBe("gemini-secret");
    expect(launch.args).toEqual(["--acp", "--extensions", "none"]);
    const written = JSON.parse(
      await readFile(join(cwd, "kassiber-gemini-system-settings.json"), "utf8"),
    ) as { mcp: { allowed: string[] } };
    // No tools this turn: not even a user server named "kassiber" may start.
    expect(written.mcp.allowed[0]).toMatch(/^kassiber-none-/);
  });

  it("limits Copilot to the advertised Kassiber tools, or to none", async () => {
    const withTools = await COPILOT_AGENT.launch({
      cwd: "/tmp",
      request: chatRequest({ model: "gpt-x", options: { reasoning_effort: "high" } }),
      mcp: ["/usr/bin/node", "broker.mjs", "mcp", "/tmp/s.sock", "/tmp/m.json"],
    });
    const at = withTools.args.indexOf("--available-tools");
    expect(withTools.args.slice(at)).toEqual(["--available-tools", "kassiber-status"]);
    expect(withTools.args).toContain("--disable-builtin-mcps");
    expect(withTools.args).toContain("--no-custom-instructions");
    expect(withTools.args).toEqual(expect.arrayContaining(["--model", "gpt-x"]));
    expect(withTools.args).toEqual(expect.arrayContaining(["--reasoning-effort", "high"]));
    const config = JSON.parse(
      withTools.args[withTools.args.indexOf("--additional-mcp-config") + 1] ?? "{}",
    ) as { mcpServers: Record<string, { command: string }> };
    expect(Object.keys(config.mcpServers)).toEqual(["kassiber"]);

    const chatOnly = await COPILOT_AGENT.launch({ cwd: "/tmp", request: chatRequest({ tools: [] }) });
    expect(chatOnly.args.slice(chatOnly.args.indexOf("--available-tools"))).toEqual([
      "--available-tools",
      "kassiber-none",
    ]);
    expect(chatOnly.args).not.toContain("--additional-mcp-config");
  });

  it("keeps Copilot's allow-all switch out of its environment", () => {
    process.env.COPILOT_ALLOW_ALL = "true";
    process.env.COPILOT_GITHUB_TOKEN = "gh-token";
    process.env.GEMINI_CLI_SYSTEM_SETTINGS_PATH = "/tmp/user-settings.json";
    const copilot = providerEnvironment("copilot");
    expect(copilot.COPILOT_ALLOW_ALL).toBeUndefined();
    expect(copilot.COPILOT_GITHUB_TOKEN).toBe("gh-token");
    expect(providerEnvironment("gemini").GEMINI_CLI_SYSTEM_SETTINGS_PATH).toBeUndefined();
  });

  it("reads models from a model config option, including grouped ones", () => {
    expect(
      modelsFromConfigOptions([
        {
          type: "select",
          id: "model",
          name: "Model",
          category: "model",
          currentValue: "a",
          options: [
            { group: "fast", name: "Fast", options: [{ value: "a", name: "Model A" }] },
            { group: "smart", name: "Smart", options: [{ value: "b", name: "Model B" }] },
          ],
        },
      ]),
    ).toEqual([
      { id: "a", display_name: "Model A" },
      { id: "b", display_name: "Model B" },
    ]);
    expect(modelsFromConfigOptions(undefined)).toEqual([]);
  });
});

describe("ACP session enforcement", () => {
  it("streams the answer, drops CLI notices, and lets Kassiber tool calls through", async () => {
    await fakeAgent(`
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Info: Disabled tools: bash, view" } });
      update({ sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "thinking" } });
      update({ sessionUpdate: "tool_call", toolCallId: "t1", title: "kassiber-status", kind: "read", status: "pending" });
      update({ sessionUpdate: "tool_call_update", toolCallId: "t1", status: "completed" });
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Hello" } });
      reply({ stopReason: "end_turn" });
    `);
    await runChat(chatRequest());
    const deltas = events.filter((event) => event.type === "delta");
    expect(deltas).toEqual([
      { type: "delta", reasoning: "thinking" },
      { type: "delta", content: "Hello" },
    ]);
    expect(events.at(-1)).toEqual({
      type: "done",
      finish_reason: "stop",
      provider_session_id: "fake-session",
    });
  });

  it("stops the agent when it starts a native tool", async () => {
    await fakeAgent(`
      update({ sessionUpdate: "tool_call", toolCallId: "t1", title: "Read file", kind: "read", status: "pending", rawInput: { path: "~/.ssh/id_rsa" } });
      // Never answers; Kassiber must not wait for the read to finish.
    `);
    await expect(runChat(chatRequest())).rejects.toThrow(/provider-native tool/);
    expect(events.some((event) => event.type === "done")).toBe(false);
  });

  it("rejects a permission request for a native tool and stops the agent", async () => {
    await fakeAgent(`
      ask({ toolCallId: "t2", title: "bash: curl https://example.test", kind: "execute" });
    `);
    await expect(runChat(chatRequest())).rejects.toThrow(/provider-native tool/);
  });

  it("refuses selected-data requests instead of leaving them in the agent's history", async () => {
    await fakeAgent(`reply({ stopReason: "end_turn" });`);
    await expect(
      runChat(chatRequest({ tools: [], options: { sensitive_context: true } })),
    ).rejects.toThrow(/private request/);
  });

  it("launches the installed CLI with Kassiber's lockdown flags", async () => {
    const { argvFile } = await fakeAgent(`reply({ stopReason: "end_turn" });`);
    await runChat(chatRequest());
    const argv = JSON.parse(await readFile(argvFile, "utf8")) as string[];
    expect(argv[0]).toBe("--acp");
    expect(argv.slice(argv.indexOf("--available-tools"))).toEqual([
      "--available-tools",
      "kassiber-status",
    ]);
  });
});
