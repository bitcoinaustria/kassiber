import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { openCodeChat, openCodeStatus } from "./opencode.js";

const roots: string[] = [];
const previousPath = process.env.PATH;
afterEach(async () => {
  process.env.PATH = previousPath;
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

async function fakeOpenCode(version: string, modelsCode = 0) {
  const root = await mkdtemp(join(tmpdir(), "kassiber-opencode-version-"));
  roots.push(root);
  const executable = join(root, "opencode");
  await writeFile(executable, `#!${process.execPath}
const {appendFileSync}=require('node:fs');
appendFileSync(${JSON.stringify(join(root, "calls.jsonl"))},JSON.stringify(process.argv.slice(2))+'\\n');
const command=process.argv[2];
if(command==='--version') console.log(${JSON.stringify(version)});
else if(command==='debug') console.log('{}');
else if(command==='models') { console.log('provider/model'); process.exit(${modelsCode}); }
else process.exit(1);
`);
  await chmod(executable, 0o755);
  process.env.PATH = root;
  return { root, calls: async () => (await readFile(join(root, "calls.jsonl"), "utf8"))
    .trim().split("\n").map((line) => JSON.parse(line)) };
}

describe.skipIf(process.platform === "win32")("OpenCode CLI compatibility", () => {
  it.each(["opencode v2.2.0-beta.1", "opencode v2.2.0+custom"])("refuses unaudited channel builds (%s) before starting a server", async (version) => {
    const fixture = await fakeOpenCode(version);
    const status = await openCodeStatus(fixture.root);
    expect(status.state).toBe("error");
    expect(status.message).toContain("stable OpenCode 2");
    expect(await fixture.calls()).toEqual([["--version"]]);
  });
  it("reports an unknown future major before config, discovery, or chat starts", async () => {
    const fixture = await fakeOpenCode("opencode v3.0.0");
    const status = await openCodeStatus(fixture.root);
    expect(status.state).toBe("error");
    expect(status.message).toMatch(/OpenCode major version.*not supported/);
    expect(status.models).toEqual([]);
    await expect(openCodeChat({
      command: "chat", request_id: "fixture", provider: "opencode", model: "provider/model",
      messages: [{ role: "user", content: "synthetic prompt" }],
    }, fixture.root)).rejects.toThrow(/OpenCode major version.*not supported/);
    expect(await fixture.calls()).toEqual([["--version"], ["--version"]]);
  });

  it("still discovers models on a compatible v1 CLI", async () => {
    const fixture = await fakeOpenCode("1.18.10");
    const status = await openCodeStatus(fixture.root);
    expect(status.state).toBe("ready");
    expect(status.version).toBe("1.18.10");
    expect(status.models.map((model) => model.id)).toEqual(["provider/model"]);
  });

  it("reports a failed model command as a check error rather than a login problem", async () => {
    const fixture = await fakeOpenCode("1.18.10", 1);
    const status = await openCodeStatus(fixture.root);
    expect(status.state).toBe("error");
    expect(status.message).toMatch(/could not list models/);
    expect(status.models).toEqual([]);
  });
});
