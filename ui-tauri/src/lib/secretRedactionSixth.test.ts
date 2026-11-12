import { createHash } from "node:crypto";
import { createReadStream, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import spec from "../../../tests/fixtures/secret_redaction_sixth_generator.json";
import { clearAppLogRecords, emitAppLog } from "./appLogs";
import { writeCorpus } from "./redactionCorpus";

type SixthCase = { i: number; kind: string; input: string; secrets: [string, string][]; numeric: [string, string][] };

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

function leaks(testCase: SixthCase, output: string): string[] {
  const all = views(output);
  const planted = testCase.secrets.map(([secret]) => secret);
  const found: string[] = [];
  // Every class is inside the contract.
  for (const [secret] of testCase.secrets) {
    const blanked = testCase.input.replaceAll(secret, "\0");
    const fragments = new Set<string>();
    for (let start = 0; start < secret.length - 3; start += 1) {
      const fragment = secret.slice(start, start + 4);
      if (!planted.some((other) => other !== secret && other.includes(fragment)) && !blanked.includes(fragment)) fragments.add(fragment);
    }
    if (all.some((view) => [...fragments].some((fragment) => view.includes(fragment)))) found.push(secret);
  }
  for (const [number] of testCase.numeric) {
    // A code that also appears as readable data (a token count) is ambiguous.
    const pattern = new RegExp(`(?<![0-9])${number}(?![0-9])`, "g");
    if ((testCase.input.match(pattern) ?? []).length === 1 && pattern.test(output)) found.push(number);
  }
  return found;
}

async function generated(count: number, seed: number, visit: (output: string) => void) {
  // The frozen corpus of the original generator (see redactionCorpus.ts).
  const target = fileURLToPath(new URL("../../../target/", import.meta.url));
  mkdirSync(target, { recursive: true });
  const scratch = mkdtempSync(join(target, "sixth-redaction-"));
  const file = join(scratch, "cases.jsonl");
  writeCorpus("sixth", seed, count, file);
  try {
    const lines = createInterface({ input: createReadStream(file), crlfDelay: Infinity });
    for await (const line of lines) {
      const testCase = JSON.parse(line) as SixthCase;
      if (testCase.i % 100 === 0) clearAppLogRecords();
      const output = emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {} })!.msg;
      const found = leaks(testCase, output);
      if (found.length) throw new Error(`Secret at sixth case ${seed}/${testCase.i}: ${found.join(", ")}: ${testCase.input} => ${output}`);
      visit(output);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
    clearAppLogRecords();
  }
}

it("floors 100,000 sixth-generator reviewer records identically to Python", async () => {
  const digest = createHash("sha256");
  await generated(spec.count, spec.seed, (output) => digest.update(output + "\n"));
  expect(digest.digest("hex")).toBe(spec.output_sha256);
}, 180_000);

it.each(spec.extra_seeds)("floors sixth-generator seed %i", async (seed) => {
  await generated(spec.extra_count, seed, () => undefined);
}, 180_000);
