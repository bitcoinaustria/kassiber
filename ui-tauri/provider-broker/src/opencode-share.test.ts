// OpenCode sharing uploads a session (prompts, answers, tool results) to a
// public link. Every OpenCode turn runs with it off, whatever the user's
// config or environment asks for, and fails closed if a later config layer
// turns it back on. Incognito's storage isolation needs a file symlink, so on
// Windows it is refused up front instead of failing obscurely.
import { chmod, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { providerEnvironment } from "./executables.js";
import {
  openCodeChat,
  statelessOpenCodeEnvironment,
  verifyOpenCodeSharingOff,
  withoutSharing,
} from "./opencode.js";
import type { ChatRequest } from "./protocol.js";

const posix = process.platform !== "win32";
const roots: string[] = [];
const previousEnv = { ...process.env };

afterEach(async () => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  process.env = { ...previousEnv };
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

function chat(options: ChatRequest["options"] = {}): ChatRequest {
  return {
    command: "chat",
    request_id: "share",
    provider: "opencode",
    model: "provider/model",
    messages: [{ role: "user", content: "synthetic prompt that must not be shared" }],
    options,
  };
}

/**
 * A fake `opencode` on PATH. `serve` records its environment, then exits, or
 * with `listen` reports that it is listening and waits to be stopped; the
 * SDK's requests are then answered by {@link answerLoopback}.
 */
async function fakeOpenCode(listen = false): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "kassiber-opencode-share-"));
  roots.push(root);
  const executable = join(root, "opencode");
  await writeFile(
    executable,
    `#!${process.execPath}
const {appendFileSync}=require('node:fs');
const {join}=require('node:path');
const argv=process.argv.slice(2);
if(argv[0]==='db'){console.log(process.env.OPENCODE_DB||'/home/user/.local/share/opencode/opencode.db');process.exit(0);}
if(argv[0]!=='serve') process.exit(2);
appendFileSync(join(__dirname,'serve.jsonl'),JSON.stringify({
 autoShare:process.env.OPENCODE_AUTO_SHARE??null,
 disableShare:process.env.OPENCODE_DISABLE_SHARE??null,
 config:process.env.OPENCODE_CONFIG_CONTENT??null,
})+'\\n');
if(!${JSON.stringify(listen)}) process.exit(1);
console.log('opencode server listening');
setInterval(()=>{},1000);
`,
  );
  await chmod(executable, 0o755);
  process.env.PATH = root;
  return root;
}

/**
 * The test setup blocks the network; answer the SDK's loopback calls here
 * instead: `GET /config` returns `config`, anything else is recorded.
 */
function answerLoopback(config: Record<string, unknown>): string[] {
  const other: string[] = [];
  vi.stubGlobal("fetch", async (input: string | URL | Request) => {
    const request = input instanceof Request ? input : new Request(input);
    const url = new URL(request.url);
    if (url.hostname !== "127.0.0.1") throw new Error(`unexpected host ${url.hostname}`);
    if (request.method === "GET" && url.pathname === "/config") {
      return new Response(JSON.stringify(config), {
        headers: { "content-type": "application/json" },
      });
    }
    other.push(`${request.method} ${url.pathname}`);
    return new Response("{}", { status: 500, headers: { "content-type": "application/json" } });
  });
  return other;
}

async function run(request: ChatRequest, root: string): Promise<unknown> {
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  const cwd = await mkdtemp(join(root, "turn-"));
  try {
    await openCodeChat(request, cwd);
  } catch (error) {
    return error;
  } finally {
    vi.restoreAllMocks();
  }
  return undefined;
}

