import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

import { COPILOT_AGENT } from "./acp.js";
import {
  CLAUDE_DISABLED_TOOLS,
  CLAUDE_MODELS,
  chatArgs,
  claudeFastModeApplies,
} from "./claude.js";
import {
  CODEX_FAST_SERVICE_TIER,
  CODEX_STANDARD_SERVICE_TIER,
  codexChat,
  fastServiceTier,
} from "./codex.js";
import { requestedFastMode, type ChatRequest } from "./protocol.js";
import {
  NATIVE_RUNTIME_FAST_MODELS,
  NATIVE_RUNTIME_REASONING_EFFORTS,
} from "../../src/lib/aiCapabilities.js";

function chat(provider: ChatRequest["provider"], model: string, options: ChatRequest["options"] = {}): ChatRequest {
  return {
    command: "chat",
    request_id: "fast",
    provider,
    model,
    messages: [{ role: "user", content: "Reply with just: ok" }],
    options,
  };
}

describe("fast mode option", () => {
  it("accepts only booleans", () => {
    expect(requestedFastMode(chat("codex", "gpt-5.4", { fast_mode: true }))).toBe(true);
    expect(requestedFastMode(chat("codex", "gpt-5.4", { fast_mode: false }))).toBe(false);
    expect(requestedFastMode(chat("codex", "gpt-5.4"))).toBe(false);
    expect(() =>
      requestedFastMode(chat("codex", "gpt-5.4", { fast_mode: "true" as never })),
    ).toThrow("Invalid fast mode option.");
  });
});

