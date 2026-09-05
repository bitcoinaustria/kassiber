import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createServer } from "node:net";
import { createInterface } from "node:readline";
import { access, mkdir, symlink } from "node:fs/promises";
import { join, isAbsolute } from "node:path";
import { homedir } from "node:os";
import { createOpencodeClient } from "@opencode-ai/sdk/v2";
import { mcpCommand, type NativeToolBridge } from "./native-tools.js";
import type { BrokerModel, ChatRequest, ProviderStatus } from "./protocol.js";
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

export const DENY_ALL = [{ permission: "*", pattern: "*", action: "deny" as const }];

export function permissionsFor(request: ChatRequest) {
  return [
    ...DENY_ALL,
    ...(request.tools ?? []).map((tool) => ({
      permission: `kassiber_${tool.name}`,
      pattern: "*",
      action: "allow" as const,
    })),
  ];
}

function splitModel(model: string): { providerID: string; modelID: string } {
  const separator = model.indexOf("/");
  if (separator <= 0 || separator === model.length - 1) {
    throw new Error("OpenCode models must use provider/model format.");
  }
  return { providerID: model.slice(0, separator), modelID: model.slice(separator + 1) };
}

async function availablePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
  });
}

async function startServer(executable: string, cwd: string, env = providerEnvironment("opencode")): Promise<{
  child: ChildProcessWithoutNullStreams;
  url: string;
}> {
  const port = await availablePort();
  const child = spawn(
    executable,
    // --pure: global plugins otherwise execute outside the session's DENY_ALL
    // permission set, with the full provider environment, before any session
    // permission applies.
    ["serve", "--pure", "--hostname=127.0.0.1", `--port=${String(port)}`],
    { cwd, env, stdio: ["pipe", "pipe", "pipe"] },
  );
  const ready = new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error("OpenCode server startup timed out.")),
      30_000,
    );
    const accept = (line: string) => {
      if (!line.toLowerCase().includes("opencode server listening")) return;
      clearTimeout(timeout);
      resolve();
    };
    createInterface({ input: child.stdout }).on("line", accept);
    createInterface({ input: child.stderr }).on("line", accept);
    child.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.once("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`OpenCode server exited with code ${String(code)}.`));
    });
  });
  try {
    await ready;
  } catch (error) {
    // The caller never receives a handle on this path, so nothing else can stop
    // the server it started.
    child.kill("SIGKILL");
    throw error;
  }
  return { child, url: `http://127.0.0.1:${String(port)}` };
}

export function sensitiveOpenCodeConfig(model: string) {
  const { providerID } = splitModel(model);
  return {
    model, small_model: model, enabled_providers: [providerID],
    share: "disabled", snapshot: false, autoupdate: false,
    compaction: { auto: false, prune: false },
    permission: { "*": "deny" },
    default_agent: "kassiber_selected_context",
    agent: {
      title: { disable: true }, summary: { disable: true }, compaction: { disable: true },
      kassiber_selected_context: { mode: "primary", model, permission: { "*": "deny" } },
    },
  };
}

export function verifySensitiveOpenCodeConfig(config: Record<string, unknown>, model: string): void {
  const agents = config.agent as Record<string, Record<string, unknown>> | undefined;
  const compaction = config.compaction as Record<string, unknown> | undefined;
  const selected = agents?.kassiber_selected_context;
  const permission = config.permission as Record<string, unknown> | undefined;
  const selectedPermission = selected?.permission as Record<string, unknown> | undefined;
  if (config.model !== model || config.small_model !== model || config.share !== "disabled" ||
    config.snapshot !== false || compaction?.auto !== false || compaction?.prune !== false ||
    config.default_agent !== "kassiber_selected_context" || selected?.model !== model ||
    permission?.["*"] !== "deny" || selectedPermission?.["*"] !== "deny" ||
    agents?.title?.disable !== true || agents?.summary?.disable !== true || agents?.compaction?.disable !== true ||
    Object.values((selected?.permission ?? {}) as object).some((value) => value !== "deny") ||
    Object.values((config.permission ?? {}) as object).some((value) => value !== "deny")
  ) throw new Error("OpenCode configuration overrides prevent private selected-context execution; no selected data was sent.");
}

async function probeEnvironment(executable: string, args: string[], cwd: string, env: NodeJS.ProcessEnv) {
  return new Promise<string>((resolve, reject) => {
    const child = spawn(executable, args, { cwd, env, stdio: ["ignore", "pipe", "ignore"] });
    let output = "";
    const timeout = setTimeout(() => { child.kill(); reject(new Error("OpenCode privacy verification timed out.")); }, 30_000);
    child.stdout.on("data", (chunk) => {
      output += String(chunk);
      if (output.length > 1_000_000) { child.kill(); reject(new Error("OpenCode privacy verification exceeded its bound.")); }
    });
    child.once("error", () => { clearTimeout(timeout); reject(new Error("OpenCode privacy verification failed.")); });
    child.once("close", (code) => {
      clearTimeout(timeout);
      if (code !== 0) reject(new Error("OpenCode privacy verification failed."));
      else resolve(output.trim());
    });
  });
}

