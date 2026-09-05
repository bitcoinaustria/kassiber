import { describe, expect, it, vi } from "vitest";
import { mkdtemp, writeFile, readFile, readlink, chmod, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { chatArgs, claudeChat } from "./claude.js";
import { codexChat, sensitiveCodexArgs, sensitiveCodexConfig, SENSITIVE_CODEX_PROFILE } from "./codex.js";
import { sensitiveOpenCodeConfig, sensitiveOpenCodeEnvironment, verifySensitiveOpenCodeConfig } from "./opencode.js";
import { sensitiveContext, type ChatRequest } from "./protocol.js";

const request: ChatRequest = {
  command: "chat", request_id: "selected", provider: "claude", model: "fable",
  messages: [{ role: "user", content: "synthetic selected evidence" }],
  options: { sensitive_context: true, reasoning_effort: "high" },
};

describe("selected sensitive provider boundary", () => {
  it("rejects session continuation, tools, and malformed options", () => {
    expect(sensitiveContext(request)).toBe(true);
    expect(sensitiveContext({ ...request, options: {} })).toBe(false);
    expect(() => sensitiveContext({ ...request, options: { ...request.options, provider_session_id: "prior" } })).toThrow();
    expect(() => sensitiveContext({ ...request, tools: [{ name: "read", description: "read", parameters: {} }] })).toThrow();
    expect(() => sensitiveContext({ ...request, options: { sensitive_context: "true" as never } })).toThrow();
  });

  it("Claude disables persistence without changing the selected model or ordinary chat", async () => {
    const args = await chatArgs(request, "/tmp/empty-fixture");
    expect(args).toContain("--no-session-persistence");
    expect(args[args.indexOf("--model") + 1]).toBe("fable");
    expect(args).not.toContain("--resume");
    expect(args[args.indexOf("--tools") + 1]).toBe("");
    expect(await chatArgs({ ...request, options: {} }, "/tmp/empty-fixture")).not.toContain("--no-session-persistence");
  });

  it("Codex installs restricted startup settings before opening any thread", () => {
    const config = sensitiveCodexConfig();
    expect(config.permissions[SENSITIVE_CODEX_PROFILE]).toEqual({
      filesystem: { ":root": "deny", ":minimal": "read", ":workspace_roots": "read" },
      network: { enabled: false },
    });
    expect(Object.values(config.features).every((value) => value === false)).toBe(true);
    expect(config.features.shell_tool).toBe(false);
    expect(config.features.code_mode_host).toBe(false);
    expect(config.features.multi_agent).toBe(false);
    expect(config.features.browser_use).toBe(false);
    expect(sensitiveCodexArgs()).toContain('history={"persistence"="none"}');
    expect(config.project_doc_max_bytes).toBe(0);
    expect(config.tools.view_image).toBe(false);
  });

  it("OpenCode disables secondary models and rejects effective configuration drift", () => {
    const model = "local/selected-model";
    const config = sensitiveOpenCodeConfig(model);
    expect(() => verifySensitiveOpenCodeConfig(config, model)).not.toThrow();
    for (const changed of [
      { ...config, model: "remote/other" },
      { ...config, small_model: "remote/title-model" },
      { ...config, share: "auto" },
      { ...config, snapshot: true },
      { ...config, compaction: { auto: true, prune: false } },
      { ...config, permission: { "*": "deny", read: "allow" } },
      { ...config, agent: { ...config.agent, title: { disable: false } } },
    ]) expect(() => verifySensitiveOpenCodeConfig(changed, model)).toThrow("no selected data");
  });

  it("rejects an unverified Codex thread before sending any selected prompt", async () => {
    const root = await mkdtemp(join(tmpdir(), "kassiber-private-provider-test-"));
    const capture = join(root, "methods.jsonl");
    const executable = join(root, "codex");
    await writeFile(executable, `#!${process.execPath}
const {createInterface}=require('node:readline');
const {appendFileSync}=require('node:fs');
createInterface({input:process.stdin}).on('line',line=>{
 const m=JSON.parse(line); appendFileSync(${JSON.stringify(capture)},JSON.stringify(m)+'\\n');
 if(m.id) console.log(JSON.stringify({id:m.id,result:m.method==='thread/start'?{thread:{id:'fixture',ephemeral:false},activePermissionProfile:{id:'kassiber-selected-context'},instructionSources:[]}:{}}));
});
`);
    await chmod(executable, 0o755);
    const previous = process.env.PATH;
    process.env.PATH = root;
    const output = vi.spyOn(process.stdout, "write").mockReturnValue(true);
    try {
      await expect(codexChat({ ...request, provider: "codex" }, root)).rejects.toThrow("sensitive-context request failed");
      const wire = await readFile(capture, "utf8");
      expect(wire).not.toContain("turn/start");
      expect(wire).not.toContain("synthetic selected evidence");
      expect(wire).not.toContain("thread/resume");
    } finally {
      output.mockRestore(); process.env.PATH = previous;
      await rm(root, { recursive: true, force: true });
    }
  });

  it("rejects old Claude before launching a sensitive chat", async () => {
    const root = await mkdtemp(join(tmpdir(), "kassiber-private-provider-test-"));
    const executable = join(root, "claude");
    await writeFile(executable, `#!${process.execPath}\nconsole.log('older help without ephemeral support');\n`);
    await chmod(executable, 0o755);
    const previous = process.env.PATH;
    process.env.PATH = root;
    try {
      await expect(claudeChat(request, root)).rejects.toThrow("does not support stateless");
    } finally {
      process.env.PATH = previous; await rm(root, { recursive: true, force: true });
    }
  });

  it.skipIf(process.platform === "win32")("installs OpenCode log discard before the in-memory database probe", async () => {
    const root = await mkdtemp(join(tmpdir(), "kassiber-private-provider-test-"));
    const executable = join(root, "opencode");
    await writeFile(executable, `#!${process.execPath}
const {appendFileSync}=require('node:fs');
if(process.argv.includes('--version')) console.log('1.18.4');
else {
 appendFileSync(process.env.XDG_DATA_HOME+'/opencode/log/opencode.log','synthetic log content must be discarded');
 console.log(process.env.OPENCODE_DB);
}
`);
    await chmod(executable, 0o755);
    try {
      const env = await sensitiveOpenCodeEnvironment(executable, root, "local/fixture");
      const log = join(root, "private-runtime/data/opencode/log/opencode.log");
      expect(await readlink(log)).toBe("/dev/null");
      expect(await readFile(log, "utf8")).toBe("");
      expect(env.OPENCODE_DB).toBe(":memory:");
      expect(env.OPENCODE_AUTO_SHARE).toBe("0");
      expect(JSON.parse(env.OPENCODE_CONFIG_CONTENT!).small_model).toBe("local/fixture");
    } finally { await rm(root, { recursive: true, force: true }); }
  });
});