describe("Codex fast tier", () => {
  it("reads the Fast service tier and its description from model/list", () => {
    expect(
      fastServiceTier({
        serviceTiers: [{ id: "priority", name: "Fast", description: "2x speed, increased usage" }],
      }),
    ).toEqual({ description: "2x speed, increased usage" });
    expect(fastServiceTier({ serviceTiers: [{ id: "priority", name: "Fast" }] })).toEqual({});
    // Older app-servers only send the deprecated speed-tier list.
    expect(fastServiceTier({ additionalSpeedTiers: ["fast"] })).toEqual({});
    expect(fastServiceTier({ serviceTiers: [], additionalSpeedTiers: [] })).toBeNull();
    expect(fastServiceTier({ serviceTiers: [{ id: "flex", name: "Flex" }] })).toBeNull();
  });

  type Fixture = {
    model?: string;
    options?: ChatRequest["options"];
    /** `model/list` entries the fixture app-server reports. */
    catalog?: Array<Record<string, unknown>>;
    /** What `thread/start` reports: the resolved model and the saved tier. */
    threadModel?: string;
    savedTier?: string | null;
  };

  const CATALOG = [
    {
      id: "gpt-5.4",
      model: "gpt-5.4",
      isDefault: true,
      serviceTiers: [{ id: "priority", name: "Fast", description: "2x speed, increased usage" }],
    },
    { id: "gpt-5.4-mini", model: "gpt-5.4-mini", serviceTiers: [] },
  ];

  async function codexTurn(fixture: Fixture) {
    const root = await mkdtemp(join(tmpdir(), "kassiber-fast-codex-"));
    const capture = join(root, "methods.jsonl");
    const executable = join(root, "codex");
    const opened = {
      thread: { id: "fixture" },
      model: fixture.threadModel ?? "gpt-5.4",
      // A user's saved `service_tier = "fast"` shows up here.
      serviceTier: fixture.savedTier ?? null,
    };
    await writeFile(
      executable,
      `#!${process.execPath}
const {createInterface}=require('node:readline');
const {appendFileSync}=require('node:fs');
const opened=${JSON.stringify(opened)};
const catalog=${JSON.stringify(fixture.catalog ?? CATALOG)};
createInterface({input:process.stdin}).on('line',line=>{
 const m=JSON.parse(line); appendFileSync(${JSON.stringify(capture)},JSON.stringify(m)+'\\n');
 if(!m.id) return;
 const result=m.method==='thread/start'||m.method==='thread/resume'?opened
  :m.method==='model/list'?{data:catalog}:{};
 console.log(JSON.stringify({id:m.id,result}));
 if(m.method==='turn/start') console.log(JSON.stringify({method:'turn/completed',params:{turn:{status:'completed'}}}));
});
`,
    );
    await chmod(executable, 0o755);
    const previous = process.env.PATH;
    process.env.PATH = root;
    const written: string[] = [];
    const output = vi.spyOn(process.stdout, "write").mockImplementation((chunk) => {
      written.push(String(chunk));
      return true;
    });
    try {
      await codexChat(chat("codex", fixture.model ?? "gpt-5.4", fixture.options ?? {}), root);
      const wire = (await readFile(capture, "utf8"))
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line) as { method?: string; params?: Record<string, unknown> });
      const events = written.map(
        (line) => JSON.parse(line) as { type: string; phase?: string; provider_session_id?: string },
      );
      return {
        turnStart: wire.find((message) => message.method === "turn/start")?.params,
        methods: wire.map((message) => message.method),
        unavailable: events.filter((event) => event.phase === "fast_mode_unavailable").length,
        sessionId: events.find((event) => event.type === "done")?.provider_session_id,
      };
    } finally {
      output.mockRestore();
      process.env.PATH = previous;
      await rm(root, { recursive: true, force: true });
    }
  }

  const posix = process.platform !== "win32";

  it.runIf(posix)("sends the priority tier when the model advertises Fast", async () => {
    const turn = await codexTurn({ options: { fast_mode: true } });
    expect(turn.turnStart?.serviceTier).toBe(CODEX_FAST_SERVICE_TIER);
    expect(turn.unavailable).toBe(0);
  });

  it.runIf(posix)("resolves Kassiber's default model through the thread before choosing Fast", async () => {
    const supported = await codexTurn({ model: "default", options: { fast_mode: true } });
    expect(supported.turnStart?.serviceTier).toBe(CODEX_FAST_SERVICE_TIER);
    const unsupported = await codexTurn({
      model: "default",
      threadModel: "gpt-5.4-mini",
      options: { fast_mode: true },
    });
    expect(unsupported.turnStart?.serviceTier).toBe(CODEX_STANDARD_SERVICE_TIER);
    expect(unsupported.unavailable).toBe(1);
  });

  it.runIf(posix)("keeps the standard tier, with a hint, for unsupported or unknown models", async () => {
    for (const model of ["gpt-5.4-mini", "gpt-unknown"]) {
      const turn = await codexTurn({ model, options: { fast_mode: true } });
      expect(turn.turnStart?.serviceTier).toBe(CODEX_STANDARD_SERVICE_TIER);
      expect(turn.unavailable).toBe(1);
    }
  });

  it.runIf(posix)("pins the standard tier when fast is off, overriding a saved fast config", async () => {
    for (const options of [{}, { fast_mode: false }]) {
      const turn = await codexTurn({ options, savedTier: "fast" });
      expect(turn.turnStart?.serviceTier).toBe(CODEX_STANDARD_SERVICE_TIER);
      expect(turn.unavailable).toBe(0);
      // Fast off needs no catalog lookup at all.
      expect(turn.methods).not.toContain("model/list");
    }
  });

  it.runIf(posix)("returns a resumed fast thread to the standard tier once fast is off", async () => {
    const first = await codexTurn({ options: { fast_mode: true } });
    expect(first.turnStart?.serviceTier).toBe(CODEX_FAST_SERVICE_TIER);
    // The thread now carries the priority tier from that turn.
    const next = await codexTurn({
      options: { provider_session_id: first.sessionId },
      savedTier: CODEX_FAST_SERVICE_TIER,
    });
    expect(next.methods).toContain("thread/resume");
    expect(next.turnStart?.serviceTier).toBe(CODEX_STANDARD_SERVICE_TIER);
  });
});

