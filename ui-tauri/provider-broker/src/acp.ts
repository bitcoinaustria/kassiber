import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Readable, Writable } from "node:stream";
import * as acp from "@agentclientprotocol/sdk";
import type {
  InitializeRequest,
  InitializeResponse,
  McpServer,
  NewSessionResponse,
  RequestPermissionRequest,
  RequestPermissionResponse,
  SessionConfigOption,
  SessionNotification,
  StopReason,
  ToolCallUpdate,
} from "@agentclientprotocol/sdk";
import { mcpCommand, type NativeToolBridge } from "./native-tools.js";
import type {
  BrokerModel,
  BrokerToolDefinition,
  ChatRequest,
  ProviderId,
  ProviderStatus,
} from "./protocol.js";
import {
  providerEnvironment,
  resolveExecutable,
  runProvider,
} from "./executables.js";
import { promptFromMessages, systemInstructions } from "./prompt.js";
import {
  providerStatus,
  safeErrorMessage,
  safeSessionCursor,
  sensitiveContext,
  writeEvent,
} from "./protocol.js";

/**
 * Agent Client Protocol (https://agentclientprotocol.com) adapter.
 *
 * ACP gives one wire format for many coding agents, but no portable way to
 * switch off an agent's own shell, file and web tools — the protocol only lets
 * a client answer permission prompts, and agents run some tools without
 * asking. So an agent is listed here only when its CLI can be launched with
 * every built-in tool removed and with no user MCP servers, hooks, skills or
 * extensions. ACP then carries the conversation and the Kassiber MCP server.
 *
 * On top of each agent's own lockdown, the session itself fails closed:
 *   - Kassiber advertises no client file system and no terminal, so `fs/*` and
 *     `terminal/*` requests are refused as unknown methods.
 *   - Any tool call that is not one of the advertised Kassiber tools ends the
 *     turn and stops the agent, and its permission request is rejected.
 *   - Sensitive selected-data requests are refused: neither agent can promise
 *     not to keep the session on disk.
 */

/** The server name agents see; also the prefix they put on tool titles. */
export const ACP_MCP_SERVER_NAME = "kassiber";

/** ACP major version this client speaks. */
const ACP_PROTOCOL_VERSION = 1;

const HANDSHAKE_TIMEOUT_MS = 45_000;

export type AcpAgentId = Extract<ProviderId, "gemini" | "copilot">;

type LaunchContext = {
  cwd: string;
  request?: ChatRequest;
  /** Command line of the per-turn Kassiber MCP server, when tools are on. */
  mcp?: string[];
};

type AgentLaunch = {
  args: string[];
  env: NodeJS.ProcessEnv;
  /** Pass the Kassiber MCP server in `session/new` instead of the CLI. */
  sessionMcp: boolean;
};

export type AcpAgentSpec = {
  id: AcpAgentId;
  displayName: string;
  executable: string;
  loginHint: string;
  /** Reasoning levels the CLI accepts, if it has a flag for them. */
  efforts?: string[];
  /** CLI notices the agent sends as ordinary message text. */
  isNotice?: (text: string) => boolean;
  launch: (context: LaunchContext) => Promise<AgentLaunch>;
};

const COPILOT_EFFORTS = ["low", "medium", "high", "xhigh", "max"];

/**
 * GitHub Copilot CLI.
 *
 * `--available-tools` is an allowlist of what the model may see at all, so
 * shell, file, web, task and skill tools are gone rather than merely denied.
 * Copilot ignores MCP servers passed in `session/new` (verified against
 * 1.0.93: the server is never started), so Kassiber's server goes through
 * `--additional-mcp-config` and its tools are named `kassiber-<tool>`.
 */