export async function sensitiveOpenCodeEnvironment(executable: string, cwd: string, model: string) {
  const env = providerEnvironment("opencode");
  // The storage implementation is audited against this release. Unknown storage
  // semantics fail before a prompt, rather than guessing from a newer version.
  const version = await probeEnvironment(executable, ["--version"], cwd, env);
  if (version !== "1.18.4") throw new Error("OpenCode sensitive context requires the audited 1.18.4 runtime.");
  const runtime = join(cwd, "private-runtime");
  const data = join(runtime, "data", "opencode");
  await mkdir(join(data, "log"), { recursive: true, mode: 0o700 });
  // 1.18.4 always creates a file logger, including with --print-logs. A sink is
  // installed BEFORE launching it; no selected content is written then deleted.
  const sink = process.platform === "win32" ? "\\\\.\\NUL" : "/dev/null";
  await symlink(sink, join(data, "log", "opencode.log"));
  const originalData = process.env.XDG_DATA_HOME && isAbsolute(process.env.XDG_DATA_HOME)
    ? process.env.XDG_DATA_HOME : join(homedir(), ".local", "share");
  const auth = join(originalData, "opencode", "auth.json");
  try { await access(auth); await symlink(auth, join(data, "auth.json")); } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw new Error("OpenCode authentication isolation failed.");
  }
  let prior: Record<string, unknown> = {};
  try {
    if (env.OPENCODE_CONFIG_CONTENT) prior = JSON.parse(env.OPENCODE_CONFIG_CONTENT) as Record<string, unknown>;
  } catch { throw new Error("OpenCode configuration could not be isolated."); }
  Object.assign(env, {
    XDG_DATA_HOME: join(runtime, "data"), XDG_STATE_HOME: join(runtime, "state"),
    OPENCODE_DB: ":memory:", OPENCODE_DISABLE_AUTOUPDATE: "1", OPENCODE_DISABLE_AUTOCOMPACT: "1",
    OPENCODE_DISABLE_PRUNE: "1", OPENCODE_DISABLE_MODELS_FETCH: "1", OPENCODE_AUTO_SHARE: "0",
    OPENCODE_PURE: "1", OPENCODE_PRINT_LOGS: "0", OPENCODE_LOG_LEVEL: "ERROR",
    OPENCODE_CONFIG_CONTENT: JSON.stringify({ ...prior, ...sensitiveOpenCodeConfig(model) }),
  });
  if (await probeEnvironment(executable, ["db", "path"], cwd, env) !== ":memory:") {
    throw new Error("OpenCode did not enable its in-memory database; no selected data was sent.");
  }
  return env;
}

/**
 * Loopback endpoints are the only ones we can call local: the model never
 * leaves the machine. Anything else — including a LAN address — is remote.
 */
export function isLoopbackEndpoint(baseUrl: string): boolean {
  let host: string;
  try {
    host = new URL(baseUrl).hostname.toLowerCase();
  } catch {
    return false;
  }
  // URL() keeps IPv6 hosts in brackets.
  const bare = host.replace(/^\[|\]$/g, "");
  if (bare === "localhost" || bare.endsWith(".localhost")) return true;
  if (bare === "::1") return true;
  // 127.0.0.0/8, matched as a literal IPv4 address. A prefix test like /^127\./
  // also accepts hostnames such as `127.api.example.com`, which resolve
  // publicly — that would badge a remote model as local.
  const ipv4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(bare);
  if (!ipv4) return false;
  const octets = ipv4.slice(1).map(Number);
  return octets.every((octet) => octet <= 255) && octets[0] === 127;
}

/**
 * Resolved `provider.<id>.options.baseURL` per source provider, straight from
 * OpenCode's own merged config, so a model routed to a local runtime is not
 * mislabelled remote just because OpenCode proxies it.
 *
 * Only the baseURL is read. That options block also holds `apiKey` for some
 * providers, which must never leave this function.
 */
