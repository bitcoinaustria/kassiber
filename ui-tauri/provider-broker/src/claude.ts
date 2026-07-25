import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import type { BrokerModel, ChatRequest, ProviderStatus } from "./protocol.js";
import {
  providerEnvironment,
  resolveExecutable,
  runProvider,
} from "./executables.js";
import { CHAT_ONLY_INSTRUCTIONS, promptFromMessages } from "./prompt.js";
import { providerStatus, safeErrorMessage, writeEvent } from "./protocol.js";

/**
 * The Claude CLI has no model-enumeration command, so this list is maintained
 * by hand. It holds *aliases*, not pinned ids, deliberately: an alias always
 * resolves to the current model behind it, whereas `claude-sonnet-5` would rot
 * on the next release. The concrete id is only knowable at runtime — the CLI
 * reports it in the `init` event of a real request — so no version shows here.
 *
 * `default` is Kassiber's own sentinel, not a Claude alias: it means "send no
 * --model" and let the CLI's own configured default apply.
 *
 * Effort levels are the set `claude --effort` documents (low, medium, high,
 * xhigh, max). The CLI applies them per session, not per model, and publishes
 * no per-model capability data.
 */
const CLAUDE_EFFORTS = ["low", "medium", "high", "xhigh", "max"];

export const CLAUDE_MODELS: BrokerModel[] = [
  {
    id: "default",
    display_name: "Claude (CLI default)",
    owned_by: "Anthropic",
    supports_reasoning_effort: true,
    reasoning_efforts: CLAUDE_EFFORTS,
  },
  {
    id: "fable",
    display_name: "Claude Fable",
    owned_by: "Anthropic",
    supports_reasoning_effort: true,
    reasoning_efforts: CLAUDE_EFFORTS,
  },
  {
    id: "opus",
    display_name: "Claude Opus",
    owned_by: "Anthropic",
    supports_reasoning_effort: true,
    reasoning_efforts: CLAUDE_EFFORTS,
  },
  {
    id: "sonnet",
    display_name: "Claude Sonnet",
    owned_by: "Anthropic",
    supports_reasoning_effort: true,
    reasoning_efforts: CLAUDE_EFFORTS,
  },
  {
    id: "haiku",
    display_name: "Claude Haiku",
    owned_by: "Anthropic",
    supports_reasoning_effort: true,
    reasoning_efforts: CLAUDE_EFFORTS,
  },
];

export const CLAUDE_DISABLED_TOOLS = [
  "Bash",
  "Read",
  "Write",
  "Edit",
  "MultiEdit",
  "Glob",
  "Grep",
  "WebFetch",
  "WebSearch",
  "NotebookEdit",
  "Task",
  "Agent",
] as const;

export async function claudeStatus(): Promise<ProviderStatus> {
  const executable = await resolveExecutable("claude");
  if (!executable) {
    return providerStatus("claude", "Claude", {
      state: "missing_executable",
      message: "Install Claude Code, then run `claude login` outside Kassiber.",
    });
  }
  try {
    const [auth, version] = await Promise.all([
      runProvider("claude", executable, ["auth", "status", "--json"], { limit: 16_384 }),
      runProvider("claude", executable, ["--version"], { limit: 16_384 }),
    ]);
    let loggedIn = auth.code === 0;
    try {
      loggedIn = (JSON.parse(auth.stdout) as { loggedIn?: boolean }).loggedIn === true;
    } catch {
      // Older Claude versions communicate authentication through the exit code.
    }
    return providerStatus("claude", "Claude", {
      executable,
      version: version.stdout.trim().slice(0, 80) || undefined,
      state: loggedIn ? "ready" : "authentication_required",
      message: loggedIn
        ? "Ready using the existing Claude login."
        : "Run `claude login` outside Kassiber.",
      models: CLAUDE_MODELS,
    });
  } catch (error) {
    return providerStatus("claude", "Claude", {
      executable,
      state: "error",
      message: safeErrorMessage(error),
    });
  }
}

/**
 * The subset of `claude --output-format stream-json` we read. The CLI emits the
 * raw Anthropic streaming events under `stream_event`, so these shapes are the
 * API's, not ours.
 */