export const COPILOT_AGENT: AcpAgentSpec = {
  id: "copilot",
  displayName: "GitHub Copilot",
  executable: "copilot",
  loginHint: "Run `copilot login` outside Kassiber.",
  efforts: COPILOT_EFFORTS,
  // Sent as `agent_message_chunk` with no marker when the tool allowlist is
  // applied (1.0.93); it describes Kassiber's lockdown, not the answer.
  isNotice: (text) =>
    /^Info: (?:Disabled tools:|Unknown tool name in the tool allowlist:)/.test(text),
  launch: async ({ request, mcp }) => {
    const tools = request?.tools ?? [];
    const args = [
      "--acp",
      "--no-auto-update",
      "--no-custom-instructions",
      "--disable-builtin-mcps",
    ];
    if (mcp) {
      const [command, ...commandArgs] = mcp;
      args.push(
        "--additional-mcp-config",
        JSON.stringify({
          mcpServers: {
            [ACP_MCP_SERVER_NAME]: {
              type: "local",
              command,
              args: commandArgs,
              tools: ["*"],
            },
          },
        }),
      );
    }
    if (request && request.model !== "default") args.push("--model", request.model);
    const effort = request?.options?.reasoning_effort;
    if (effort && COPILOT_EFFORTS.includes(effort)) {
      args.push("--reasoning-effort", effort);
    }
    // Variadic, so it goes last. A name that matches no tool leaves the model
    // with none at all; Copilot only logs that the name is unknown.
    args.push(
      "--available-tools",
      ...(mcp && tools.length
        ? tools.map((tool) => `${ACP_MCP_SERVER_NAME}-${tool.name}`)
        : [`${ACP_MCP_SERVER_NAME}-none`]),
    );
    return {
      args,
      env: { ...providerEnvironment("copilot"), COPILOT_AUTO_UPDATE: "false" },
      sessionMcp: false,
    };
  },
};

/**
 * Settings Gemini CLI reads at the highest precedence, ahead of the user's
 * and the workspace's. `tools.core` is an allowlist of built-in tools, so an
 * empty list registers none of them (`maybeRegister` in Gemini's tool
 * registry); `tools.exclude` names the dangerous ones again in case a future
 * release stops treating an empty list that way.
 */
export function geminiSystemSettings(mcpServer: string): Record<string, unknown> {
  return {
    tools: {
      core: [],
      exclude: [
        "run_shell_command",
        "read_file",
        "read_many_files",
        "write_file",
        "replace",
        "list_directory",
        "glob",
        "grep_search",
        "search_file_content",
        "web_fetch",
        "google_web_search",
        "save_memory",
        "write_todos",
      ],
    },
    // Only Kassiber's per-turn server may start; user-configured servers stay off.
    mcp: { allowed: [mcpServer] },
    hooksConfig: { enabled: false },
    skills: { enabled: false },
    experimental: { enableAgents: false },
    general: {
      enableAutoUpdate: false,
      enableAutoUpdateNotification: false,
      plan: { enabled: false },
    },
    privacy: { usageStatisticsEnabled: false },
    telemetry: { enabled: false },
  };
}

/**
 * Google Gemini CLI.
 *
 * Built-in tools are removed with a Kassiber-owned system settings file (see
 * `geminiSystemSettings`), extensions with `--extensions none`. Gemini takes
 * MCP servers from `session/new` and titles their calls
 * `<tool> (kassiber MCP Server)`.
 */
export const GEMINI_AGENT: AcpAgentSpec = {
  id: "gemini",
  displayName: "Gemini",
  executable: "gemini",
  loginHint: "Run `gemini` once outside Kassiber to sign in.",
  launch: async ({ cwd, request, mcp }) => {
    // With no Kassiber tools this turn, allow a server name nothing uses, so a
    // user server that happens to be called `kassiber` cannot start either.
    const allowed = mcp ? ACP_MCP_SERVER_NAME : `${ACP_MCP_SERVER_NAME}-none-${randomUUID()}`;
    const settingsPath = join(cwd, "kassiber-gemini-system-settings.json");
    await writeFile(settingsPath, JSON.stringify(geminiSystemSettings(allowed)), {
      mode: 0o600,
    });
    const args = ["--acp", "--extensions", "none"];
    if (request && request.model !== "default") args.push("--model", request.model);
    return {
      args,
      env: {
        ...providerEnvironment("gemini"),
        GEMINI_CLI_SYSTEM_SETTINGS_PATH: settingsPath,
        GEMINI_CLI_NO_RELAUNCH: "true",
      },
      sessionMcp: true,
    };
  },
};

