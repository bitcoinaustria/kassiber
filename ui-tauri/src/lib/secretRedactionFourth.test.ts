import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { closeSync, createReadStream, mkdirSync, mkdtempSync, openSync, rmSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import spec from "../../../tests/fixtures/secret_redaction_fourth_generator.json";
import { clearAppLogRecords, emitAppLog } from "./appLogs";

const ENTITIES: Record<string, string> = { quot: "\"", apos: "'", lt: "<", gt: ">", amp: "&", colon: ":", sol: "/", commat: "@", equals: "=" };

function views(output: string): string[] {
  const result = [output, output.replaceAll("\\", "")];
  for (const start of [...result]) {
    let view = start;
    for (let layer = 0; layer < 3; layer += 1) {
      view = view.replace(/%([a-f0-9]{2})/gi, (_, hex: string) => String.fromCharCode(Number.parseInt(hex, 16)));
      result.push(view);
    }
  }
  return [...result, ...result.map((view) => view.replace(/&(?:#([0-9]{1,7})|#[xX]([0-9a-fA-F]{1,6})|([A-Za-z]+));/g, (entity, decimal?: string, hex?: string, name?: string) => {
    const code = decimal ? Number.parseInt(decimal, 10) : hex ? Number.parseInt(hex, 16) : -1;
    if (code >= 0) return code <= 0x10ffff ? String.fromCodePoint(code) : entity;
    return ENTITIES[name!] ?? entity;
  }))];
}

async function generated(count: number, seed: number, visit: (testCase: { i: number; kind: string; input: string; sentinel: string; keep: string[] }, output: string) => void) {
  // Run the original Python RNG and distribution, rather than porting the
  // generator or materializing a huge checked-in corpus.
  const generator = fileURLToPath(new URL("../../../tests/redaction_fourth_generation.py", import.meta.url));
  const target = fileURLToPath(new URL("../../../target/", import.meta.url));
  mkdirSync(target, { recursive: true });
  const scratch = mkdtempSync(join(target, "fourth-redaction-"));
  const file = join(scratch, "cases.jsonl");
  const fd = openSync(file, "w");
  try {
    execFileSync(process.env.KASSIBER_TEST_PYTHON ?? "python3", [generator, String(count), String(seed)], { stdio: ["ignore", fd, "pipe"] });
  } finally {
    closeSync(fd);
  }
  try {
    const lines = createInterface({ input: createReadStream(file), crlfDelay: Infinity });
    for await (const line of lines) {
      const testCase = JSON.parse(line) as { i: number; kind: string; input: string; sentinel: string; keep: string[] };
      if (testCase.i % 100 === 0) clearAppLogRecords();
      const output = emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {} })!.msg;
      const all = views(output);
      for (let start = 0; start < testCase.sentinel.length - 3; start += 1) {
        if (all.some((view) => view.includes(testCase.sentinel.slice(start, start + 4)))) throw new Error(`Secret at fourth case ${seed}/${testCase.i}: ${testCase.input} => ${output}`);
      }
      for (const keep of testCase.keep) if (!output.includes(keep)) throw new Error(`Lost non-secret at fourth case ${seed}/${testCase.i}: ${keep} => ${output}`);
      visit(testCase, output);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
    clearAppLogRecords();
  }
}

it("floors 100,000 fourth-generator reviewer records identically to Python", async () => {
  const digest = createHash("sha256");
  await generated(spec.count, spec.seed, (_, output) => digest.update(output + "\n"));
  expect(digest.digest("hex")).toBe(spec.output_sha256);
}, 120_000);

it.each(spec.extra_seeds)("floors fourth-generator seed %i", async (seed) => {
  await generated(spec.extra_count, seed, () => undefined);
}, 120_000);
