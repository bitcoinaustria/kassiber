import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createHash } from "node:crypto";
import { createInterface } from "node:readline";
import type { BrokerModel, ChatRequest, ProviderStatus } from "./protocol.js";
import type { NativeToolBridge } from "./native-tools.js";
import {
  providerStatus,
  safeErrorMessage,
  safeSessionCursor,
  writeEvent,
} from "./protocol.js";
import { providerEnvironment, resolveExecutable } from "./executables.js";
import { promptFromMessages, systemInstructions } from "./prompt.js";

export const CODEX_NON_TOOL_ITEM_TYPES = new Set([
  "userMessage",
  "agentMessage",
  "reasoning",
  "dynamicToolCall",
]);

type JsonRpc = {
  id?: number | string;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
  error?: { message?: string };
};

class CodexConnection {
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  private readonly listeners = new Set<(message: JsonRpc) => void>();
  private nextId = 1;
  private stderrPreview = "";
  /**
   * Rejects when the app-server goes away. Notification-backed promises such as
   * the per-turn completion have no pending JSON-RPC id, so a child that exits
   * mid-turn would otherwise leave them pending forever.
   */
  readonly closed: Promise<never>;
  private closedReject!: (error: Error) => void;

  constructor(executable: string, cwd: string, toolBridge?: NativeToolBridge) {
    this.closed = new Promise<never>((_, reject) => {
      this.closedReject = reject;
    });
    // Nothing awaits `closed` until a turn races it; keep Node quiet until then.
    this.closed.catch(() => undefined);
    this.child = spawn(executable, ["app-server", "--stdio"], {
      cwd,
      env: providerEnvironment("codex"),
      stdio: ["pipe", "pipe", "pipe"],
    });
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      let message: JsonRpc;
      try {
        message = JSON.parse(line) as JsonRpc;
      } catch {
        return;
      }
      if (message.id !== undefined && message.method) {
        if (message.method === "item/tool/call" && toolBridge) {
          const params = message.params;
          const name = typeof params?.tool === "string" ? params.tool : "";
          const callId = typeof params?.callId === "string" ? params.callId : "";
          const args =
            typeof params?.arguments === "object" && params.arguments !== null
              ? (params.arguments as Record<string, unknown>)
              : {};
          toolBridge
            .request(name, args, callId)
            .then((output) =>
              this.send({
                id: message.id,
                result: {
                  contentItems: [{ type: "inputText", text: output }],
                  success: true,
                },
              }),
            )
            .catch(() =>
              this.send({
                id: message.id,
                result: {
                  contentItems: [{ type: "inputText", text: "Kassiber denied the tool call." }],
                  success: false,
                },
              }),
            );
          return;
        }
        this.send({
          id: message.id,
          error: {
            code: -32001,
            message: "Provider-native tools are disabled by Kassiber.",
          },
        });
        return;
      }
      if (typeof message.id === "number") {
        const waiter = this.pending.get(message.id);
        if (!waiter) return;
        this.pending.delete(message.id);
        if (message.error) waiter.reject(new Error(message.error.message || "Codex request failed"));
        else waiter.resolve(message.result);
        return;
      }
      for (const listener of this.listeners) listener(message);
    });
    this.child.stderr.on("data", (chunk) => {
      if (this.stderrPreview.length < 1_000) {
        this.stderrPreview += String(chunk);
      }
    });
    // `close`, not `exit`: exit can fire while stdout lines are still queued.
    this.child.once("close", (code) => {
      const detail = safeErrorMessage(this.stderrPreview);
      const error = new Error(
        detail && detail !== "Provider request failed."
          ? detail
          : `Codex app-server exited with code ${String(code)}.`,
      );
      for (const waiter of this.pending.values()) waiter.reject(error);
      this.pending.clear();
      this.closedReject(error);
    });
  }

  private send(message: unknown): void {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  request<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    const id = this.nextId++;
    return new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ id, method, params });
    }) as Promise<T>;
  }

  notify(method: string, params?: unknown): void {
    this.send(params === undefined ? { method } : { method, params });
  }

  onNotification(listener: (message: JsonRpc) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  close(): void {
    this.child.kill("SIGTERM");
  }
}

