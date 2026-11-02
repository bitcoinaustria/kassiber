/** OpenCode 2's HTTP API. No SDK dependency or provider-native tool dispatch. */
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { access, mkdir, readFile, readdir, readlink, stat, symlink } from "node:fs/promises";
import { homedir, userInfo } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { trackChild } from "./cleanup.js";
import { providerEnvironment } from "./executables.js";
import { mcpCommand, type NativeToolBridge } from "./native-tools.js";
import { isLoopbackEndpoint } from "./opencode.js";
import { promptFromMessages, promptImages, systemInstructions } from "./prompt.js";
import { safeSessionCursor, writeEvent, type BrokerModel, type ChatRequest } from "./protocol.js";

type JsonObject = Record<string, unknown>;
const object = (value: unknown): JsonObject => value && typeof value === "object" && !Array.isArray(value)
  ? value as JsonObject : {};
const AGENT = "kassiber";
const LIMIT = 2 * 1024 * 1024;
const PLUGINS = ["-*", "opencode.models-dev", "opencode.provider.*", "opencode.config.provider",
  "opencode.config.agent", "opencode.config.compaction"];

/** Install the logger's discard sink before starting any v2 process. */
export async function openCodeDiscardEnvironment(cwd: string): Promise<NodeJS.ProcessEnv> {
  if (process.platform === "win32") {
    throw new Error("OpenCode 2 requires a log discard sink unavailable on Windows. Choose another provider.");
  }
  const root = join(cwd, "opencode-runtime");
  const logs = join(root, "data", "opencode", "log");
  await mkdir(logs, { recursive: true, mode: 0o700 });
  for (const name of ["opencode.log", "opencode-latest.log"]) {
    const path = join(logs, name);
    try { await symlink("/dev/null", path); } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
    try {
      if (await readlink(path) !== "/dev/null") throw new Error();
    } catch { throw new Error("OpenCode's log discard sink could not be verified; nothing was sent."); }
  }
  return { ...providerEnvironment("opencode"), XDG_DATA_HOME: join(root, "data"),
    XDG_STATE_HOME: join(root, "state"), XDG_CACHE_HOME: join(root, "cache") };
}

/** JSONC's comments and trailing commas, without ever editing quoted strings. */
export function parseOpenCodeConfig(text: string): JsonObject {
  try {
    const comments = /("(?:\\.|[^"\\])*")|\/\/[^\r\n]*|\/\*[\s\S]*?\*\//g;
    const clean = text.replace(comments, (token, quoted: string | undefined) => quoted ?? token.replace(/[^\r\n]/g, " "));
    const value: unknown = JSON.parse(clean.replace(/("(?:\\.|[^"\\])*")|,(\s*[}\]])/g,
      (token, quoted: string | undefined, closing: string) => quoted ?? closing ?? token));
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error();
    return value as JsonObject;
  } catch {
    throw new Error("OpenCode provider configuration could not be read. Check its JSON or JSONC outside Kassiber.");
  }
}

function merge(a: JsonObject, b: JsonObject): JsonObject {
  const result = Object.assign(Object.create(null) as JsonObject, a);
  for (const [key, value] of Object.entries(b)) {
    result[key] = value && typeof value === "object" && !Array.isArray(value)
      ? merge(object(result[key]), object(value)) : value;
  }
  return result;
}

async function exists(path: string): Promise<boolean> {
  try { await access(path); return true; } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw new Error("OpenCode configuration isolation could not be verified.");
  }
}

/**
 * Read only provider settings into RAM. Passing the original config to v2
 * would import user plugins before a later '-*' could disable them.
 * Credentials in provider settings never enter a broker result or disk file.
 */