export async function loadProviderEndpoints(
  executable: string,
  cwd: string,
): Promise<Map<string, string>> {
  const endpoints = new Map<string, string>();
  // `opencode debug config` intermittently exits non-zero with empty stdout when
  // other opencode processes are running concurrently. An empty result here is
  // not neutral: it downgrades a genuinely local model's badge to remote. Fail
  // safe but retry once, and let the caller run this before the other probes
  // rather than racing them.
  let result = await runProvider("opencode", executable, ["debug", "config"], { cwd });
  if (result.code !== 0 || !result.stdout.trim()) {
    result = await runProvider("opencode", executable, ["debug", "config"], { cwd });
  }
  if (result.code !== 0) return endpoints;
  let config: unknown;
  try {
    config = JSON.parse(result.stdout);
  } catch {
    return endpoints;
  }
  const providers = (config as { provider?: Record<string, unknown> }).provider;
  if (!providers || typeof providers !== "object") return endpoints;
  for (const [name, entry] of Object.entries(providers)) {
    const options = (entry as { options?: { baseURL?: unknown } }).options;
    const baseUrl = options?.baseURL;
    if (typeof baseUrl === "string" && baseUrl.trim()) {
      endpoints.set(name, baseUrl.trim());
    }
  }
  return endpoints;
}

export function parseOpenCodeModels(
  stdout: string,
  endpoints: Map<string, string> = new Map(),
): BrokerModel[] {
  const ansiEscape = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, "g");
  const lines = stdout
    .split(/\r?\n/)
    .map((line) => line.replace(ansiEscape, "").trimEnd());
  const rows: BrokerModel[] = [];
  let id: string | undefined;
  let jsonLines: string[] = [];
  const flush = () => {
    if (!id) return;
    let metadata: Record<string, unknown> = {};
    try {
      metadata = JSON.parse(jsonLines.join("\n")) as Record<string, unknown>;
    } catch {
      // Plain `opencode models` output has no JSON metadata.
    }
    const variants =
      metadata.variants &&
      typeof metadata.variants === "object" &&
      !Array.isArray(metadata.variants)
        ? Object.keys(metadata.variants)
        : [];
    const sourceProvider = id.split("/", 1)[0] ?? id;
    // The model row's own `api.url` is empty for locally-served models, so the
    // route is judged by the source provider's configured endpoint instead.
    // Unknown endpoint stays remote — the conservative default.
    const endpoint = endpoints.get(sourceProvider);
    const local = endpoint !== undefined && isLoopbackEndpoint(endpoint);
    rows.push({
      id,
      display_name:
        typeof metadata.name === "string" ? metadata.name : id.split("/", 2)[1],
      owned_by: `OpenCode · ${sourceProvider}`,
      source_provider: sourceProvider,
      privacy_posture: local ? "local" : "remote",
      privacy_reason: local
        ? `OpenCode routes ${sourceProvider} to a loopback endpoint on this machine.`
        : endpoint === undefined
          ? "OpenCode does not report an endpoint for this provider."
          : `OpenCode routes ${sourceProvider} off this machine.`,
      supports_reasoning_effort: variants.length > 0,
      reasoning_efforts: variants,
    });
    id = undefined;
    jsonLines = [];
  };
  for (const line of lines) {
    // A provider segment plus an arbitrary remainder: namespaced ids such as
    // `openrouter/anthropic/claude-x` are valid and must not be dropped, which
    // would silently empty the inventory and read as "authentication required".
    if (/^[^\s/]+\/[^\s]+$/.test(line)) {
      flush();
      id = line;
    } else if (id) {
      jsonLines.push(line);
    }
  }
  flush();
  return rows;
}

export async function openCodeStatus(cwd: string): Promise<ProviderStatus> {
  const executable = await resolveExecutable("opencode");
  if (!executable) {
    return providerStatus("opencode", "OpenCode", {
      state: "missing_executable",
      message: "Install OpenCode, then run `opencode auth login` outside Kassiber.",
    });
  }
  try {
    // Sequential: three concurrent opencode invocations are what makes the
    // config read flaky, and a lost read silently mislabels local models.
    const endpoints = await loadProviderEndpoints(executable, cwd);
    const [modelsResult, versionResult] = await Promise.all([
      runProvider("opencode", executable, ["models", "--verbose"], { cwd }),
      runProvider("opencode", executable, ["--version"], { cwd }),
    ]);
    const models =
      modelsResult.code === 0
        ? parseOpenCodeModels(modelsResult.stdout, endpoints)
        : [];
    return providerStatus("opencode", "OpenCode", {
      executable,
      version: versionResult.stdout.trim().slice(0, 80) || undefined,
      state: models.length ? "ready" : "authentication_required",
      message: models.length
        ? "Ready using the existing OpenCode configuration."
        : "Run `opencode auth login` outside Kassiber.",
      models,
    });
  } catch (error) {
    return providerStatus("opencode", "OpenCode", {
      executable,
      state: "error",
      message: safeErrorMessage(error),
    });
  }
}