async function initialize(connection: CodexConnection): Promise<void> {
  await connection.request("initialize", {
    clientInfo: { name: "kassiber", title: "Kassiber", version: "0.22.55" },
    capabilities: { experimentalApi: true },
  });
  connection.notify("initialized");
}

async function loadModels(connection: CodexConnection): Promise<BrokerModel[]> {
  const rows: BrokerModel[] = [];
  let cursor: string | undefined;
  do {
    const response = await connection.request<{
      data?: Array<Record<string, unknown>>;
      nextCursor?: unknown;
    }>("model/list", cursor ? { cursor } : {});
    for (const model of Array.isArray(response?.data) ? response.data : []) {
      const efforts = Array.isArray(model.supportedReasoningEfforts)
        ? model.supportedReasoningEfforts
            .map((entry: unknown) =>
              typeof entry === "string"
                ? entry
                : typeof entry === "object" && entry !== null
                  ? String((entry as { reasoningEffort?: unknown }).reasoningEffort ?? "")
                  : "",
            )
            .filter(Boolean)
        : [];
      rows.push({
        id: String(model.id || model.model),
        display_name: typeof model.displayName === "string" ? model.displayName : undefined,
        owned_by: "OpenAI Codex",
        supports_reasoning_effort: efforts.length > 0,
        reasoning_efforts: efforts,
      });
    }
    cursor = typeof response?.nextCursor === "string" ? response.nextCursor : undefined;
  } while (cursor);
  return rows;
}

export async function codexStatus(cwd: string): Promise<ProviderStatus> {
  const executable = await resolveExecutable("codex");
  if (!executable) {
    return providerStatus("codex", "Codex", {
      state: "missing_executable",
      message: "Install Codex, then run `codex login` outside Kassiber.",
    });
  }
  const connection = new CodexConnection(executable, cwd);
  try {
    await initialize(connection);
    const models = await loadModels(connection);
    return providerStatus("codex", "Codex", {
      executable,
      state: models.length ? "ready" : "authentication_required",
      message: models.length
        ? "Ready using the existing Codex login."
        : "Run `codex login` outside Kassiber.",
      models,
    });
  } catch (error) {
    const message = safeErrorMessage(error);
    const needsLogin = /auth|login|unauthor/i.test(message);
    return providerStatus("codex", "Codex", {
      executable,
      state: needsLogin ? "authentication_required" : "error",
      message: needsLogin ? "Run `codex login` outside Kassiber." : message,
    });
  } finally {
    connection.close();
  }
}

function toolFingerprint(request: ChatRequest): string {
  return createHash("sha256")
    .update(JSON.stringify(request.tools ?? []))
    .digest("hex")
    .slice(0, 16);
}