export async function openCode2Environment(cwd: string, request?: ChatRequest): Promise<NodeJS.ProcessEnv> {
  // The CLI cannot disable managed sources. Refuse them before starting it;
  // their plugins could load before our private inline document is applied.
  const system = process.platform === "darwin" ? "/Library/Application Support/opencode"
    : process.platform === "win32" ? join(process.env.ProgramData ?? "C:\\ProgramData", "opencode") : "/etc/opencode";
  const managed = [system, ...(process.platform === "darwin" ? [
    "/Library/Managed Preferences/ai.opencode.managed.plist",
    `/Library/Managed Preferences/${userInfo().username}/ai.opencode.managed.plist`,
  ] : [])];
  for (const path of managed) if (await exists(path)) {
    throw new Error("System-managed OpenCode configuration cannot be isolated by this build. Choose another provider.");
  }
  const env = await openCodeDiscardEnvironment(cwd);
  const root = join(cwd, "opencode2-config");
  await mkdir(root, { recursive: true, mode: 0o700 });
  const configRoot = process.env.OPENCODE_CONFIG_DIR ?? join(
    process.env.XDG_CONFIG_HOME ?? join(homedir(), ".config"), "opencode");
  const files = [join(configRoot, "opencode.json"), join(configRoot, "opencode.jsonc"),
    ...(process.env.OPENCODE_CONFIG ? [resolve(process.env.OPENCODE_CONFIG)] : [])];
  let settings: JsonObject = {};
  const variables = new Set<string>();
  const providerValue = (value: unknown, directory: string): unknown => {
    if (typeof value === "string") {
      for (const match of value.matchAll(/\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g)) {
        if (match[1]) variables.add(match[1]);
      }
      return value.replace(/\{file:([^}]+)\}/g, (_match, path: string) =>
        `{file:${isAbsolute(path) || path.startsWith("~") ? path : resolve(directory, path)}}`);
    }
    if (Array.isArray(value)) return value.map((item) => providerValue(item, directory));
    if (value && typeof value === "object") return Object.fromEntries(Object.entries(value)
      .map(([key, item]) => [key, providerValue(item, directory)]));
    return value;
  };
  const retain = (config: JsonObject, directory: string) => {
    const kept: JsonObject = {};
    for (const key of ["provider", "providers", "enabled_providers", "disabled_providers", "policies"]) {
      if (config[key] !== undefined) kept[key] = config[key];
    }
    // Keep provider-relative credential-file references relative to their
    // original document. OpenCode itself performs the substitution.
    settings = merge(settings, object(providerValue(kept, directory)));
    for (const source of [config.provider, config.providers]) {
      for (const provider of Object.values(object(source))) {
        const names = object(provider).env;
        for (const name of Array.isArray(names) ? names : []) if (typeof name === "string") variables.add(name);
      }
    }
  };
  for (const file of new Set(files)) {
    if (!await exists(file)) continue;
    if ((await stat(file)).size > LIMIT) throw new Error("OpenCode provider configuration exceeds its size limit.");
    retain(parseOpenCodeConfig(await readFile(file, "utf8")), dirname(file));
  }
  if (process.env.OPENCODE_CONFIG_CONTENT) retain(parseOpenCodeConfig(process.env.OPENCODE_CONFIG_CONTENT), cwd);
  const rules = openCode2Permissions(request);
  const config = {
    ...settings, plugins: PLUGINS, share: "disabled", update: "disable", snapshots: false,
    warming: false, compaction: { auto: false }, instructions: [], skills: [],
    permissions: rules, default_agent: AGENT,
    agents: { [AGENT]: { mode: "primary", system: request ? systemInstructions(request) : "Kassiber model check.",
      permissions: rules }, title: { disabled: true }, summary: { disabled: true }, compaction: { disabled: true } },
    mcp: { servers: {} },
  };
  // Preserve only provider configuration variables, never ambient OpenCode
  // server/plugin/runtime overrides. Other vendor credentials stay subject
  // to providerEnvironment's existing allowlist.
  for (const key of Object.keys(env)) if (key.startsWith("OPENCODE_")) delete env[key];
  for (const key of variables) if (!key.startsWith("OPENCODE_") && process.env[key] !== undefined) env[key] = process.env[key];
  Object.assign(env, {
    OPENCODE_CONFIG_DIR: root, OPENCODE_CONFIG_CONTENT: JSON.stringify(config),
    OPENCODE_CLI_CONFIG_CONTENT: JSON.stringify({ update: "disable" }),
    OPENCODE_CONFIG_PROJECT_DISABLE: "1", OPENCODE_DISABLE_CLAUDE_CODE: "1",
    OPENCODE_DISABLE_MODELS_FETCH: "1", OPENCODE_DISABLE_FREE_MODELS: "1",
    OPENCODE_FILEWATCHER_DISABLE: "1", OPENCODE_DISABLE_FFF: "1", OPENCODE_TEST_HOME: root,
    // An absolute path keeps credentials and ordinary sessions in OpenCode's
    // own DB, while its always-on logger writes into our preinstalled sink.
    OPENCODE_DB: process.env.OPENCODE_DB === ":memory:" ? ":memory:" : resolve(
      process.env.XDG_DATA_HOME ?? join(homedir(), ".local", "share"), "opencode", process.env.OPENCODE_DB ?? "opencode.db"),
  });
  if (env.OPENCODE_DB !== ":memory:") await mkdir(dirname(env.OPENCODE_DB!), { recursive: true, mode: 0o700 });
  return env;
}