type ClaudeStreamLine = {
  type?: string;
  subtype?: string;
  session_id?: string;
  event?: {
    type?: string;
    delta?: { type?: string; text?: string; thinking?: string };
    content_block?: { type?: string };
  };
};

/**
 * Build the CLI invocation.
 *
 * Tool denial is layered, because a coding CLI can otherwise read and write the
 * user's disk from a chat box:
 *   --strict-mcp-config with no --mcp-config, loads no MCP servers
 *   --permission-mode   dontAsk, so nothing escalates by prompting
 *   --disallowed-tools  the file/exec/network tools by name
 *   --setting-sources   user only, no project or local settings
 * and the reader below still hard-fails on any `tool_use` block that appears.
 *
 * ponytail: `--bare` would also drop hooks/LSP/plugins, but it silently
 * suppresses every `stream_event` line — the reply then arrives only in the
 * terminal `result`, so the answer lands in one lump instead of streaming.
 * Revisit if the CLI ever streams under `--bare`.
 */
function chatArgs(request: ChatRequest): string[] {
  const args = [
    "--print",
    "--output-format",
    "stream-json",
    // stream-json refuses to emit under --print without --verbose.
    "--verbose",
    "--include-partial-messages",
    "--setting-sources",
    "user",
    "--strict-mcp-config",
    "--permission-mode",
    "dontAsk",
    "--system-prompt",
    CHAT_ONLY_INSTRUCTIONS,
  ];
  if (request.model !== "default") args.push("--model", request.model);
  const sessionId = request.options?.provider_session_id;
  if (sessionId) args.push("--resume", sessionId);
  const effort = request.options?.reasoning_effort;
  if (effort && effort !== "auto") args.push("--effort", effort);
  // Variadic, so it goes last: the CLI consumes tool names until the next flag.
  args.push("--disallowed-tools", ...CLAUDE_DISABLED_TOOLS);
  return args;
}

export async function claudeChat(request: ChatRequest, cwd: string): Promise<void> {
  const executable = await resolveExecutable("claude");
  if (!executable) throw new Error("Claude is not installed.");
  const resumed = Boolean(request.options?.provider_session_id);

  const child = spawn(executable, chatArgs(request), {
    cwd,
    env: providerEnvironment("claude"),
    stdio: ["pipe", "pipe", "pipe"],
  });

  let providerSessionId: string | undefined;
  let sawTerminal = false;
  let stderrPreview = "";
  child.stderr.on("data", (chunk) => {
    if (stderrPreview.length < 4_096) stderrPreview += String(chunk);
  });

  try {
    writeEvent({ type: "status", phase: "connecting", message: "Starting Claude" });
    // The prompt goes over stdin, not argv — a long conversation would blow
    // past the platform argument limit.
    child.stdin.end(promptFromMessages(request.messages, resumed));

    const lines = createInterface({ input: child.stdout });
    const exited = new Promise<number | null>((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => resolve(code));
    });

    for await (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      let message: ClaudeStreamLine;
      try {
        message = JSON.parse(trimmed) as ClaudeStreamLine;
      } catch {
        continue;
      }
      if (typeof message.session_id === "string") providerSessionId = message.session_id;

      if (message.type === "stream_event" && message.event) {
        const event = message.event;
        const delta = event.delta;
        if (event.type === "content_block_delta" && delta?.type === "text_delta" && delta.text) {
          writeEvent({ type: "delta", content: delta.text });
        } else if (
          event.type === "content_block_delta" &&
          delta?.type === "thinking_delta" &&
          delta.thinking
        ) {
          writeEvent({ type: "delta", reasoning: delta.thinking });
        } else if (
          event.type === "content_block_start" &&
          event.content_block?.type === "tool_use"
        ) {
          throw new Error("Claude attempted to use a provider-native tool; Kassiber stopped it.");
        }
      }

      if (message.type === "result") {
        if (message.subtype !== "success") {
          throw new Error("Claude did not complete the response.");
        }
        sawTerminal = true;
        writeEvent({
          type: "done",
          finish_reason: "stop",
          provider_session_id: providerSessionId,
        });
        return;
      }
    }

    const code = await exited;
    if (!sawTerminal) {
      const detail = safeErrorMessage(stderrPreview || `Claude exited with code ${code}`);
      throw new Error(`Claude ended without a terminal response. ${detail}`);
    }
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill();
  }
}