export async function codexChat(
  request: ChatRequest,
  cwd: string,
  toolBridge?: NativeToolBridge,
): Promise<void> {
  const executable = await resolveExecutable("codex");
  if (!executable) throw new Error("Codex is not installed.");
  const connection = new CodexConnection(executable, cwd, toolBridge);
  try {
    writeEvent({ type: "status", phase: "connecting", message: "Starting Codex app-server" });
    await initialize(connection);
    const instructions = systemInstructions(request);
    const common = {
      cwd,
      model: request.model === "default" ? undefined : request.model,
      approvalPolicy: "untrusted",
      approvalsReviewer: "user",
      sandbox: "read-only",
      ephemeral: false,
      baseInstructions: instructions,
      developerInstructions: instructions,
      config: {
        web_search: "disabled",
        mcp_servers: {},
        multi_agent_mode: "explicitRequestOnly",
      },
    };
    const startParams = {
      ...common,
      dynamicTools: (request.tools ?? []).map((tool) => ({
        type: "function",
        name: tool.name,
        description: tool.description,
        inputSchema: tool.parameters,
      })),
    };
    const fingerprint = toolFingerprint(request);
    const cursorPrefix = `kdt-${fingerprint}:`;
    const rawResumeId = safeSessionCursor(request.options?.provider_session_id);
    const resumeId = rawResumeId?.startsWith(cursorPrefix)
      ? rawResumeId.slice(cursorPrefix.length)
      : request.tools?.length
        ? undefined
        : rawResumeId;
    let opened: { thread: { id: string } };
    let resumed = false;
    try {
      if (resumeId) {
        opened = await connection.request<{ thread: { id: string } }>("thread/resume", {
          threadId: resumeId,
          ...common,
        });
        resumed = true;
      } else {
        opened = await connection.request<{ thread: { id: string } }>(
          "thread/start",
          startParams,
        );
      }
    } catch (error) {
      if (!resumeId || !/not found|missing|unknown/i.test(safeErrorMessage(error))) throw error;
      opened = await connection.request<{ thread: { id: string } }>(
        "thread/start",
        startParams,
      );
    }
    const threadId = String(opened.thread.id);
    const prompt = promptFromMessages(request.messages, resumed);
    // Known residual risk: Codex exposes no no-tools profile, so tools are
    // constrained rather than absent. `sandboxPolicy` below sets readOnly with
    // networkAccess: false, and the listener aborts the turn on the first
    // non-text item — but a local read can begin before that abort lands. With
    // the network off, its content can only surface through assistant text on a
    // turn we are already failing. Revisit if Codex ships a tool-free mode.
    const streamedAgentText = new Map<string, string>();
    const completion = new Promise<{ status: string; error?: unknown }>((resolve, reject) => {
      connection.onNotification((message) => {
        if (message.method === "item/agentMessage/delta") {
          const delta = message.params?.delta;
          const itemId = message.params?.itemId;
          if (typeof delta === "string" && delta) {
            if (typeof itemId === "string") {
              streamedAgentText.set(itemId, (streamedAgentText.get(itemId) ?? "") + delta);
            }
            writeEvent({ type: "delta", content: delta });
          }
        } else if (
          message.method === "item/completed" &&
          typeof message.params?.item === "object" &&
          message.params.item !== null &&
          (message.params.item as { type?: string }).type === "agentMessage"
        ) {
          const item = message.params.item as { id?: string; text?: string };
          const streamed = typeof item.id === "string" ? streamedAgentText.get(item.id) ?? "" : "";
          const remainder =
            typeof item.text === "string" && item.text.startsWith(streamed)
              ? item.text.slice(streamed.length)
              : "";
          if (remainder) writeEvent({ type: "delta", content: remainder });
        } else if (
          message.method === "item/started" &&
          typeof message.params?.item === "object" &&
          message.params.item !== null &&
          !CODEX_NON_TOOL_ITEM_TYPES.has(
            String((message.params.item as { type?: string }).type || ""),
          )
        ) {
          reject(new Error("Codex attempted to use a provider-native tool; Kassiber stopped it."));
        } else if (message.method === "turn/completed") {
          const turn = message.params?.turn as { status?: string; error?: unknown } | undefined;
          resolve({ status: turn?.status || "completed", error: turn?.error });
        } else if (message.method === "error") {
          reject(new Error("Codex reported a provider error."));
        }
      });
    });
    const effort = request.options?.reasoning_effort;
    await connection.request("turn/start", {
      threadId,
      input: [{ type: "text", text: prompt }],
      approvalPolicy: "untrusted",
      approvalsReviewer: "user",
      sandboxPolicy: { type: "readOnly", networkAccess: false },
      ...(request.model === "default" ? {} : { model: request.model }),
      ...(effort && effort !== "auto" ? { effort } : {}),
    });
    const result = await Promise.race([completion, connection.closed]);
    if (result.status !== "completed") throw new Error("Codex did not complete the response.");
    writeEvent({
      type: "done",
      finish_reason: "stop",
      provider_session_id: `${cursorPrefix}${threadId}`,
    });
  } finally {
    connection.close();
  }
}
