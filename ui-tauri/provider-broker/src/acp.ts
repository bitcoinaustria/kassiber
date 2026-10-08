import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir } from "node:fs/promises";
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
import { trackChild } from "./cleanup.js";
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
 * every built-in tool removed and with none of the user's MCP servers, hooks,
 * plugins, skills, memory or settings loaded. That launch is the boundary;
 * ACP then carries the conversation and the Kassiber MCP server.
 *
 * The session also fails closed behind it:
 *   - Kassiber advertises no client file system and no terminal, so `fs/*` and
 *     `terminal/*` requests are refused as unknown methods.
 *   - A tool call or permission request is accepted only when its title is
 *     exactly the agent's form for an advertised Kassiber tool and it is not
 *     an `execute` call; anything else stops the agent. Titles can echo
 *     model-chosen text, so this is a tripwire, not the boundary.
 *   - Sensitive selected-data requests are refused.
 */

/** The server name agents see; also the prefix they put on tool titles. */
export const ACP_MCP_SERVER_NAME = "kassiber";

/** ACP major version this client speaks. */
const ACP_PROTOCOL_VERSION = 1;

/**
 * Startup budget for `initialize` and `session/new`. The Python caller gives a
 * whole status probe 35 seconds before killing the broker, so the agent must
 * be stopped here first, while `finally` can still clean it up.
 */
const HANDSHAKE_TIMEOUT_MS = 25_000;

export type AcpAgentId = Extract<ProviderId, "copilot">;

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
  /** The exact title the agent gives a call to an advertised Kassiber tool. */
  toolTitle: (tool: string) => string;
  /**
   * Whether a session survives the turn. An agent whose state lives in the
   * per-turn directory starts fresh each time from the visible transcript.
   */
  persistentSessions: boolean;
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
 *
 * That flag only adds to `~/.copilot/mcp-config.json`, and the same directory
 * holds plugins, settings (including a persisted allow-all) and saved
 * sessions. `COPILOT_HOME` therefore points at an empty directory inside the
 * turn's working directory, so none of it loads and nothing outlives the
 * turn. Sign-in still works: Copilot keeps its token in the system keyring or
 * takes `COPILOT_GITHUB_TOKEN` / `GH_TOKEN`; a login stored only in the
 * user's own config file is not found, and status reports a sign-in problem.
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
  toolTitle: (tool) => `${ACP_MCP_SERVER_NAME}-${tool}`,
  persistentSessions: false,
  launch: async ({ cwd, request, mcp }) => {
    const home = join(cwd, "copilot-home");
    await mkdir(home, { mode: 0o700 });
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
      env: {
        ...providerEnvironment("copilot"),
        COPILOT_HOME: home,
        COPILOT_AUTO_UPDATE: "false",
      },
      sessionMcp: false,
    };
  },
};

export const ACP_AGENTS = {
  copilot: COPILOT_AGENT,
} satisfies Record<AcpAgentId, AcpAgentSpec>;

/**
 * The advertised Kassiber tool an ACP tool call is, or undefined for anything
 * else. ACP has no typed MCP field, so this compares the title with the
 * agent's exact form for each advertised tool, with no prefix or suffix
 * tolerance, and never accepts an `execute` call: a shell tool titled with
 * its command line could otherwise spell `kassiber-status`.
 */
export function kassiberToolFor(
  spec: Pick<AcpAgentSpec, "toolTitle">,
  call: Pick<ToolCallUpdate, "title" | "kind">,
  advertised: ReadonlySet<string>,
): string | undefined {
  if (call.kind === "execute" || typeof call.title !== "string") return undefined;
  const title = call.title.trim();
  for (const tool of advertised) {
    if (spec.toolTitle(tool) === title) return tool;
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
  trackChild(child);
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
      `${spec.displayName} cannot be held to a stateless, tool-free exchange; use another provider for selected data.`,
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

  const inspectToolCall = (update: Pick<ToolCallUpdate, "toolCallId" | "title" | "kind">) => {
    if (kassiberCalls.has(update.toolCallId)) {
      // A later update may still retitle the call or turn it into execution.
      if (update.kind === "execute") stop();
      return;
    }
    if (kassiberToolFor(spec, update, advertised)) {
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
    if (
      call.kind !== "execute" &&
      (kassiberCalls.has(call.toolCallId) || kassiberToolFor(spec, call, advertised))
    ) {
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

        const cursor = spec.persistentSessions
          ? safeSessionCursor(request.options?.provider_session_id)
          : undefined;
        let resumed = false;
        if (cursor && init.agentCapabilities?.loadSession) {
          replaying = true;
          sessionId = cursor;
          try {
            await withTimeout(
              ctx.request(acp.methods.agent.session.load, { sessionId: cursor, cwd, mcpServers }),
              HANDSHAKE_TIMEOUT_MS,
              `${spec.displayName} did not reopen the session.`,
            );
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
          const session = await withTimeout(
            ctx.request(acp.methods.agent.session.new, { cwd, mcpServers }),
            HANDSHAKE_TIMEOUT_MS,
            `${spec.displayName} did not open a session.`,
          );
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
      ...(spec.persistentSessions ? { provider_session_id: result.sessionId } : {}),
    });
  } catch (error) {
    if (error instanceof NativeToolAttempt) throw error;
    if (isAuthError(error)) throw new Error(`${spec.displayName} requires authentication.`);
    throw error;
  } finally {
    agent.stop();
  }
}