export const ACP_AGENTS = {
  copilot: COPILOT_AGENT,
  gemini: GEMINI_AGENT,
} satisfies Record<AcpAgentId, AcpAgentSpec>;

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Recover which advertised Kassiber tool an ACP tool call is, or undefined
 * when it is anything else.
 *
 * ACP has no typed MCP tool-call field, so agents only say it in the title.
 * The forms below cover the agents in `ACP_AGENTS` plus the common prefixes
 * other agents use (`mcp__kassiber__x`, `kassiber_x`, `kassiber.x`), and every
 * match must name a tool advertised this turn, so a title that merely
 * mentions Kassiber is still treated as foreign.
 */
export function kassiberToolFromTitle(
  title: string | null | undefined,
  advertised: ReadonlySet<string>,
): string | undefined {
  if (typeof title !== "string") return undefined;
  const server = escapeRegExp(ACP_MCP_SERVER_NAME);
  // Some agents append the arguments: `tool: {"a":1}`.
  const trimmed = title.trim();
  const patterns = [
    new RegExp(
      `^(?:mcp__)?${server}(?:___|__|_|-|\\.|/|:)(?<tool>[A-Za-z0-9][A-Za-z0-9_]*)(?::.*)?$`,
      "s",
    ),
    new RegExp(`^(?<tool>[A-Za-z0-9][A-Za-z0-9_]*) \\(${server} MCP Server\\)(?::.*)?$`, "s"),
  ];
  for (const pattern of patterns) {
    const tool = pattern.exec(trimmed)?.groups?.tool;
    if (tool && advertised.has(tool)) return tool;
  }
  return undefined;
}

/** Model rows from a session's `model` config option, if the agent has one. */
export function modelsFromConfigOptions(
  options: ReadonlyArray<SessionConfigOption> | null | undefined,
): BrokerModel[] {
  const select = options?.find(
    (option) => option.category === "model" && option.type === "select",
  );
  if (!select || select.type !== "select") return [];
  const rows: BrokerModel[] = [];
  for (const entry of select.options) {
    const values = "group" in entry ? entry.options : [entry];
    for (const value of values) {
      if (typeof value.value !== "string" || !value.value.trim()) continue;
      rows.push({ id: value.value, display_name: value.name || value.value });
    }
  }
  return rows;
}

function defaultModel(spec: AcpAgentSpec): BrokerModel {
  return {
    id: "default",
    display_name: `${spec.displayName} (CLI default)`,
    supports_reasoning_effort: Boolean(spec.efforts?.length),
    ...(spec.efforts?.length ? { reasoning_efforts: spec.efforts } : {}),
  };
}

function withEfforts(spec: AcpAgentSpec, model: BrokerModel): BrokerModel {
  return spec.efforts?.length
    ? { ...model, supports_reasoning_effort: true, reasoning_efforts: spec.efforts }
    : model;
}

class NativeToolAttempt extends Error {
  constructor(spec: AcpAgentSpec) {
    super(`${spec.displayName} attempted to use a provider-native tool; Kassiber stopped it.`);
  }
}

/** ACP reserves -32000 for "authentication required". */
function isAuthError(error: unknown): boolean {
  if (typeof error === "object" && error !== null && (error as { code?: unknown }).code === -32000) {
    return true;
  }
  const message = error instanceof Error ? error.message : String(error);
  return /auth|log ?in|sign ?in|credential|api key/i.test(message);
}

type AgentProcess = {
  child: ChildProcessWithoutNullStreams;
  stream: acp.Stream;
  exited: Promise<void>;
  stderr: () => string;
  stop: () => void;
};