export function openCode2Permissions(request?: ChatRequest) {
  return [{ action: "*", resource: "*", effect: "deny" }, ...(request?.tools ?? []).map((tool) => ({
    action: `kassiber_${tool.name}`, resource: "*", effect: "allow",
  }))];
}

type Server = { child: ChildProcessWithoutNullStreams; url: string; headers: Record<string, string> };
async function startServer(executable: string, cwd: string, env: NodeJS.ProcessEnv): Promise<Server> {
  const password = randomBytes(32).toString("base64url");
  const child = spawn(executable, ["serve", "--stdio", "--hostname=127.0.0.1", "--port=0"], {
    cwd, env: { ...env, OPENCODE_SERVER_PASSWORD: password }, stdio: ["pipe", "pipe", "pipe"],
  });
  trackChild(child);
  child.stderr.resume();
  const lines = createInterface({ input: child.stdout });
  try {
    const url = await new Promise<string>((done, reject) => {
      const timeout = setTimeout(() => reject(new Error("OpenCode server startup timed out.")), 30_000);
      const fail = () => { clearTimeout(timeout); reject(new Error("OpenCode server could not start.")); };
      child.once("error", fail);
      child.once("exit", fail);
      lines.on("line", (line) => {
        if (line.length > 4096) return;
        try {
          const value = new URL(String((JSON.parse(line) as { url?: unknown }).url));
          if (value.protocol !== "http:" || value.hostname !== "127.0.0.1" || !value.port || value.username || value.password) return;
          clearTimeout(timeout); done(value.origin);
        } catch { /* Only accept the CLI's structured loopback announcement. */ }
      });
    });
    // Drain bounded process output without retaining diagnostics or secrets.
    child.stderr.resume();
    const logs = join(env.XDG_DATA_HOME!, "opencode", "log");
    for (const entry of await readdir(logs)) {
      if (entry !== "opencode.log" && entry !== "opencode-latest.log") {
        throw new Error("OpenCode's log destination could not be isolated; nothing was sent.");
      }
      if (await readlink(join(logs, entry)) !== "/dev/null") {
        throw new Error("OpenCode's log discard sink could not be verified; nothing was sent.");
      }
    }
    return { child, url, headers: { Authorization: `Basic ${Buffer.from(`opencode:${password}`).toString("base64")}` } };
  } catch (error) { child.kill("SIGKILL"); throw error; }
  finally { lines.close(); child.stdout.resume(); }
}

async function stop(server: Server): Promise<void> {
  if (server.child.exitCode !== null || server.child.signalCode !== null) return;
  await new Promise<void>((done) => {
    const timeout = setTimeout(() => server.child.kill("SIGKILL"), 1_000);
    server.child.once("exit", () => { clearTimeout(timeout); done(); });
    server.child.stdin.end();
    server.child.kill("SIGTERM");
  });
}

