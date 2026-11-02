import { chmod, mkdir, mkdtemp, readFile, readlink, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NativeToolBridge } from "./native-tools.js";
import { openCodeChat, openCodeStatus } from "./opencode.js";
import { openCode2Environment, parseOpenCodeConfig } from "./opencode2.js";
import type { ChatRequest } from "./protocol.js";

const roots: string[] = [];
const previousEnv = { ...process.env };
afterEach(async () => {
  vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); process.env = { ...previousEnv };
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

const request = (options: ChatRequest["options"] = {}): ChatRequest => ({ command: "chat", request_id: "fixture",
  provider: "opencode", model: "fixture/namespaced/model", instructions: "Kassiber system fixture",
  messages: [{ role: "user", content: "Synthetic user message" }], options });

async function fixture(config = {}) {
  const root = await mkdtemp(join(tmpdir(), "kassiber-opencode2-test-")); roots.push(root);
  process.env.XDG_CONFIG_HOME = join(root, "config"); process.env.XDG_DATA_HOME = join(root, "data");
  delete process.env.OPENCODE_CONFIG; delete process.env.OPENCODE_CONFIG_DIR; delete process.env.OPENCODE_CONFIG_CONTENT;
  delete process.env.OPENCODE_DB;
  const directory = join(root, "config", "opencode"); await mkdir(directory, { recursive: true });
  await writeFile(join(directory, "opencode.json"), JSON.stringify(config));
  const executable = join(root, "opencode");
  await writeFile(executable, `#!${process.execPath}
const fs=require('node:fs'), path=require('node:path');
if(process.argv[2]==='--version'){console.log('opencode v2.2.0');process.exit(0);}
if(process.argv[2]!=='serve')process.exit(2);
const file=path.join(__dirname,'serves.jsonl');
const lines=fs.existsSync(file)?fs.readFileSync(file,'utf8').trim().split('\\n'):[];
fs.appendFileSync(file,JSON.stringify({args:process.argv.slice(2),config:JSON.parse(process.env.OPENCODE_CONFIG_CONTENT),
 data:process.env.XDG_DATA_HOME,db:process.env.OPENCODE_DB,password:process.env.OPENCODE_SERVER_PASSWORD,
 configDir:process.env.OPENCODE_CONFIG_DIR,project:process.env.OPENCODE_CONFIG_PROJECT_DISABLE})+'\\n');
console.log(JSON.stringify({url:'http://127.0.0.1:'+String(45000+lines.length)}));
process.stdin.resume();process.stdin.on('end',()=>process.exit(0));
`);
  await chmod(executable, 0o755); process.env.PATH = root;
  return { root, serves: async () => (await readFile(join(root, "serves.jsonl"), "utf8")).trim()
    .split("\n").map((line) => JSON.parse(line) as { config: Record<string, unknown>; args: string[]; db: string;
      data: string; password: string; configDir: string; project: string }) };
}

function api(ending = "session.execution.succeeded", options: {
  connectedDelay?: number; mcpDelay?: number; oauth?: boolean; neverConnected?: boolean; onEvent?: () => void;
} = {}) {
  const calls: Array<{ port: string; path: string; method: string; body?: Record<string, unknown>; authorization: string | null }> = [];
  let controller: ReadableStreamDefaultController<Uint8Array>;
  let connected = false;
  let mcpReady = false;
  let mcpRequested = false;
  const encoder = new TextEncoder();
  const emit = (type: string, data: unknown) => controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type, data })}\n\n`));
  vi.stubGlobal("fetch", async (input: string | URL | Request, init?: RequestInit) => {
    const value = input instanceof Request ? input : new Request(input, init);
    const url = new URL(value.url); const path = url.pathname;
    expect(url.hostname).toBe("127.0.0.1");
    const text = value.method === "GET" ? "" : await value.text();
    const body = text ? JSON.parse(text) as Record<string, unknown> : undefined;
    calls.push({ port: url.port, path, method: value.method, body, authorization: value.headers.get("Authorization") });
    const json = (data: unknown) => new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
    if (path === "/api/model") return json({ data: [{ providerID: "fixture", id: "namespaced/model", name: "Saved fixture",
      variants: [{ id: "high" }], enabled: true, capabilities: { tools: true } }] });
    if (path === "/api/provider") return json({ data: [{ id: "fixture", integrationID: "selected-account",
      settings: { baseURL: "http://127.0.0.1:1234/v1", apiKey: "secret-key" } }] });
    if (path === "/api/provider/fixture") return json({ data: { id: "fixture", integrationID: "selected-account" } });
    if (path === "/api/credential" && value.method === "GET") return json({ data: [
      { integrationID: "selected-account", active: true, value: options.oauth
        ? { type: "oauth", access: "synthetic-access", refresh: "synthetic-refresh", expires: 1 }
        : { type: "key", key: "selected-secret" } },
      { integrationID: "selected-account", active: false, value: { type: "key", key: "inactive-secret" } },
      { integrationID: "other-account", active: true, value: { type: "key", key: "other-secret" } },
    ] });
    if (path === "/api/credential") return json({ data: { id: "created-credential" } });
    if (path === "/api/experimental/mcp/kassiber") return new Response(null, { status: 204 });
    if (path.endsWith("/kassiber/connect")) {
      mcpRequested = true;
      await new Promise((done) => setTimeout(done, options.mcpDelay ?? 0));
      mcpReady = true;
      return new Response(null, { status: 204 });
    }
    if (path === "/api/mcp") return json({ data: [{ name: "kassiber", status: { status: "connected" } }] });
    if (path === "/api/session") {
      if (mcpRequested) expect(mcpReady).toBe(true);
      return json({ data: { id: "ses_fixture" } });
    }
    if (path === "/api/session/ses_fixture") return json({ data: { id: "ses_fixture", outcome: "succeeded" } });
    if (path === "/api/event") return new Response(new ReadableStream<Uint8Array>({ start(stream) {
      controller = stream;
      const ready = () => { connected = true; emit("server.connected", {}); };
      value.signal.addEventListener("abort", () => stream.error(new Error("Fixture stream aborted")), { once: true });
      if (!options.neverConnected) {
        if (options.connectedDelay) setTimeout(ready, options.connectedDelay);
        else ready();
      }
      options.onEvent?.();
    } }), { headers: { "Content-Type": "text/event-stream" } });
    if (path.endsWith("/prompt")) {
      expect(connected).toBe(true);
      emit("session.text.delta", { sessionID: "ses_other", delta: "foreign-secret" });
      emit("session.reasoning.delta", { sessionID: "ses_fixture", delta: "Reasoning fixture" });
      emit("session.text.delta", { sessionID: "ses_fixture", delta: "Answer fixture" });
      emit(ending, { sessionID: "ses_fixture", name: "shell", error: { message: "private provider diagnostic" } });
      return json({ data: {} });
    }
    throw new Error(`Unexpected fixture API ${value.method} ${path}`);
  });
  return calls;
}

describe.skipIf(process.platform === "win32")("OpenCode 2 adapter", () => {
  it("discovers the v2 catalog and preserves local routing without leaking provider settings", async () => {
    const test = await fixture(); const calls = api();
    const status = await openCodeStatus(test.root);
    expect(status.state).toBe("ready");
    expect(status.models).toEqual([expect.objectContaining({ id: "fixture/namespaced/model", privacy_posture: "local",
      reasoning_efforts: ["high"] })]);
    expect(JSON.stringify(status)).not.toContain("secret-key");
    expect(calls.map((call) => call.path)).toEqual(["/api/model", "/api/provider"]);
    const server = (await test.serves())[0]!;
    expect(server.args).toEqual(["serve", "--stdio", "--hostname=127.0.0.1", "--port=0"]);
    expect(calls.every((call) => call.authorization === `Basic ${Buffer.from(`opencode:${server.password}`).toString("base64")}`)).toBe(true);
  });

  it("streams reasoning and text, filters other sessions, and completes on the v2 execution event", async () => {
    const test = await fixture(); const calls = api();
    const output = vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await openCodeChat(request({ reasoning_effort: "high", provider_session_id: "old-v1-cursor" }), test.root);
    const wire = output.mock.calls.map(([line]) => JSON.parse(String(line)));
    expect(wire).toContainEqual({ type: "delta", content: "Answer fixture" });
    expect(wire).toContainEqual({ type: "delta", reasoning: "Reasoning fixture" });
    expect(wire).toContainEqual({ type: "done", finish_reason: "stop" });
    expect(JSON.stringify(wire)).not.toContain("foreign-secret");
    const create = calls.find((call) => call.path === "/api/session")!;
    expect(create.body?.model).toEqual({ providerID: "fixture", id: "namespaced/model", variant: "high" });
    expect(create.body?.permissions).toEqual([{ action: "*", resource: "*", effect: "deny" }]);
    expect(calls.find((call) => call.path.endsWith("/prompt"))?.body?.text).toContain("Synthetic user message");
    expect(calls.some((call) => call.path.includes("old-v1-cursor"))).toBe(false);
  });

  it.each(["session.execution.failed", "session.execution.interrupted", "session.tool.input.started", "permission.asked"])(
    "fails closed on %s without leaking provider diagnostics", async (ending) => {
      const test = await fixture(); api(ending);
      vi.spyOn(process.stdout, "write").mockImplementation(() => true);
      await expect(openCodeChat(request(), test.root)).rejects.toThrow(/OpenCode/);
    });

  it("exposes only the named Kassiber tools and disables MCP Code Mode", async () => {
    const test = await fixture(); const calls = api("session.execution.succeeded", { mcpDelay: 75, connectedDelay: 75 });
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    const turn = request(); turn.tools = [{ name: "book_read", description: "Fixture read", parameters: { type: "object", properties: {} } }];
    await openCodeChat(turn, test.root, { socketPath: join(test.root, "fixture.sock") } as unknown as NativeToolBridge);
    expect(calls.find((call) => call.path === "/api/session")?.body?.permissions).toEqual([
      { action: "*", resource: "*", effect: "deny" }, { action: "kassiber_book_read", resource: "*", effect: "allow" },
    ]);
    expect(calls.find((call) => call.path.includes("/mcp/"))?.body?.config).toMatchObject({ type: "local", codemode: false });
    const paths = calls.map((call) => call.path);
    expect(paths.indexOf("/api/session")).toBeGreaterThan(paths.indexOf("/api/experimental/mcp/kassiber/connect"));
    expect(paths.indexOf("/api/session")).toBeGreaterThan(paths.indexOf("/api/mcp"));
    expect(paths.indexOf("/api/session/ses_fixture/prompt")).toBeGreaterThan(paths.indexOf("/api/event"));
  });

  it("refuses private OAuth turns before copying or refreshing the persistent credential", async () => {
    const test = await fixture(); const calls = api("session.execution.succeeded", { oauth: true });
    await expect(openCodeChat(request({ stateless_session: true }), test.root)).rejects.toThrow(/OAuth accounts cannot safely/);
    expect(await test.serves()).toHaveLength(1);
    expect(calls.some((call) => call.path.includes("session") || call.method !== "GET")).toBe(false);
  });

  it("times out a server that sends event headers without its ready frame before sending a prompt", async () => {
    const test = await fixture();
    let ready!: () => void;
    const started = new Promise<void>((done) => { ready = done; });
    const calls = api("session.execution.succeeded", { neverConnected: true,
      onEvent: () => { vi.useFakeTimers(); ready(); } });
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    const turn = openCodeChat(request(), test.root);
    const failed = expect(turn).rejects.toThrow(/aborted/);
    await started;
    await vi.advanceTimersByTimeAsync(30_000);
    vi.useRealTimers();
    await failed;
    expect(calls.some((call) => call.path.endsWith("/prompt"))).toBe(false);
  });

  it.each([{ stateless_session: true }, { sensitive_context: true }])("isolates private turns and hands off only the active selected credential (%j)", async (options) => {
    const test = await fixture(); const calls = api();
    const output = vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await openCodeChat(request(options), test.root);
    const servers = await test.serves(); expect(servers).toHaveLength(2);
    expect(servers[0]!.db).toBe(join(test.root, "data/opencode/opencode.db"));
    expect(servers[1]!.db).toBe(":memory:");
    expect(JSON.stringify(servers[0]!.config)).not.toContain("Kassiber system fixture");
    expect(JSON.stringify(servers[1]!.config)).toContain("Kassiber system fixture");
    expect(calls.filter((call) => call.path === "/api/credential" && call.method === "POST")).toEqual([
      expect.objectContaining({ port: "45001", body: { integrationID: "selected-account",
        value: { type: "key", key: "selected-secret" }, activate: true } }),
    ]);
    expect(calls.filter((call) => call.path.includes("/session")).every((call) => call.port === "45001")).toBe(true);
    expect(JSON.stringify(output.mock.calls)).not.toMatch(/selected-secret|inactive-secret|other-secret/);
  });

  it("loads only provider configuration, strips user plugins/MCP/instructions, and discards logs before launch", async () => {
    const test = await fixture({ plugins: ["evil-plugin"], mcp: { servers: { evil: { type: "local", command: ["shell"] } } },
      instructions: ["https://invalid.example"], providers: { fixture: { settings: { baseURL: "http://localhost:1234", apiKey: "fake-key" } } } });
    const env = await openCode2Environment(test.root, request());
    const config = JSON.parse(env.OPENCODE_CONFIG_CONTENT!) as Record<string, unknown>;
    expect(config.providers).toEqual({ fixture: { settings: { baseURL: "http://localhost:1234", apiKey: "fake-key" } } });
    expect(JSON.stringify(config)).not.toMatch(/evil|invalid.example/);
    expect(config.plugins).toContain("-*"); expect(config.mcp).toEqual({ servers: {} });
    expect(config.share).toBe("disabled"); expect(config.warming).toBe(false);
    expect(env.OPENCODE_CONFIG_PROJECT_DISABLE).toBe("1");
    expect(await readlink(join(env.XDG_DATA_HOME!, "opencode/log/opencode.log"))).toBe("/dev/null");
  });

  it("reports HTTP failures using only bounded status codes", async () => {
    const test = await fixture();
    vi.stubGlobal("fetch", async () => new Response("private prompt and credential", { status: 503 }));
    const status = await openCodeStatus(test.root);
    expect(status.state).toBe("error"); expect(status.message).toContain("HTTP 503");
    expect(JSON.stringify(status)).not.toContain("private prompt");
  });

  it("never includes malformed API payloads in a check error", async () => {
    const test = await fixture();
    vi.stubGlobal("fetch", async () => new Response("private account data: invalid JSON"));
    const status = await openCodeStatus(test.root);
    expect(status.state).toBe("error");
    expect(status.message).toBe("OpenCode returned an invalid API response.");
  });

  it("rejects a replaced log sink before launching OpenCode", async () => {
    const test = await fixture();
    const env = await openCode2Environment(test.root);
    const sink = join(env.XDG_DATA_HOME!, "opencode/log/opencode.log");
    await unlink(sink); await writeFile(sink, "disposable fixture");
    await expect(openCode2Environment(test.root)).rejects.toThrow(/discard sink could not be verified/);
    await expect(test.serves()).rejects.toThrow();
  });
});

describe("OpenCode JSONC configuration", () => {
  it("preserves quoted comments/commas and strips actual comments/trailing commas", () => {
    expect(parseOpenCodeConfig('{/*comment*/"url":"http://localhost/", "text":"quoted ,} //", // line\n "list":[1,],}'))
      .toEqual({ url: "http://localhost/", text: "quoted ,} //", list: [1] });
  });
  it.each(["[]", "null", '{"key":"private broken value"']) ("sanitizes invalid config errors", (text) => {
    expect(() => parseOpenCodeConfig(text)).toThrow("OpenCode provider configuration could not be read.");
  });
});
