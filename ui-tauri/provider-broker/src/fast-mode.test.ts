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
import { CODEX_FAST_SERVICE_TIER, codexChat, fastServiceTier } from "./codex.js";
import { requestedFastMode, type ChatRequest } from "./protocol.js";

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

  async function turnStartParams(options: ChatRequest["options"]) {
    const root = await mkdtemp(join(tmpdir(), "kassiber-fast-codex-"));
    const capture = join(root, "methods.jsonl");
    const executable = join(root, "codex");
    await writeFile(
      executable,
      `#!${process.execPath}
const {createInterface}=require('node:readline');
const {appendFileSync}=require('node:fs');
createInterface({input:process.stdin}).on('line',line=>{
 const m=JSON.parse(line); appendFileSync(${JSON.stringify(capture)},JSON.stringify(m)+'\\n');
 if(!m.id) return;
 const result=m.method==='thread/start'?{thread:{id:'fixture'}}:{};
 console.log(JSON.stringify({id:m.id,result}));
 if(m.method==='turn/start') console.log(JSON.stringify({method:'turn/completed',params:{turn:{status:'completed'}}}));
});
`,
    );
    await chmod(executable, 0o755);
    const previous = process.env.PATH;
    process.env.PATH = root;
    const output = vi.spyOn(process.stdout, "write").mockReturnValue(true);
    try {
      await codexChat(chat("codex", "gpt-5.4", options), root);
      const wire = (await readFile(capture, "utf8"))
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line) as { method?: string; params?: Record<string, unknown> });
      return wire.find((message) => message.method === "turn/start")?.params;
    } finally {
      output.mockRestore();
      process.env.PATH = previous;
      await rm(root, { recursive: true, force: true });
    }
  }

  it.skipIf(process.platform === "win32")("sends the priority tier on turn/start only when fast was requested", async () => {
    expect((await turnStartParams({ fast_mode: true }))?.serviceTier).toBe(CODEX_FAST_SERVICE_TIER);
    expect(await turnStartParams({ fast_mode: false })).not.toHaveProperty("serviceTier");
    expect(await turnStartParams({})).not.toHaveProperty("serviceTier");
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