async function response(server: Server, path: string, method = "GET", body?: unknown, signal?: AbortSignal) {
  let result: Response;
  try {
    result = await fetch(`${server.url}/api/${path}`, { method, headers: { ...server.headers, "Content-Type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }), redirect: "error",
      signal: signal ?? AbortSignal.timeout(30_000) });
  } catch { throw new Error("OpenCode local API connection failed."); }
  if (!result.ok) {
    await result.body?.cancel();
    throw new Error(`OpenCode local API request failed (HTTP ${result.status}).`);
  }
  return result;
}

async function json(server: Server, path: string, method = "GET", body?: unknown): Promise<JsonObject> {
  const result = await response(server, path, method, body);
  if (result.status === 204) return {};
  if (!result.body) throw new Error("OpenCode returned an invalid API response.");
  const reader = result.body.getReader();
  let text = "";
  const decoder = new TextDecoder();
  try {
    for (;;) {
      const part = await reader.read();
      if (part.done) break;
      text += decoder.decode(part.value, { stream: true });
      if (text.length > LIMIT) throw new Error("OpenCode API response exceeds its size limit.");
    }
    try { return object(JSON.parse(text + decoder.decode())); }
    catch { throw new Error("OpenCode returned an invalid API response."); }
  } finally { await reader.cancel(); }
}

const locationQuery = (cwd: string) => new URLSearchParams({ "location[directory]": cwd });

export function parseOpenCode2Models(payload: JsonObject, providers: JsonObject): BrokerModel[] {
  const routes = new Map<string, unknown>();
  for (const raw of Array.isArray(providers.data) ? providers.data : []) {
    const row = object(raw);
    routes.set(String(row.id), object(row.settings).baseURL);
  }
  return (Array.isArray(payload.data) ? payload.data : []).flatMap((raw) => {
    const row = object(raw);
    if (typeof row.id !== "string" || typeof row.providerID !== "string" || row.enabled === false) return [];
    const endpoint = object(row.settings).baseURL ?? routes.get(row.providerID);
    const local = typeof endpoint === "string" && isLoopbackEndpoint(endpoint);
    const variants = (Array.isArray(row.variants) ? row.variants : []).flatMap((value) =>
      typeof object(value).id === "string" ? [object(value).id as string] : []);
    return [{ id: `${row.providerID}/${row.id}`, display_name: typeof row.name === "string" ? row.name : row.id,
      source_provider: row.providerID, owned_by: `OpenCode · ${row.providerID}`,
      privacy_posture: local ? "local" : "remote",
      privacy_reason: local ? "OpenCode reports a loopback endpoint for this model."
        : "OpenCode does not report a loopback endpoint for this model.",
      supports_reasoning_effort: variants.length > 0, reasoning_efforts: variants,
      supports_tools: object(row.capabilities).tools === true,
    }];
  });
}

export async function openCode2Models(executable: string, cwd: string): Promise<BrokerModel[]> {
  const server = await startServer(executable, cwd, await openCode2Environment(cwd));
  try {
    const query = locationQuery(cwd);
    // The model read awaits plugin/catalog activation. Provider endpoints are
    // read afterwards, and only their projected routing metadata escapes.
    const models = await json(server, `model?${query}`);
    const providers = await json(server, `provider?${query}`);
    return parseOpenCode2Models(models, providers);
  } finally { await stop(server); }
}

async function* events(server: Server, signal: AbortSignal): AsyncGenerator<JsonObject> {
  const result = await response(server, "event", "GET", undefined, signal);
  if (!result.body) throw new Error("OpenCode event stream is unavailable.");
  const reader = result.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) throw new Error("OpenCode event stream ended unexpectedly.");
      buffer += decoder.decode(chunk.value, { stream: true });
      if (buffer.length > LIMIT) throw new Error("OpenCode event stream exceeds its size limit.");
      buffer = buffer.replace(/\r\n/g, "\n");
      for (;;) {
        const end = buffer.indexOf("\n\n");
        if (end < 0) break;
        const frame = buffer.slice(0, end); buffer = buffer.slice(end + 2);
        const data = frame.split("\n").filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart()).join("\n");
        if (data) {
          let event: JsonObject;
          try { event = object(JSON.parse(data)); }
          catch { throw new Error("OpenCode returned an invalid event frame."); }
          yield event;
        }
      }
    }
  } finally { await reader.cancel(); }
}

