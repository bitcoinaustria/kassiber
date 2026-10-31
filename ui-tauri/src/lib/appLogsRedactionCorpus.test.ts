import { afterAll, beforeAll, expect, it, vi } from "vitest";
import corpus from "../../../tests/fixtures/operational_redaction_cases.json";
import secrets from "../../../tests/fixtures/secret_redaction_cases.json";

let logs: typeof import("./appLogs");

beforeAll(async () => {
  vi.resetModules();
  vi.stubGlobal("crypto", {
    getRandomValues: (bytes: Uint32Array) => {
      bytes.set([1, 2]); // corpus salt: 0000000100000002
      return bytes;
    },
  });
  logs = await import("./appLogs");
});
afterAll(() => vi.unstubAllGlobals());

it.each(corpus.cases)("matches Python exactly for $input", (testCase) => {
  for (const mode of ["high_signal", "public_safe"] as const) {
    const record = logs.emitAppLog({
      level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {},
    });
    expect(logs.redactLogRecord(record!, { redacted: true, maskAmounts: true, mode }).msg).toBe(testCase.expected);
  }
});

it.each(secrets)("floors the same secret as Python for $input", (testCase) => {
  const record = logs.emitAppLog({
    level: "info", module: "test", file: "test", line: 1, msg: testCase.input,
    fields: { detail: { type: "text", value: testCase.input } },
  });
  expect(record!.msg).toBe(testCase.expected);
  expect(record!.fields.detail.value).toBe(testCase.expected);
});

it.each(["-", "."])("redacts 100k characters in well under a second (%s)", (separator) => {
  const text = `word${separator}`.repeat(20_000);
  const start = performance.now();
  const record = logs.emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: text, fields: {} });
  expect(record!.msg).toBe(text);
  expect(performance.now() - start).toBeLessThan(500);
});