export async function openCodeChat(
  request: ChatRequest,
  cwd: string,
  toolBridge?: NativeToolBridge,
): Promise<void> {
  const executable = await resolveExecutable("opencode");
  if (!executable) throw new Error("OpenCode is not installed.");
  const sensitive = sensitiveContext(request);
  const env = sensitive ? await sensitiveOpenCodeEnvironment(executable, cwd, request.model) : undefined;
  writeEvent({ type: "status", phase: "connecting", message: "Starting OpenCode server" });
  const server = await startServer(executable, cwd, env);
  try {
    const client = createOpencodeClient({
      baseUrl: server.url,
      directory: cwd,
      throwOnError: true,
    });
    if (sensitive) {
      const effective = await client.config.get();
      verifySensitiveOpenCodeConfig((effective.data ?? {}) as Record<string, unknown>, request.model);
    }
    if (toolBridge && request.tools?.length) {
      await client.mcp.add({
        name: "kassiber",
        config: {
          type: "local",
          command: await mcpCommand(cwd, request.tools, toolBridge),
          enabled: true,
        },
      });
    }
    const permissions = permissionsFor(request);
    const allowedToolNames = new Set(
      (request.tools ?? []).map((tool) => `kassiber_${tool.name}`),
    );
    const resumeId = safeSessionCursor(request.options?.provider_session_id);
    let session: { id: string } | undefined;
    let resumed = false;
    if (resumeId) {
      try {
        const existing = await client.session.get({ sessionID: resumeId });
        if (existing.data) {
          await client.session.update({ sessionID: resumeId, permission: permissions });
          session = existing.data;
          resumed = true;
        }
      } catch {
        // OpenCode may have pruned the prior session; start a fresh one.
      }
    }
    if (!session) {
      const created = await client.session.create({ permission: permissions,
        ...(sensitive ? { title: "Kassiber selected context" } : {}),
      });
      session = created.data;
    }
    if (!session) throw new Error("OpenCode did not create a chat session.");

    const subscription = await client.event.subscribe();
    const roles = new Map<string, string>();
    const emitted = new Map<string, string>();
    const completed = (async () => {
      for await (const event of subscription.stream) {
        const properties = "properties" in event ? event.properties : undefined;
        if (!properties || typeof properties !== "object") continue;
        const eventSessionId =
          "sessionID" in properties && typeof properties.sessionID === "string"
            ? properties.sessionID
            : undefined;
        if (eventSessionId && eventSessionId !== session.id) continue;
        if (event.type === "message.updated") {
          roles.set(event.properties.info.id, event.properties.info.role);
        } else if (event.type === "message.part.updated") {
          const part = event.properties.part;
          if (part.type === "tool") {
            if (!allowedToolNames.has(part.tool)) {
              throw new Error(
                "OpenCode attempted to use a provider-native tool; Kassiber stopped it.",
              );
            }
            continue;
          }
          if (roles.get(part.messageID) !== "assistant") continue;
          if (
            (part.type === "text" || part.type === "reasoning") &&
            typeof part.text === "string"
          ) {
            const previous = emitted.get(part.id) ?? "";
            const delta = part.text.startsWith(previous)
              ? part.text.slice(previous.length)
              : part.text;
            emitted.set(part.id, part.text);
            if (!delta) continue;
            writeEvent(
              part.type === "reasoning"
                ? { type: "delta", reasoning: delta }
                : { type: "delta", content: delta },
            );
          }
        } else if (event.type === "session.idle") {
          writeEvent({
            type: "done",
            finish_reason: "stop",
            ...(sensitive ? {} : { provider_session_id: session.id }),
          });
          return;
        } else if (event.type === "session.error") {
          throw new Error("OpenCode reported a provider error.");
        }
      }
      throw new Error("OpenCode event stream ended unexpectedly.");
    })();
    // The consumer can reject before anything awaits it — a failing splitModel
    // or promptAsync below would leave this rejection unobserved until the
    // server closes, surfacing as an unhandled rejection instead of a
    // sanitized broker error. Observing it now keeps the failure attached.
    let consumerError: unknown;
    completed.catch((error: unknown) => {
      consumerError = error;
    });
    const failFast = <T,>(work: Promise<T>): Promise<T> =>
      work.catch((error: unknown) => {
        throw consumerError ?? error;
      });

    const model = splitModel(request.model);
    await failFast(client.session.promptAsync({
      sessionID: session.id,
      model,
      ...(sensitive ? { agent: "kassiber_selected_context" } : {}),
      system: systemInstructions(request),
      ...(request.options?.reasoning_effort &&
      request.options.reasoning_effort !== "auto"
        ? { variant: request.options.reasoning_effort }
        : {}),
      parts: [
        {
          type: "text",
          text: promptFromMessages(request.messages, resumed),
        },
      ],
    }));
    await completed;
  } catch (error) {
    if (sensitive) throw new Error("OpenCode sensitive-context request failed; no provider diagnostic content was retained.");
    throw error;
  } finally {
    server.child.kill("SIGTERM");
  }
}