async function serves(root: string) {
  return (await readFile(join(root, "serve.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(
      (line) =>
        JSON.parse(line) as { autoShare: string | null; disableShare: string | null; config: string | null },
    );
}

describe("OpenCode sharing", () => {
  it("overrides a user's auto-share in config and environment", () => {
    const env = withoutSharing({
      OPENCODE_AUTO_SHARE: "1",
      OPENCODE_CONFIG_CONTENT: JSON.stringify({ share: "auto", autoshare: true, model: "x/y" }),
    });
    expect(env.OPENCODE_AUTO_SHARE).toBeUndefined();
    expect(env.OPENCODE_DISABLE_SHARE).toBe("1");
    // The user's other settings stay; only sharing is forced off.
    expect(JSON.parse(env.OPENCODE_CONFIG_CONTENT!)).toEqual({
      share: "disabled",
      autoshare: false,
      model: "x/y",
    });
    expect(() => withoutSharing({ OPENCODE_CONFIG_CONTENT: "[1]" })).toThrow(/could not be isolated/);
  });

  it("never passes the auto-share variable to OpenCode", () => {
    process.env.OPENCODE_AUTO_SHARE = "1";
    process.env.OPENCODE_CONFIG = "/tmp/example-opencode.json";
    const env = providerEnvironment("opencode");
    expect(env.OPENCODE_AUTO_SHARE).toBeUndefined();
    expect(env.OPENCODE_CONFIG).toBe("/tmp/example-opencode.json");
  });

  it("refuses any effective setting other than disabled", () => {
    expect(() => verifyOpenCodeSharingOff({ share: "disabled" })).not.toThrow();
    for (const share of ["auto", "manual", undefined]) {
      expect(() => verifyOpenCodeSharingOff({ share })).toThrow(/would share this session/);
    }
  });

  it.skipIf(!posix)("starts every OpenCode turn with sharing off", async () => {
    const root = await fakeOpenCode();
    process.env.OPENCODE_AUTO_SHARE = "1";
    process.env.OPENCODE_CONFIG_CONTENT = JSON.stringify({ share: "auto" });
    for (const options of [{}, { stateless_session: true }]) {
      expect(String(await run(chat(options), root))).toMatch(/server exited/);
    }
    const started = await serves(root);
    expect(started).toHaveLength(2);
    for (const serve of started) {
      expect(serve.autoShare).toBeNull();
      expect(serve.disableShare).toBe("1");
      expect(JSON.parse(serve.config ?? "{}").share).toBe("disabled");
    }
  });

  it.skipIf(!posix)("stops before any session when OpenCode still reports sharing", async () => {
    // A managed or organisation config layer merged after Kassiber's.
    const root = await fakeOpenCode(true);
    const other = answerLoopback({ share: "auto" });
    const error = await run(chat(), root);
    expect(String(error)).toMatch(/would share this session/);
    // No session was created and no prompt was sent.
    expect(other).toEqual([]);
    // The control: with sharing off the turn goes on to open a session.
    const allowed = answerLoopback({ share: "disabled" });
    await run(chat(), root);
    expect(allowed[0]).toBe("POST /session");
  });
});

describe("OpenCode Incognito on Windows", () => {
  it("is refused before anything is created or started", async () => {
    const root = await fakeOpenCode();
    const cwd = await mkdtemp(join(root, "turn-"));
    await expect(statelessOpenCodeEnvironment(join(root, "opencode"), cwd, "win32")).rejects.toThrow(
      "Incognito isn't available for OpenCode on Windows",
    );
    expect(await readdir(cwd)).toEqual([]);
  });

  it("follows the platform the broker runs on", async () => {
    const root = await fakeOpenCode();
    const cwd = await mkdtemp(join(root, "turn-"));
    const platform = Object.getOwnPropertyDescriptor(process, "platform")!;
    Object.defineProperty(process, "platform", { ...platform, value: "win32" });
    try {
      await expect(statelessOpenCodeEnvironment(join(root, "opencode"), cwd)).rejects.toThrow(
        /on Windows/,
      );
    } finally {
      Object.defineProperty(process, "platform", platform);
    }
    expect(await readdir(cwd)).toEqual([]);
  });
});
