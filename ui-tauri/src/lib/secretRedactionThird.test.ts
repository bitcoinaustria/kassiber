import { createHash } from "node:crypto";
import { createReadStream, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import spec from "../../../tests/fixtures/secret_redaction_third_generator.json";
import { clearAppLogRecords, emitAppLog } from "./appLogs";
import { writeCorpus } from "./redactionCorpus";

it("floors 100,000 third-generator reviewer records identically to Python", async () => {
  // The frozen corpus of the original generator (see redactionCorpus.ts).
  const target = fileURLToPath(new URL("../../../target/", import.meta.url));
  mkdirSync(target, { recursive: true });
  const scratch = mkdtempSync(join(target, "third-redaction-"));
  const file = join(scratch, "cases.jsonl");
  writeCorpus("third", spec.seed, spec.count, file);
  const digest = createHash("sha256");
  try {
    const lines = createInterface({ input: createReadStream(file), crlfDelay: Infinity });
    for await (const line of lines) {
      const testCase = JSON.parse(line) as { i: number; kind: string; input: string; sentinel: string; keep: string };
      if (testCase.i % 100 === 0) clearAppLogRecords();
      const output = emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {} })!.msg;
      const views = [output, output.replaceAll("\\", "")];
      for (let layer = 0; layer < 3; layer += 1) views.push(views.at(-1)!.replace(/%([a-f0-9]{2})/gi, (_, hex: string) => String.fromCharCode(Number.parseInt(hex, 16))));
      for (let start = 0; start < testCase.sentinel.length - 3; start += 1) {
        if (views.some((view) => (testCase.kind.includes("pctlow") ? view.toUpperCase() : view).includes(testCase.sentinel.slice(start, start + 4)))) throw new Error(`Secret at third case ${testCase.i}: ${testCase.input} => ${output}`);
      }
      if (!output.includes(testCase.keep)) throw new Error(`Lost non-secret at third case ${testCase.i}: ${testCase.keep} => ${output}`);
      digest.update(output + "\n");
    }
    expect(digest.digest("hex")).toBe(spec.output_sha256);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  clearAppLogRecords();
}, 120_000);