function startAgent(executable: string, launch: AgentLaunch, cwd: string): AgentProcess {
  const child = spawn(executable, launch.args, {
    cwd,
    env: launch.env,
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stderrPreview = "";
  child.stderr.on("data", (chunk) => {
    if (stderrPreview.length < 4_096) stderrPreview += String(chunk);
  });
  const exited = new Promise<void>((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", () => resolve());
  });
  // Unhandled here on purpose only until a caller races it.
  exited.catch(() => undefined);
  const stream = acp.ndJsonStream(
    Writable.toWeb(child.stdin) as WritableStream<Uint8Array>,
    Readable.toWeb(child.stdout) as ReadableStream<Uint8Array>,
  );
  return {
    child,
    stream,
    exited,
    stderr: () => stderrPreview,
    stop: () => {
      if (child.exitCode === null && child.signalCode === null) child.kill();
    },
  };
}

function initializeParams(): InitializeRequest {
  return {
    protocolVersion: ACP_PROTOCOL_VERSION,
    clientCapabilities: {
      fs: { readTextFile: false, writeTextFile: false },
      terminal: false,
    },
    clientInfo: { name: "kassiber", version: "1" },
  };
}

function checkProtocol(spec: AcpAgentSpec, init: InitializeResponse): void {
  if (init.protocolVersion !== ACP_PROTOCOL_VERSION) {
    throw new Error(
      `${spec.displayName} speaks ACP version ${String(init.protocolVersion)}; Kassiber supports version ${ACP_PROTOCOL_VERSION}.`,
    );
  }
}

function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), ms);
    }),
  ]).finally(() => clearTimeout(timer));
}

export async function acpStatus(spec: AcpAgentSpec, cwd: string): Promise<ProviderStatus> {
  const executable = await resolveExecutable(spec.executable);
  if (!executable) {
    return providerStatus(spec.id, spec.displayName, {
      state: "missing_executable",
      message: `Install the ${spec.displayName} CLI, then sign in outside Kassiber.`,
    });
  }
  let version: string | undefined;
  try {
    const result = await runProvider(spec.id, executable, ["--version"], { limit: 16_384 });
    version = result.stdout.trim().split(/\r?\n/, 1)[0]?.slice(0, 80) || undefined;
  } catch {
    version = undefined;
  }
  const agent = startAgent(executable, await spec.launch({ cwd }), cwd);
  try {
    const models = await withTimeout(
      acp
        .client({ name: "kassiber" })
        .onRequest(acp.methods.client.session.requestPermission, () => ({
          outcome: { outcome: "cancelled" as const },
        }))
        .onNotification(acp.methods.client.session.update, () => undefined)
        .connectWith(agent.stream, async (ctx) => {
          const init = await ctx.request(acp.methods.agent.initialize, initializeParams());
          checkProtocol(spec, init);
          // A session is the only proof of a working login ACP offers, and the
          // only place an agent lists its models.
          const session: NewSessionResponse = await ctx.request(acp.methods.agent.session.new, {
            cwd,
            mcpServers: [],
          });
          if (init.agentCapabilities?.sessionCapabilities?.close) {
            await ctx
              .request(acp.methods.agent.session.close, { sessionId: session.sessionId })
              .catch(() => undefined);
          }
          return modelsFromConfigOptions(session.configOptions);
        }),
      HANDSHAKE_TIMEOUT_MS,
      `${spec.displayName} did not answer the ACP handshake.`,
    );
    const discovered = models
      .filter((model) => model.id !== "default")
      .map((model) => withEfforts(spec, model));
    return providerStatus(spec.id, spec.displayName, {
      executable,
      version,
      state: "ready",
      message: `Ready using the existing ${spec.displayName} login.`,
      models: [defaultModel(spec), ...discovered],
    });
  } catch (error) {
    if (isAuthError(error)) {
      return providerStatus(spec.id, spec.displayName, {
        executable,
        version,
        state: "authentication_required",
        message: spec.loginHint,
      });
    }
    return providerStatus(spec.id, spec.displayName, {
      executable,
      version,
      state: "error",
      message: safeErrorMessage(error),
    });
  } finally {
    agent.stop();
  }
}