export async function openCode2Chat(executable: string, request: ChatRequest, cwd: string, bridge?: NativeToolBridge): Promise<void> {
  const slash = request.model.indexOf("/");
  if (slash < 1 || slash === request.model.length - 1) throw new Error("OpenCode models must use provider/model format.");
  const env = await openCode2Environment(cwd, request);
  const privateTurn = request.options?.sensitive_context || request.options?.stateless_session;
  let credentials: JsonObject[] = [];
  if (privateTurn) {
    // Only the chosen integration's active credential crosses this local,
    // authenticated API, in RAM. No prompt is sent to the persistent DB.
    // The persistent source needs provider settings only, never this turn's
    // system context or tool schemas, even as configuration.
    const source = await startServer(executable, cwd, await openCode2Environment(cwd));
    try {
      // provider.list waits for catalog/plugin activation; provider.get does
      // not, and can incorrectly return 404 for a configured provider at boot.
      const providers = await json(source, `provider?${locationQuery(cwd)}`);
      const provider = (Array.isArray(providers.data) ? providers.data : []).map(object)
        .find((item) => item.id === request.model.slice(0, slash)) ?? {};
      const integration = provider.integrationID ?? request.model.slice(0, slash);
      const saved = await json(source, "credential");
      credentials = (Array.isArray(saved.data) ? saved.data : []).map(object)
        .filter((entry) => entry.integrationID === integration && entry.active === true);
      // Refreshing a copied OAuth token can invalidate the persistent token.
      // Until v2 exposes a shared credential store independent of its session
      // database, never risk rotating that credential inside a private DB.
      if (credentials.some((entry) => object(entry.value).type === "oauth")) {
        throw new Error("OpenCode 2 OAuth accounts cannot safely use Incognito or selected-data chats yet. Use a key-backed provider or another CLI provider.");
      }
    } finally { await stop(source); }
    env.OPENCODE_DB = ":memory:";
  }
  writeEvent({ type: "status", phase: "connecting", message: "Starting OpenCode server" });
  const server = await startServer(executable, cwd, env);
  const abort = new AbortController();
  try {
    for (const entry of credentials) {
      await json(server, "credential", "POST", { integrationID: entry.integrationID,
        value: entry.value, activate: true });
    }
    credentials = [];
    if (bridge && request.tools?.length) {
      await json(server, `experimental/mcp/kassiber?${locationQuery(cwd)}`, "PUT", { config: {
        type: "local", command: await mcpCommand(cwd, request.tools, bridge), codemode: false,
      } });
      // add() starts discovery in the background. connect() awaits the MCP
      // handshake and tools/list; otherwise the first inference can have no
      // tools at all. v2.2 then debounces its tool-registry reload by 100ms,
      // without a public flush API. Allow that reconciliation to settle before
      // constructing the session or sending any prompt.
      await json(server, `experimental/mcp/kassiber/connect?${locationQuery(cwd)}`, "POST");
      const connected = await json(server, `mcp?${locationQuery(cwd)}`);
      const mcp = (Array.isArray(connected.data) ? connected.data : []).map(object)
        .find((entry) => entry.name === "kassiber");
      if (object(mcp?.status).status !== "connected") throw new Error("OpenCode could not connect Kassiber's tool bridge.");
      await new Promise((done) => setTimeout(done, 250));
    }
    const model = { providerID: request.model.slice(0, slash), id: request.model.slice(slash + 1),
      ...(request.options?.reasoning_effort && request.options.reasoning_effort !== "auto"
        ? { variant: request.options.reasoning_effort } : {}) };
    // Rebuild from Kassiber's transcript. Native v1 cursors or older v2 agents
    // must not resume a session with a different system/tool configuration.
    const created = await json(server, "session", "POST", { title: "Kassiber assistant", agent: AGENT,
      location: { directory: cwd }, model, permissions: openCode2Permissions(request) });
    const id = safeSessionCursor(object(created.data).id);
    if (!id) throw new Error("OpenCode did not create a chat session.");
    const allowed = new Set((request.tools ?? []).map((tool) => `kassiber_${tool.name}`));
    const stream = events(server, abort.signal);
    // Abort the actual read on timeout, so a server that sends headers but no
    // first frame cannot leave an unobserved stream reader behind.
    const firstFrame = stream.next();
    const readyTimeout = setTimeout(() => abort.abort(), 30_000);
    const first = await firstFrame.finally(() => clearTimeout(readyTimeout));
    if (first.value?.type !== "server.connected") throw new Error("OpenCode did not establish its event stream.");
    const completed = (async () => {
      for await (const event of stream) {
        const data = object(event.data);
        if (data.sessionID !== id) continue;
        if ((event.type === "session.text.delta" || event.type === "session.reasoning.delta") && typeof data.delta === "string") {
          writeEvent(event.type === "session.text.delta" ? { type: "delta", content: data.delta }
            : { type: "delta", reasoning: data.delta });
        } else if (event.type === "session.tool.input.started" && !allowed.has(String(data.name))) {
          throw new Error("OpenCode attempted to use a provider-native tool; Kassiber stopped it.");
        } else if (event.type === "permission.asked") {
          throw new Error("OpenCode requested a permission outside Kassiber's tool bridge.");
        } else if (event.type === "session.execution.failed" || event.type === "session.execution.interrupted") {
          throw new Error("OpenCode did not complete the response.");
        } else if (event.type === "session.execution.succeeded" || event.type === "session.idle") {
          const session = object((await json(server, `session/${encodeURIComponent(id)}`)).data);
          if (session.outcome !== "succeeded") throw new Error("OpenCode did not complete the response.");
          writeEvent({ type: "done", finish_reason: "stop" });
          return;
        }
      }
    })();
    completed.catch(() => undefined);
    await Promise.race([completed, json(server, `session/${encodeURIComponent(id)}/prompt`, "POST", {
      text: promptFromMessages(request.messages, false),
      files: promptImages(request.messages, false).map((image) => ({
        uri: `data:${image.media_type};base64,${image.data}`,
      })),
    })]);
    await completed;
  } finally { abort.abort(); await stop(server); }
}
