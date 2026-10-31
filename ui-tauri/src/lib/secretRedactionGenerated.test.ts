import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { clearAppLogRecords, emitAppLog } from "./appLogs";
import { generatedCases, spec } from "./redactionGeneration.test-support";

it("floors 100,000 generated records identically to Python, preserving non-secrets", () => {
  const digest = createHash("sha256");
  const python = process.env.KASSIBER_REDACTION_PYTHON_OUTPUT
    ? readFileSync(process.env.KASSIBER_REDACTION_PYTHON_OUTPUT, "utf8").trimEnd().split("\n").map((line) => JSON.parse(line) as string)
    : undefined;
  for (const testCase of generatedCases()) {
    if (testCase.index % 100 === 0) clearAppLogRecords();
    const record = emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {} })!;
    const output = record.msg;
    for (let start = 0; start < testCase.sentinel.length - 3; start += 1) {
      if (output.includes(testCase.sentinel.slice(start, start + 4))) throw new Error(`Secret tail at case ${testCase.index}: ${testCase.input} => ${output}`);
    }
    for (const preserved of testCase.preserved) {
      if (!output.includes(preserved)) throw new Error(`Lost non-secret at case ${testCase.index}: ${preserved} => ${output}`);
    }
    if (python && output !== python[testCase.index]) throw new Error(`Python/TS difference at case ${testCase.index}: ${output} != ${python[testCase.index]}`);
    digest.update(output + "\n");
  }
  expect(digest.digest("hex")).toBe(spec.output_sha256);
  clearAppLogRecords();
}, 120_000);