function finishReason(stopReason: StopReason): string {
  switch (stopReason) {
    case "end_turn":
      return "stop";
    case "max_tokens":
    case "max_turn_requests":
      return "length";
    case "refusal":
      return "content_filter";
    default:
      throw new Error("The agent cancelled the response.");
  }
}

/**
 * Pick the "allow once" answer for an advertised Kassiber tool. Kassiber's
 * own consent still runs when the call reaches the tool bridge; this only
 * lets the agent hand the call over.
 */
function allowOnce(params: RequestPermissionRequest): RequestPermissionResponse {
  const option = params.options.find((candidate) => candidate.kind === "allow_once");
  return option
    ? { outcome: { outcome: "selected", optionId: option.optionId } }
    : { outcome: { outcome: "cancelled" } };
}

function rejectOnce(params: RequestPermissionRequest): RequestPermissionResponse {
  const option =
    params.options.find((candidate) => candidate.kind === "reject_once") ??
    params.options.find((candidate) => candidate.kind === "reject_always");
  return option
    ? { outcome: { outcome: "selected", optionId: option.optionId } }
    : { outcome: { outcome: "cancelled" } };
}

function mcpServerFor(command: string[]): McpServer {
  const [executable, ...args] = command;
  if (!executable) throw new Error("Kassiber MCP bridge command is unavailable.");
  return { name: ACP_MCP_SERVER_NAME, command: executable, args, env: [] };
}

function promptText(request: ChatRequest, resumed: boolean): string {
  const conversation = promptFromMessages(request.messages, resumed);
  // ACP has no system prompt field. A resumed session already holds them.
  if (resumed) return conversation;
  return `SYSTEM INSTRUCTIONS:\n${systemInstructions(request)}\n\n${conversation}`;
}

