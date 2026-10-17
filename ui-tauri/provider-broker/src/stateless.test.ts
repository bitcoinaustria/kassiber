// Incognito turns (`options.stateless_session`) ask each CLI provider not to
// save its session: Codex an ephemeral thread, Claude
// `--no-session-persistence`, OpenCode an in-memory database. Unlike
// selected-data turns they keep tools; only persistence changes, and no
// session cursor goes in or comes back.
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { chatArgs, claudeChat } from "./claude.js";
import { codexChat } from "./codex.js";
import { providerEnvironment } from "./executables.js";
import { openCodeChat, statelessOpenCodeEnvironment } from "./opencode.js";
import { statelessSession, type ChatRequest } from "./protocol.js";

const TOOL = {
  name: "status",
  description: "Return the Kassiber book status.",
  parameters: { type: "object", properties: {}, additionalProperties: false },
};

function chat(
  provider: ChatRequest["provider"],
  options: ChatRequest["options"] = {},
  model = "default",
): ChatRequest {
  return {
    command: "chat",
    request_id: "incognito",
    provider,
    model,
    messages: [{ role: "user", content: "synthetic incognito prompt" }],
    tools: [TOOL],
    options,
  };
}

const posix = process.platform !== "win32";
const roots: string[] = [];
const previousPath = process.env.PATH;

afterEach(async () => {
  vi.restoreAllMocks();
  process.env.PATH = previousPath;
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

/** A fake provider CLI on PATH; returns its directory. */
async function fakeCli(name: string, body: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), `kassiber-stateless-${name}-`));
  roots.push(root);
  const executable = join(root, name);
  await writeFile(executable, `#!${process.execPath}\n${body}`);
  await chmod(executable, 0o755);
  process.env.PATH = root;
  return root;
}

/** Broker events written to stdout during `run`. */
async function brokerEvents(run: () => Promise<void>) {
  const written: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation((chunk) => {
    for (const line of String(chunk).split("\n")) if (line.trim()) written.push(line);
    return true;
  });
  let error: unknown;
  try {
    await run();
  } catch (caught) {
    error = caught;
  }
  vi.restoreAllMocks();
  const events = written.map((line) => JSON.parse(line) as Record<string, unknown>);
  return { error, events, done: events.find((event) => event.type === "done") };
}

describe("stateless session option", () => {
  it("accepts only booleans and never a session to resume", () => {
    expect(statelessSession(chat("codex", { stateless_session: true }))).toBe(true);
    expect(statelessSession(chat("codex"))).toBe(false);
    expect(() => statelessSession(chat("codex", { stateless_session: "yes" as never }))).toThrow();
    expect(() =>
      statelessSession(chat("codex", { stateless_session: true, provider_session_id: "prior" })),
    ).toThrow(/cannot resume/);
  });
});

