import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

import { COPILOT_AGENT } from "./acp.js";
import {
  CLAUDE_DISABLED_TOOLS,
  CLAUDE_MODELS,
  chatArgs,
  claudeFastFallbackNotification,
  claudeFastModeApplies,
  claudeModelsFor,
  claudeVersionSupportsFast,
} from "./claude.js";
import {
  CODEX_FAST_SERVICE_TIER,
  CODEX_STANDARD_SERVICE_TIER,
  fastTierLookup,
  codexChat,
  codexStatus,
  fastServiceTier,
  supportsPerTurnServiceTier,
} from "./codex.js";
import { requestedFastMode, type ChatRequest } from "./protocol.js";
import { NATIVE_RUNTIME_REASONING_EFFORTS } from "../../src/lib/aiCapabilities.js";

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
    ).toEqual({ id: "priority", description: "2x speed, increased usage" });
    expect(fastServiceTier({ serviceTiers: [{ id: "priority", name: "Fast" }] })).toEqual({
      id: "priority",
    });
    // The backend owns tier ids: a Fast tier under another id keeps that id.
    expect(fastServiceTier({ serviceTiers: [{ id: "turbo-1", name: "Fast" }] })).toEqual({
      id: "turbo-1",
    });
    // Older app-servers only send the deprecated speed-tier list.
    expect(fastServiceTier({ additionalSpeedTiers: ["fast"] })).toEqual({ id: "fast" });
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
    /** Leave `model/list` unanswered, as a stalled catalog would. */
    stallCatalog?: boolean;
    /**
     * Deserialize `turn/start` tiers as app-servers from before the
     * `"default"` id did: only `null`, `fast` or `flex` (or none).
     */
    legacyServer?: boolean;
    /**
     * The Codex version `initialize` reports (default 0.160.0, which takes
     * `serviceTierForTurn`); null leaves `userAgent` out.
     */
    version?: string | null;
    /** Reject every `turn/start` with this message. */
    rejectTurnStart?: string;
    /** Run the provider status check instead of a chat turn. */
    status?: boolean;
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
 if(m.method==='model/list'&&${JSON.stringify(Boolean(fixture.stallCatalog))}) return;
 if(m.method==='turn/start'&&${JSON.stringify(fixture.rejectTurnStart ?? null)}){
  console.log(JSON.stringify({id:m.id,error:{code:-32600,message:${JSON.stringify(fixture.rejectTurnStart ?? "")}}}));
  return;
 }
 const tier=m.params&&m.params.serviceTier;
 if(m.method==='turn/start'&&${JSON.stringify(Boolean(fixture.legacyServer))}&&tier!=null&&tier!=='fast'&&tier!=='flex'){
  console.log(JSON.stringify({id:m.id,error:{code:-32600,message:'Invalid request: unknown variant \`'+tier+'\`, expected \`fast\` or \`flex\`'}}));
  return;
 }
 const result=m.method==='thread/start'||m.method==='thread/resume'?opened
  :m.method==='model/list'?{data:catalog}
  :m.method==='initialize'?${JSON.stringify(
    fixture.version === null
      ? {}
      : { userAgent: `kassiber/${fixture.version ?? "0.160.0"} (Linux 6.0; x86_64) xterm` },
  )}:{};
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
    let error: unknown;
    let status: Awaited<ReturnType<typeof codexStatus>> | undefined;
    try {
      try {
        if (fixture.status) status = await codexStatus(root);
        else await codexChat(chat("codex", fixture.model ?? "gpt-5.4", fixture.options ?? {}), root);
      } catch (caught) {
        error = caught;
      }
      const wire = (await readFile(capture, "utf8"))
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line) as { method?: string; params?: Record<string, unknown> });
      const events = written.map(
        (line) => JSON.parse(line) as { type: string; phase?: string; provider_session_id?: string },
      );
      return {
        turnStart: wire.findLast((message) => message.method === "turn/start")?.params,
        /** Every tier `turn/start` was sent with, retries included. */
        tiers: wire
          .filter((message) => message.method === "turn/start")
          .map((message) => message.params?.serviceTierForTurn ?? message.params?.serviceTier),
        error,
        status,
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

  it("reads per-turn tier support from the app-server's version", () => {
    for (const agent of [
      "codex_cli_rs/0.151.0 (Mac OS 15.1; arm64) Apple_Terminal",
      "kassiber/0.160.0 (Linux 6.0; x86_64) xterm",
      "kassiber/1.0.0",
    ]) {
      expect(supportsPerTurnServiceTier(agent)).toBe(true);
    }
    for (const agent of [
      "codex_cli_rs/0.150.1 (Linux 6.0; x86_64)",
      "codex_cli_rs/0.151.0-alpha.3 (Linux 6.0; x86_64)",
      // Source builds report 0.0.0; anything unparseable counts as older.
      "codex_cli_rs/0.0.0 (Linux 6.0; x86_64)",
      "0.160.0",
      "",
      undefined,
    ]) {
      expect(supportsPerTurnServiceTier(agent)).toBe(false);
    }
  });

  it.runIf(posix)("overrides the tier for one turn only, never the thread's", async () => {
    const fast = await codexTurn({ options: { fast_mode: true } });
    expect(fast.turnStart?.serviceTierForTurn).toBe(CODEX_FAST_SERVICE_TIER);
    // The thread's saved tier is left alone, so Codex itself never inherits fast.
    expect(fast.turnStart).not.toHaveProperty("serviceTier");
    expect(fast.unavailable).toBe(0);

    for (const options of [{}, { fast_mode: false }]) {
      const standard = await codexTurn({ options, savedTier: "priority" });
      expect(standard.turnStart?.serviceTierForTurn).toBe(CODEX_STANDARD_SERVICE_TIER);
      expect(standard.turnStart).not.toHaveProperty("serviceTier");
    }
  });

  it.runIf(posix)("declines fast on Codex versions without the per-turn field", async () => {
    for (const version of ["0.150.1", "0.151.0-alpha.3", null]) {
      const declined = await codexTurn({ version, options: { fast_mode: true } });
      expect(declined.error).toBeUndefined();
      expect(declined.unavailable).toBe(1);
      // No fast tier anywhere: not on the thread, not for the turn, and no
      // catalog lookup to choose one.
      expect(declined.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
      expect(declined.turnStart).not.toHaveProperty("serviceTierForTurn");
      expect(declined.methods).not.toContain("model/list");

      const standard = await codexTurn({ version, savedTier: "priority" });
      expect(standard.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
      expect(standard.turnStart).not.toHaveProperty("serviceTierForTurn");
      expect(standard.unavailable).toBe(0);
    }
  });

  it.runIf(posix)("offers fast mode only on Codex versions that can honour it", async () => {
    const fastModels = async (version: string | null) =>
      (await codexTurn({ version, status: true })).status?.models
        .filter((model) => model.supports_fast_mode)
        .map((model) => model.id);
    expect(await fastModels("0.160.0")).toEqual(["gpt-5.4"]);
    expect(await fastModels("0.150.1")).toEqual([]);
    expect(await fastModels(null)).toEqual([]);
  });

  it.runIf(posix)("sends the tier id the catalog advertises as Fast", async () => {
    const renamed = await codexTurn({
      options: { fast_mode: true },
      catalog: [{ id: "gpt-5.4", serviceTiers: [{ id: "turbo-1", name: "Fast" }] }],
    });
    expect(renamed.tiers).toEqual(["turbo-1"]);
    expect(renamed.unavailable).toBe(0);
    // Only the deprecated list: Codex still takes its `fast` literal.
    const legacy = await codexTurn({
      options: { fast_mode: true },
      catalog: [{ id: "gpt-5.4", additionalSpeedTiers: ["fast"] }],
    });
    expect(legacy.tiers).toEqual(["fast"]);
    expect(legacy.unavailable).toBe(0);
  });

  it.runIf(posix)("keeps ordinary chat working on app-servers without the default tier id", async () => {
    // The rejected "default" is retried as null, which that version reads as
    // standard speed; a fast request is declined first, with no lookup.
    for (const options of [{}, { fast_mode: true }]) {
      const turn = await codexTurn({ version: null, legacyServer: true, savedTier: "fast", options });
      expect(turn.error).toBeUndefined();
      expect(turn.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER, null]);
      expect(turn.methods).not.toContain("model/list");
      expect(turn.unavailable).toBe(options.fast_mode ? 1 : 0);
    }
  });

  it.runIf(posix)("never retries a rejected per-turn tier", async () => {
    const turn = await codexTurn({
      options: { fast_mode: true },
      rejectTurnStart: "Invalid request: unknown variant `default`",
    });
    expect(turn.error).toBeInstanceOf(Error);
    expect(turn.tiers).toEqual([CODEX_FAST_SERVICE_TIER]);
  });

  it.runIf(posix)("resolves Kassiber's default model through the thread before choosing Fast", async () => {
    const supported = await codexTurn({ model: "default", options: { fast_mode: true } });
    expect(supported.tiers).toEqual([CODEX_FAST_SERVICE_TIER]);
    const unsupported = await codexTurn({
      model: "default",
      threadModel: "gpt-5.4-mini",
      options: { fast_mode: true },
    });
    expect(unsupported.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
    expect(unsupported.unavailable).toBe(1);
  });

  it.runIf(posix)("keeps the standard tier, with a hint, for unsupported or unknown models", async () => {
    for (const model of ["gpt-5.4-mini", "gpt-unknown"]) {
      const turn = await codexTurn({ model, options: { fast_mode: true } });
      expect(turn.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
      expect(turn.turnStart).not.toHaveProperty("serviceTier");
      expect(turn.unavailable).toBe(1);
    }
  });

  it.runIf(posix)("answers at standard speed when the catalog lookup stalls", async () => {
    const previous = fastTierLookup.timeoutMs;
    fastTierLookup.timeoutMs = 100;
    try {
      const turn = await codexTurn({ options: { fast_mode: true }, stallCatalog: true });
      expect(turn.methods).toContain("model/list");
      expect(turn.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
      expect(turn.unavailable).toBe(1);
    } finally {
      fastTierLookup.timeoutMs = previous;
    }
  });

  it.runIf(posix)("pins the standard tier when fast is off, overriding a saved fast config", async () => {
    for (const options of [{}, { fast_mode: false }]) {
      const turn = await codexTurn({ options, savedTier: "fast" });
      expect(turn.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
      expect(turn.unavailable).toBe(0);
      // Fast off needs no catalog lookup at all.
      expect(turn.methods).not.toContain("model/list");
    }
  });

  it.runIf(posix)("returns a resumed fast thread to the standard tier once fast is off", async () => {
    const first = await codexTurn({ options: { fast_mode: true } });
    expect(first.tiers).toEqual([CODEX_FAST_SERVICE_TIER]);
    // A thread whose saved tier is priority (set in Codex itself).
    const next = await codexTurn({
      options: { provider_session_id: first.sessionId },
      savedTier: CODEX_FAST_SERVICE_TIER,
    });
    expect(next.methods).toContain("thread/resume");
    expect(next.tiers).toEqual([CODEX_STANDARD_SERVICE_TIER]);
  });
});

describe("Claude fast mode", () => {
  it("advertises fast mode only on the Opus alias", () => {
    expect(
      CLAUDE_MODELS.filter((model) => model.supports_fast_mode).map((model) => model.id),
    ).toEqual(["opus"]);
  });

  it("turns fast mode on only when requested on Opus, and off explicitly otherwise", async () => {
    const args = await chatArgs(chat("claude", "opus", { fast_mode: true }), "/tmp/empty-fixture");
    expect(args[args.indexOf("--settings") + 1]).toBe('{"fastMode":true}');
    // The variadic tool denial must stay last.
    expect(args.slice(args.indexOf("--disallowed-tools") + 1)).toEqual([...CLAUDE_DISABLED_TOOLS]);
    // Other settings sources stay off.
    expect(args[args.indexOf("--setting-sources") + 1]).toBe("");

    for (const request of [
      chat("claude", "opus"),
      chat("claude", "opus", { fast_mode: false }),
      // A resumed session an earlier turn ran fast: leaving the setting out
      // would not be enough to turn it off.
      chat("claude", "opus", { provider_session_id: "322f88aa-f2bf-4a55-bf83-ed3b41fd535b" }),
      // Accepted by the CLI but silently ignored there, so never requested.
      chat("claude", "sonnet", { fast_mode: true }),
      chat("claude", "default", { fast_mode: true }),
    ]) {
      expect(claudeFastModeApplies(request)).toBe(false);
      const offArgs = await chatArgs(request, "/tmp/empty-fixture");
      expect(offArgs.filter((arg) => arg === "--settings")).toHaveLength(1);
      expect(offArgs[offArgs.indexOf("--settings") + 1]).toBe('{"fastMode":false}');
      expect(offArgs.join(" ")).not.toContain('"fastMode":true');
    }
  });
});

describe("Claude fast mode state", () => {
  // An undefined state leaves the field out, as a CLI older than fast mode does.
  async function events(
    state: string | undefined,
    finalState = state,
    /** Lines Claude prints between `init` and the answer. */
    extra: Array<Record<string, unknown>> = [],
  ) {
    const root = await mkdtemp(join(tmpdir(), "kassiber-fast-claude-"));
    const executable = join(root, "claude");
    await writeFile(
      executable,
      `#!${process.execPath}
process.stdin.resume();
process.stdin.on('end',()=>{
 console.log(JSON.stringify({type:'system',subtype:'init',session_id:'s1',fast_mode_state:${JSON.stringify(state)}}));
 for (const line of ${JSON.stringify(extra)}) console.log(JSON.stringify(line));
 console.log(JSON.stringify({type:'stream_event',event:{type:'content_block_delta',delta:{type:'text_delta',text:'ok'}}}));
 console.log(JSON.stringify({type:'result',subtype:'success',session_id:'s1',fast_mode_state:${JSON.stringify(finalState)}}));
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
      return written.map(
        (line) => JSON.parse(line) as { type: string; phase?: string; message?: string },
      );
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

  it.skipIf(process.platform === "win32")("notes a CLI that never reports a fast mode state", async () => {
    const silent = await events(undefined);
    expect(silent.filter((event) => event.phase === "fast_mode_unavailable")).toHaveLength(1);
    expect(silent.at(-1)?.type).toBe("done");
  });

  it.skipIf(process.platform === "win32")("notes a rate-limit cooldown reported after a fast start", async () => {
    const cooled = await events("on", "cooldown");
    expect(cooled.filter((event) => event.phase === "fast_mode_unavailable")).toHaveLength(1);
    expect(cooled.at(-2)?.phase).toBe("fast_mode_unavailable");
    expect(cooled.at(-1)?.type).toBe("done");
  });

  // The documented out-of-credits notice, as Claude Code 2.1.287 prints it.
  const CREDITS_NOTICE = {
    type: "system",
    subtype: "notification",
    key: "fast-mode-overage-rejected",
    text: "Fast mode disabled \u00b7 usage credits exhausted",
    priority: "immediate",
    color: "error",
    session_id: "s1",
  };

  it.skipIf(process.platform === "win32")("notes a fast turn Claude retried at standard speed after running out of credits", async () => {
    // Fast mode stays "on" throughout; only the notification tells.
    const retried = await events("on", "on", [CREDITS_NOTICE, CREDITS_NOTICE]);
    const notices = retried.filter((event) => event.phase === "fast_mode_unavailable");
    expect(notices).toHaveLength(1);
    expect(retried.at(-2)?.phase).toBe("fast_mode_unavailable");
    expect(retried.at(-1)?.type).toBe("done");
    // Claude's own text never crosses.
    expect(JSON.stringify(retried)).not.toMatch(/usage credits/);

    const other = await events("on", "on", [
      { ...CREDITS_NOTICE, key: "error-compacting-conversation", text: "Error compacting conversation" },
    ]);
    expect(other.some((event) => event.phase === "fast_mode_unavailable")).toBe(false);
  });

  it("recognises only Claude's fast-fallback notification", () => {
    const withoutKey: Record<string, unknown> = { ...CREDITS_NOTICE };
    delete withoutKey.key;
    for (const message of [
      CREDITS_NOTICE,
      withoutKey,
      { ...withoutKey, text: "Fast mode disabled \u00b7 usage credit limit reached" },
      { type: "system", subtype: "notification", key: "fast-mode-overage-rejected" },
    ]) {
      expect(claudeFastFallbackNotification(message)).toBe(true);
    }
    for (const message of [
      { ...withoutKey, subtype: "init" },
      { ...withoutKey, type: "assistant" },
      { ...withoutKey, text: "Fast mode ON" },
      { ...withoutKey, text: "Note: Fast mode disabled \u00b7 usage credits exhausted" },
      { ...withoutKey, text: "Fast mode disabledness" },
      { type: "system", subtype: "notification" },
    ]) {
      expect(claudeFastFallbackNotification(message)).toBe(false);
    }
  });
});

describe("Claude fast mode version floor", () => {
  it("offers fast mode only from the first CLI that honours it under -p", () => {
    expect(claudeVersionSupportsFast("2.1.205 (Claude Code)")).toBe(true);
    expect(claudeVersionSupportsFast("2.1.287 (Claude Code)")).toBe(true);
    expect(claudeVersionSupportsFast("2.2.0")).toBe(true);
    expect(claudeVersionSupportsFast("3.0.0")).toBe(true);
    expect(claudeVersionSupportsFast("2.1.204 (Claude Code)")).toBe(false);
    expect(claudeVersionSupportsFast("2.0.999")).toBe(false);
    expect(claudeVersionSupportsFast("1.9.300")).toBe(false);
    expect(claudeVersionSupportsFast(undefined)).toBe(false);
    expect(claudeVersionSupportsFast("unknown")).toBe(false);
  });

  it("withholds the Opus fast toggle from an older CLI", () => {
    const fastIds = (version: string | undefined) =>
      claudeModelsFor(version)
        .filter((model) => model.supports_fast_mode)
        .map((model) => model.id);
    expect(fastIds("2.1.287 (Claude Code)")).toEqual(["opus"]);
    expect(fastIds("2.1.204 (Claude Code)")).toEqual([]);
    expect(fastIds(undefined)).toEqual([]);
    expect(claudeModelsFor("2.1.204").find((model) => model.id === "opus")).not.toHaveProperty(
      "fast_mode_description",
    );
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
  it("matches Claude's effort levels", () => {
    expect(NATIVE_RUNTIME_REASONING_EFFORTS.claude).toEqual(CLAUDE_MODELS[0]?.reasoning_efforts);
  });

  it("matches Copilot's effort levels", () => {
    expect(NATIVE_RUNTIME_REASONING_EFFORTS.copilot).toEqual(COPILOT_AGENT.efforts);
  });
});
