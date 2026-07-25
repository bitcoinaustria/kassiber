import { access, chmod, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { afterEach, describe, expect, it } from "vitest";

import { CLAUDE_DISABLED_TOOLS } from "./claude.js";
import { CODEX_NON_TOOL_ITEM_TYPES } from "./codex.js";
import {
  providerEnvironment,
  resolveExecutable,
  runProvider,
} from "./executables.js";
import {
  DENY_ALL,
  DISABLED_TOOLS,
  isLoopbackEndpoint,
  parseOpenCodeModels,
} from "./opencode.js";
import { promptFromMessages } from "./prompt.js";
import {
  providerStatus,
  safeErrorMessage,
  safeSessionCursor,
} from "./protocol.js";
import { withWorkingDirectory } from "./working-directory.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true })));
});

describe("provider broker safety boundary", () => {
  it("detects an executable through PATH and reports missing binaries", async () => {
    const root = await mkdtemp(join(tmpdir(), "kassiber-broker-bin-"));
    roots.push(root);
    const command = join(root, "kassiber-test-provider");
    await writeFile(command, "#!/bin/sh\nexit 0\n");
    await chmod(command, 0o755);
    const previousPath = process.env.PATH;
    process.env.PATH = root;
    try {
      expect(await resolveExecutable("kassiber-test-provider")).toBe(command);
      expect(await resolveExecutable("definitely-not-a-provider")).toBeUndefined();
    } finally {
      process.env.PATH = previousPath;
    }
  });

  it("captures a provider's full stdout even when it exits immediately", async () => {
    // Regression: settling on the child's `exit` event loses stdout still queued
    // behind it, so a probe that succeeded looks like an unconfigured provider.
    const root = await mkdtemp(join(tmpdir(), "kassiber-broker-out-"));
    roots.push(root);
    const command = join(root, "kassiber-test-chatty");
    // Write a payload far larger than one pipe buffer, then exit at once.
    await writeFile(command, '#!/bin/sh\nawk \'BEGIN{for(i=0;i<8000;i++)print "0123456789012345678901234"}\'\n');
    await chmod(command, 0o755);
    const expected = 8000 * 26;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const result = await runProvider("codex", command, []);
      expect(result.code).toBe(0);
      expect(result.stdout.length).toBe(expected);
    }
  });

  it("keeps provider auth/config isolated and drops unrelated ambient variables", () => {
    const previous = { ...process.env };
    process.env.HOME = "/tmp/example-home";
    process.env.CODEX_HOME = "/tmp/example-codex";
    process.env.OPENAI_API_KEY = "openai-secret";
    process.env.ANTHROPIC_API_KEY = "anthropic-secret";
    process.env.CLAUDE_CODE_USE_BEDROCK = "1";
    process.env.OPENCODE_CONFIG = "/tmp/example-opencode.json";
    process.env.UNRELATED_PRIVATE_VALUE = "must-not-cross";
    try {
      const codex = providerEnvironment("codex");
      const claude = providerEnvironment("claude");
      const opencode = providerEnvironment("opencode");
      expect(codex.HOME).toBe("/tmp/example-home");
      expect(codex.CODEX_HOME).toBe("/tmp/example-codex");
      expect(codex.OPENAI_API_KEY).toBe("openai-secret");
      expect(codex.ANTHROPIC_API_KEY).toBeUndefined();
      expect(claude.ANTHROPIC_API_KEY).toBe("anthropic-secret");
      expect(claude.CLAUDE_CODE_USE_BEDROCK).toBe("1");
      expect(claude.OPENAI_API_KEY).toBeUndefined();
      expect(opencode.OPENCODE_CONFIG).toBe("/tmp/example-opencode.json");
      expect(opencode.OPENAI_API_KEY).toBe("openai-secret");
      expect(opencode.ANTHROPIC_API_KEY).toBe("anthropic-secret");
      expect(opencode.UNRELATED_PRIVATE_VALUE).toBeUndefined();
    } finally {
      process.env = previous;
    }
  });

  it("uses a fresh empty working directory and removes it afterward", async () => {
    let first = "";
    let second = "";
    await withWorkingDirectory("codex", async (cwd) => {
      first = cwd;
      await expect(access(cwd)).resolves.toBeUndefined();
      expect(await readdir(cwd)).toEqual([]);
    });
    await expect(access(first)).rejects.toThrow();
    await withWorkingDirectory("codex", async (cwd) => {
      second = cwd;
    });
    expect(second).not.toBe(first);
    await expect(access(second)).rejects.toThrow();
  });

  it("removes native tools for Claude and denies every OpenCode permission", () => {
    expect(CODEX_NON_TOOL_ITEM_TYPES.has("userMessage")).toBe(true);
    expect(CODEX_NON_TOOL_ITEM_TYPES.has("commandExecution")).toBe(false);
    expect(CODEX_NON_TOOL_ITEM_TYPES.has("fileChange")).toBe(false);
    expect(CLAUDE_DISABLED_TOOLS).toContain("Bash");
    expect(CLAUDE_DISABLED_TOOLS).toContain("Read");
    expect(DENY_ALL).toEqual([
      { permission: "*", pattern: "*", action: "deny" },
    ]);
    expect(Object.values(DISABLED_TOOLS).every((enabled) => enabled === false)).toBe(true);
  });

  it("sends only the latest user message when resuming a native session", () => {
    const messages = [
      { role: "user", content: "first" },
      { role: "assistant", content: "answer" },
      { role: "user", content: "second" },
    ];
    expect(promptFromMessages(messages, true)).toBe("USER:\nsecond");
    expect(promptFromMessages(messages, false)).toContain("first");
    // No user turn to find: fall back to the trailing message rather than
    // sending nothing.
    expect(promptFromMessages([{ role: "assistant", content: "only" }], true)).toBe(
      "ASSISTANT:\nonly",
    );
    expect(promptFromMessages([], true)).toBe("");
  });

  it("stamps the remote/tools-disabled invariants on every provider status", () => {
    const status = providerStatus("claude", "Claude", {
      state: "ready",
      message: "Ready.",
    });
    expect(status).toMatchObject({
      provider: "claude",
      privacy_posture: "remote",
      native_tools: "disabled",
      models: [],
    });
  });

  it("redacts credential-like values from provider errors", () => {
    const safe = safeErrorMessage(
      new Error("authorization: Bearer-secret token=abc api_key=def"),
    );
    expect(safe).not.toContain("Bearer-secret");
    expect(safe).not.toContain("abc");
    expect(safe).not.toContain("def");
  });

  it("reports OpenCode model sources without claiming unverified local routing", () => {
    const stdout = 'omlx/qwen-local\n{"name":"Qwen Local","variants":{"high":{}}}\n';
    // No endpoint reported for the source provider: stay conservative.
    expect(parseOpenCodeModels(stdout)).toEqual([
      expect.objectContaining({
        id: "omlx/qwen-local",
        source_provider: "omlx",
        privacy_posture: "remote",
        supports_reasoning_effort: true,
      }),
    ]);
    // OpenCode routes the provider off-machine: still remote.
    expect(
      parseOpenCodeModels(stdout, new Map([["omlx", "https://api.example.com/v1"]])),
    ).toEqual([expect.objectContaining({ privacy_posture: "remote" })]);
    // OpenCode routes it to a loopback runtime on this machine: local.
    expect(
      parseOpenCodeModels(stdout, new Map([["omlx", "http://127.0.0.1:8000/v1"]])),
    ).toEqual([expect.objectContaining({ privacy_posture: "local" })]);
  });

  it("counts only loopback endpoints as on-machine", () => {
    for (const url of [
      "http://127.0.0.1:8000/v1",
      "http://127.2.3.4:1/v1",
      "http://localhost:11434/v1",
      "http://app.localhost:3000",
      "http://[::1]:8000/v1",
    ]) {
      expect(isLoopbackEndpoint(url)).toBe(true);
    }
    for (const url of [
      "https://api.anthropic.com/v1",
      // A LAN box is still another machine.
      "http://192.168.1.20:11434/v1",
      "http://10.0.0.5:8000",
      "http://0.0.0.0:8000",
      "not-a-url",
      "",
    ]) {
      expect(isLoopbackEndpoint(url)).toBe(false);
    }
  });

  it("refuses a session cursor that could be read as a CLI flag", () => {
    // These cursors are echoed back from a prior turn and handed to a CLI, so a
    // value starting with `-` would become a new top-level flag.
    for (const hostile of [
      "--dangerously-skip-permissions",
      "-r",
      "--mcp-config=/tmp/evil.json",
      "--debug-file=/tmp/x",
      "id with space",
      "id;rm -rf /",
      "",
      "  ",
      "a".repeat(201),
      null,
      42,
    ]) {
      expect(safeSessionCursor(hostile)).toBeUndefined();
    }
    expect(safeSessionCursor("3146c223-ddaa-4e7a-b1ed-6940080a5a26")).toBe(
      "3146c223-ddaa-4e7a-b1ed-6940080a5a26",
    );
    expect(safeSessionCursor("  ses_01ABC.def:ghi-jkl  ")).toBe("ses_01ABC.def:ghi-jkl");
  });

  it("redacts the secret itself, not merely its label", () => {
    const cases: Array<[string, string[]]> = [
      ["authorization: Bearer sk-secret-123", ["sk-secret-123"]],
      ['{"apiKey":"sk-live-abcdefgh"}', ["sk-live-abcdefgh"]],
      ["connect https://user:pa55word@example.com/v1 failed", ["pa55word"]],
      ["token=ghp_abcdefghijklmnop rejected", ["ghp_abcdefghijklmnop"]],
      ["cannot read /Users/alice/.config/opencode/auth.json", ["alice"]],
    ];
    for (const [input, secrets] of cases) {
      const safe = safeErrorMessage(new Error(input));
      for (const secret of secrets) expect(safe).not.toContain(secret);
    }
  });

  it("treats a 127-prefixed hostname as remote, not loopback", () => {
    // A prefix test on "127." also matches names that resolve publicly.
    expect(isLoopbackEndpoint("http://127.api.example.com/v1")).toBe(false);
    expect(isLoopbackEndpoint("http://127.0.0.1.example.com/v1")).toBe(false);
    expect(isLoopbackEndpoint("http://127.999.0.1/v1")).toBe(false);
    expect(isLoopbackEndpoint("http://127.0.0.1:8000/v1")).toBe(true);
  });

  it("keeps namespaced OpenCode model ids", () => {
    const models = parseOpenCodeModels("openrouter/anthropic/claude-x\n");
    expect(models.map((model) => model.id)).toEqual([
      "openrouter/anthropic/claude-x",
    ]);
    expect(models[0]?.source_provider).toBe("openrouter");
  });
});