export async function acpChat(
  spec: AcpAgentSpec,
  request: ChatRequest,
  cwd: string,
  toolBridge?: NativeToolBridge,
): Promise<void> {
  const executable = await resolveExecutable(spec.executable);
  if (!executable) throw new Error(`${spec.displayName} is not installed.`);
  if (sensitiveContext(request)) {
    throw new Error(
      `${spec.displayName} cannot keep a private request out of its session history; use another provider for selected data.`,
    );
  }
  const tools: BrokerToolDefinition[] = toolBridge ? (request.tools ?? []) : [];
  const advertised = new Set(tools.map((tool) => tool.name));
  const mcp = tools.length && toolBridge ? await mcpCommand(cwd, tools, toolBridge) : undefined;
  const launch = await spec.launch({ cwd, request, ...(mcp ? { mcp } : {}) });
  const mcpServers: McpServer[] = mcp && launch.sessionMcp ? [mcpServerFor(mcp)] : [];

  writeEvent({ type: "status", phase: "connecting", message: `Starting ${spec.displayName}` });
  const agent = startAgent(executable, launch, cwd);

  let sessionId: string | undefined;
  // `session/load` replays history as updates; none of it is new output.
  let replaying = false;
  // Text before the prompt is sent is a startup notice, not the answer.
  let prompted = false;
  const kassiberCalls = new Set<string>();
  let rejectViolation: (error: Error) => void = () => undefined;
  const violation = new Promise<never>((_, reject) => {
    rejectViolation = reject;
  });
  violation.catch(() => undefined);
  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    rejectViolation(new NativeToolAttempt(spec));
    agent.stop();
  };

  const inspectToolCall = (update: Pick<ToolCallUpdate, "toolCallId" | "title">) => {
    if (kassiberCalls.has(update.toolCallId)) return;
    if (kassiberToolFromTitle(update.title, advertised)) {
      kassiberCalls.add(update.toolCallId);
      return;
    }
    stop();
  };

  const onUpdate = (notification: SessionNotification) => {
    if (replaying || notification.sessionId !== sessionId) return;
    const update = notification.update;
    switch (update.sessionUpdate) {
      case "agent_message_chunk":
        if (
          prompted &&
          update.content.type === "text" &&
          update.content.text &&
          !spec.isNotice?.(update.content.text)
        ) {
          writeEvent({ type: "delta", content: update.content.text });
        }
        return;
      case "agent_thought_chunk":
        if (prompted && update.content.type === "text" && update.content.text) {
          writeEvent({ type: "delta", reasoning: update.content.text });
        }
        return;
      case "tool_call":
        inspectToolCall(update);
        return;
      case "tool_call_update":
        // An update may be the first we hear of a call.
        inspectToolCall(update);
        return;
      default:
        return;
    }
  };

  const onPermission = (params: RequestPermissionRequest): RequestPermissionResponse => {
    if (params.sessionId !== sessionId || replaying) return { outcome: { outcome: "cancelled" } };
    const call = params.toolCall;
    if (kassiberCalls.has(call.toolCallId) || kassiberToolFromTitle(call.title, advertised)) {
      kassiberCalls.add(call.toolCallId);
      return allowOnce(params);
    }
    const answer = rejectOnce(params);
    stop();
    return answer;
  };

  try {
    const run = acp
      .client({ name: "kassiber" })
      .onRequest(acp.methods.client.session.requestPermission, (ctx) => onPermission(ctx.params))
      .onNotification(acp.methods.client.session.update, (ctx) => onUpdate(ctx.params))
      .connectWith(agent.stream, async (ctx) => {
        const init = await withTimeout(
          ctx.request(acp.methods.agent.initialize, initializeParams()),
          HANDSHAKE_TIMEOUT_MS,
          `${spec.displayName} did not answer the ACP handshake.`,
        );
        checkProtocol(spec, init);

        const cursor = safeSessionCursor(request.options?.provider_session_id);
        let resumed = false;
        if (cursor && init.agentCapabilities?.loadSession) {
          replaying = true;
          sessionId = cursor;
          try {
            await ctx.request(acp.methods.agent.session.load, { sessionId: cursor, cwd, mcpServers });
            resumed = true;
          } catch {
            // Agents key saved sessions by directory, and each turn runs in a
            // fresh one, so a failed load just starts over with the transcript.
            sessionId = undefined;
          } finally {
            replaying = false;
          }
        }
        if (!resumed) {
          const session = await ctx.request(acp.methods.agent.session.new, { cwd, mcpServers });
          sessionId = session.sessionId;
        }
        const activeSession = sessionId;
        if (!activeSession) throw new Error(`${spec.displayName} did not open a session.`);

        writeEvent({ type: "status", phase: "streaming", message: `${spec.displayName} is responding` });
        prompted = true;
        const prompt = ctx.request(acp.methods.agent.session.prompt, {
          sessionId: activeSession,
          prompt: [{ type: "text", text: promptText(request, resumed) }],
        });
        prompt.catch(() => undefined);
        const response = await Promise.race([prompt, violation]);
        return { stopReason: response.stopReason, sessionId: activeSession };
      });
    run.catch(() => undefined);
    const exitedEarly = agent.exited.then(() => {
      throw new Error(
        `${spec.displayName} ended without a terminal response. ${safeErrorMessage(agent.stderr() || "The agent exited.")}`,
      );
    });
    exitedEarly.catch(() => undefined);
    const result = await Promise.race([run, violation, exitedEarly]);
    writeEvent({
      type: "done",
      finish_reason: finishReason(result.stopReason),
      provider_session_id: result.sessionId,
    });
  } catch (error) {
    if (error instanceof NativeToolAttempt) throw error;
    if (isAuthError(error)) throw new Error(`${spec.displayName} requires authentication.`);
    throw error;
  } finally {
    agent.stop();
  }
}