describe("Claude fast mode", () => {
  it("advertises fast mode only on the Opus alias", () => {
    expect(
      CLAUDE_MODELS.filter((model) => model.supports_fast_mode).map((model) => model.id),
    ).toEqual(["opus"]);
  });

  it("passes the inline fastMode setting only when requested on Opus", async () => {
    const args = await chatArgs(chat("claude", "opus", { fast_mode: true }), "/tmp/empty-fixture");
    expect(args[args.indexOf("--settings") + 1]).toBe('{"fastMode":true}');
    // The variadic tool denial must stay last.
    expect(args.slice(args.indexOf("--disallowed-tools") + 1)).toEqual([...CLAUDE_DISABLED_TOOLS]);
    // Other settings sources stay off.
    expect(args[args.indexOf("--setting-sources") + 1]).toBe("");

    for (const request of [
      chat("claude", "opus"),
      chat("claude", "opus", { fast_mode: false }),
      // Accepted by the CLI but silently ignored there, so never sent.
      chat("claude", "sonnet", { fast_mode: true }),
      chat("claude", "default", { fast_mode: true }),
    ]) {
      expect(claudeFastModeApplies(request)).toBe(false);
      expect(await chatArgs(request, "/tmp/empty-fixture")).not.toContain("--settings");
    }
  });
});

describe("Claude fast mode state", () => {
  async function events(state: string) {
    const root = await mkdtemp(join(tmpdir(), "kassiber-fast-claude-"));
    const executable = join(root, "claude");
    await writeFile(
      executable,
      `#!${process.execPath}
process.stdin.resume();
process.stdin.on('end',()=>{
 console.log(JSON.stringify({type:'system',subtype:'init',session_id:'s1',fast_mode_state:${JSON.stringify(state)}}));
 console.log(JSON.stringify({type:'stream_event',event:{type:'content_block_delta',delta:{type:'text_delta',text:'ok'}}}));
 console.log(JSON.stringify({type:'result',subtype:'success',session_id:'s1',fast_mode_state:${JSON.stringify(state)}}));
});
`,
    );
    await chmod(executable, 0o755);
    const previous = process.env.PATH;
    process.env.PATH = root;
    const written: string[] = [];
    const output = vi.spyOn(process.stdout, "write").mockImplementation((chunk) => {
      written.push(String(chunk));
      return true;
    });
    try {
      const { claudeChat } = await import("./claude.js");
      await claudeChat(chat("claude", "opus", { fast_mode: true }), root);
      return written.map((line) => JSON.parse(line) as { type: string; phase?: string });
    } finally {
      output.mockRestore();
      process.env.PATH = previous;
      await rm(root, { recursive: true, force: true });
    }
  }

  it.skipIf(process.platform === "win32")("notes a declined fast mode once and still completes the turn", async () => {
    const declined = await events("off");
    expect(declined.filter((event) => event.phase === "fast_mode_unavailable")).toHaveLength(1);
    expect(declined.at(-1)?.type).toBe("done");

    const honoured = await events("on");
    expect(honoured.some((event) => event.phase === "fast_mode_unavailable")).toBe(false);
    expect(honoured.at(-1)?.type).toBe("done");
  });
});

describe("Copilot", () => {
  it("never turns a fast request into a launch flag", async () => {
    const root = await mkdtemp(join(tmpdir(), "kassiber-fast-copilot-"));
    try {
      const launch = await COPILOT_AGENT.launch({
        cwd: root,
        request: chat("copilot", "default", { fast_mode: true }),
      });
      expect(launch.args.join(" ")).not.toMatch(/fast|tier|settings/i);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});

describe("desktop mirrors of fixed broker capabilities", () => {
  // The desktop uses these before Check models; they must not drift from
  // what the broker actually advertises.
  it("matches Claude's fast models and effort levels", () => {
    expect(NATIVE_RUNTIME_FAST_MODELS.claude).toEqual(
      CLAUDE_MODELS.filter((model) => model.supports_fast_mode).map((model) => model.id),
    );
    expect(NATIVE_RUNTIME_REASONING_EFFORTS.claude).toEqual(CLAUDE_MODELS[0]?.reasoning_efforts);
  });

  it("matches Copilot's effort levels", () => {
    expect(NATIVE_RUNTIME_REASONING_EFFORTS.copilot).toEqual(COPILOT_AGENT.efforts);
  });
});