describe("Claude", () => {
  it("passes --no-session-persistence only for a stateless turn", async () => {
    const stateless = await chatArgs(chat("claude", { stateless_session: true }), "/tmp/empty");
    expect(stateless).toContain("--no-session-persistence");
    expect(await chatArgs(chat("claude"), "/tmp/empty")).not.toContain("--no-session-persistence");
  });

  it.skipIf(!posix)("returns no session cursor from a stateless turn", async () => {
    const root = await fakeCli(
      "claude",
      `const {appendFileSync}=require('node:fs');
const argv=process.argv.slice(2);
appendFileSync(require('node:path').join(__dirname,'argv.jsonl'),JSON.stringify(argv)+'\\n');
if(argv.includes('--help')){console.log('--no-session-persistence');process.exit(0);}
process.stdin.resume();
process.stdin.on('end',()=>{
 console.log(JSON.stringify({type:'result',subtype:'success',session_id:'claude-session'}));
});
`,
    );
    const stateless = await brokerEvents(() =>
      claudeChat(chat("claude", { stateless_session: true }), root),
    );
    expect(stateless.error).toBeUndefined();
    expect(stateless.done).toEqual({ type: "done", finish_reason: "stop" });
    const ordinary = await brokerEvents(() => claudeChat(chat("claude"), root));
    expect(ordinary.done?.provider_session_id).toBe("claude-session");
    const calls = (await readFile(join(root, "argv.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as string[]);
    const chats = calls.filter((argv) => argv.includes("--print"));
    expect(chats.map((argv) => argv.includes("--no-session-persistence"))).toEqual([true, false]);
  });

  it.skipIf(!posix)("refuses a stateless turn on a CLI that would save it anyway", async () => {
    const root = await fakeCli(
      "claude",
      `const {appendFileSync}=require('node:fs');
appendFileSync(require('node:path').join(__dirname,'argv.jsonl'),JSON.stringify(process.argv.slice(2))+'\\n');
console.log('older help without the flag');
`,
    );
    const { error } = await brokerEvents(() =>
      claudeChat(chat("claude", { stateless_session: true }), root),
    );
    expect(String(error)).toMatch(/cannot skip saving/);
    const calls = (await readFile(join(root, "argv.jsonl"), "utf8")).trim().split("\n");
    // Only the --help probe ran; the prompt was never sent.
    expect(calls).toEqual([JSON.stringify(["--help"])]);
  });
});

describe("Codex", () => {
  async function codexTurn(options: ChatRequest["options"], honoursEphemeral = true) {
    const root = await fakeCli(
      "codex",
      `const {createInterface}=require('node:readline');
const {appendFileSync}=require('node:fs');
const capture=require('node:path').join(__dirname,'methods.jsonl');
createInterface({input:process.stdin}).on('line',line=>{
 const m=JSON.parse(line); appendFileSync(capture,JSON.stringify(m)+'\\n');
 if(!m.id) return;
 const result=m.method==='thread/start'
  ?{thread:{id:'thread-1',ephemeral:${honoursEphemeral ? "m.params.ephemeral===true" : "false"}},model:'gpt-5.4'}
  :m.method==='initialize'?{userAgent:'kassiber/0.160.0 (Linux 6.0; x86_64) xterm'}:{};
 console.log(JSON.stringify({id:m.id,result}));
 if(m.method==='turn/start') console.log(JSON.stringify({method:'turn/completed',params:{turn:{status:'completed'}}}));
});
`,
    );
    const outcome = await brokerEvents(() => codexChat(chat("codex", options), root));
    const wire = (await readFile(join(root, "methods.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as { method?: string; params?: Record<string, unknown> });
    return {
      ...outcome,
      start: wire.find((message) => message.method === "thread/start")?.params,
      methods: wire.map((message) => message.method),
    };
  }

  it.skipIf(!posix)("opens an ephemeral thread that keeps Kassiber's tools", async () => {
    const turn = await codexTurn({ stateless_session: true });
    expect(turn.error).toBeUndefined();
    expect(turn.start?.ephemeral).toBe(true);
    expect((turn.start?.dynamicTools as Array<{ name: string }>).map((tool) => tool.name)).toEqual([
      "status",
    ]);
    expect((turn.start?.config as Record<string, unknown>).history).toEqual({ persistence: "none" });
    // Not the selected-data profile: ordinary read-only sandbox and instructions.
    expect(turn.start?.sandbox).toBe("read-only");
    expect(turn.methods).not.toContain("thread/resume");
    expect(turn.done).toEqual({ type: "done", finish_reason: "stop" });
  });

  it.skipIf(!posix)("keeps an ordinary thread and its cursor otherwise", async () => {
    const turn = await codexTurn({});
    expect(turn.start?.ephemeral).toBe(false);
    expect((turn.start?.config as Record<string, unknown>).history).toBeUndefined();
    expect(String(turn.done?.provider_session_id)).toMatch(/:thread-1$/);
  });

  it.skipIf(!posix)("sends nothing when Codex does not open an unsaved thread", async () => {
    const turn = await codexTurn({ stateless_session: true }, false);
    expect(String(turn.error)).toMatch(/could not open an unsaved thread/);
    expect(turn.methods).not.toContain("turn/start");
    expect(turn.done).toBeUndefined();
  });
});

describe("OpenCode", () => {
  // `db path` answers from OPENCODE_DB, or a disk path when a `disk-db`
  // marker sits next to the fake; `serve` records its environment and exits.
  const OPENCODE = `const {appendFileSync,existsSync}=require('node:fs');
const {join}=require('node:path');
const argv=process.argv.slice(2);
if(argv[0]==='--version'){console.log('1.18.35');process.exit(0);}
if(argv[0]==='db'){
 console.log(existsSync(join(__dirname,'disk-db'))?'/home/user/.local/share/opencode/opencode.db':(process.env.OPENCODE_DB||'/home/user/.local/share/opencode/opencode.db'));
 process.exit(0);
}
appendFileSync(join(__dirname,'env.jsonl'),JSON.stringify({argv,db:process.env.OPENCODE_DB??null,data:process.env.XDG_DATA_HOME??null})+'\\n');
process.exit(1);
`;

  it.skipIf(!posix)("keeps the session in memory and the turn's folder, on any version", async () => {
    const root = await fakeCli("opencode", OPENCODE);
    const cwd = await mkdtemp(join(root, "turn-"));
    const env = await statelessOpenCodeEnvironment(join(root, "opencode"), cwd);
    expect(env.OPENCODE_DB).toBe(":memory:");
    expect(env.XDG_DATA_HOME).toBe(join(cwd, "private-runtime", "data"));
    expect(env.XDG_STATE_HOME).toBe(join(cwd, "private-runtime", "state"));
    // Not the selected-data lockdown: the user's configuration, and so the
    // models and Kassiber's per-session tool rules, are unchanged.
    expect(env.OPENCODE_CONFIG_CONTENT).toBe(providerEnvironment("opencode").OPENCODE_CONFIG_CONTENT);
    expect(env.OPENCODE_PURE).toBeUndefined();
  });

  it.skipIf(!posix)("starts the server stateless only for a stateless turn", async () => {
    const root = await fakeCli("opencode", OPENCODE);
    for (const options of [{ stateless_session: true }, {}]) {
      const cwd = await mkdtemp(join(root, "turn-"));
      const { error } = await brokerEvents(() => openCodeChat(chat("opencode", options), cwd));
      expect(String(error)).toMatch(/server exited/);
    }
    const serves = (await readFile(join(root, "env.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as { argv: string[]; db: string | null; data: string | null });
    const [stateless, ordinary] = serves;
    expect(serves.map((serve) => serve.argv[0])).toEqual(["serve", "serve"]);
    expect(stateless?.db).toBe(":memory:");
    expect(stateless?.data?.startsWith(root)).toBe(true);
    expect(ordinary?.db).toBeNull();
  });

  it.skipIf(!posix)("sends nothing when OpenCode keeps a database on disk", async () => {
    const root = await fakeCli("opencode", OPENCODE);
    await writeFile(join(root, "disk-db"), "");
    const cwd = await mkdtemp(join(root, "turn-"));
    const { error } = await brokerEvents(() =>
      openCodeChat(chat("opencode", { stateless_session: true }), cwd),
    );
    expect(String(error)).toMatch(/in-memory database; nothing was sent/);
    // The server never started.
    await expect(readFile(join(root, "env.jsonl"), "utf8")).rejects.toThrow();
  });
});
