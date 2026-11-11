import { createHash } from "node:crypto";
import { createReadStream, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import spec from "../../../tests/fixtures/secret_redaction_independent_generator.json";
import { clearAppLogRecords, emitAppLog } from "./appLogs";
import { writeCorpus } from "./redactionCorpus";

it("floors 100,000 independently generated reviewer records identically to Python", async () => {
  // The frozen corpus of the original generator (see redactionCorpus.ts).
  const target = fileURLToPath(new URL("../../../target/", import.meta.url));
  mkdirSync(target, { recursive: true });
  const scratch = mkdtempSync(join(target, "independent-redaction-"));
  const file = join(scratch, "cases.jsonl");
  writeCorpus("independent", spec.seed, spec.count, file);
  const digest = createHash("sha256");
  try {
    const lines = createInterface({ input: createReadStream(file), crlfDelay: Infinity });
    for await (const line of lines) {
      const testCase = JSON.parse(line) as { index: number; input: string; sentinel: string; preserved: string[] };
      if (testCase.index % 100 === 0) clearAppLogRecords();
      const output = emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {} })!.msg;
      for (let start = 0; start < testCase.sentinel.length - 3; start += 1) {
        if (output.includes(testCase.sentinel.slice(start, start + 4))) throw new Error(`Secret at independent case ${testCase.index}: ${testCase.input} => ${output}`);
      }
      for (const preserved of testCase.preserved) {
        if (!output.includes(preserved)) throw new Error(`Lost non-secret at independent case ${testCase.index}: ${preserved} => ${output}`);
      }
      digest.update(output + "\n");
    }
    expect(digest.digest("hex")).toBe(spec.output_sha256);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  clearAppLogRecords();
}, 120_000);
