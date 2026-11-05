import { afterAll, beforeAll, expect, it, vi } from "vitest";
import corpus from "../../../tests/fixtures/operational_redaction_cases.json";
import stress from "../../../tests/fixtures/secret_redaction_stress_cases.json";
import fixes5f from "../../../tests/fixtures/secret_redaction_5f_cases.json";
import fixes5g from "../../../tests/fixtures/secret_redaction_5g_cases.json";
import fixes5h from "../../../tests/fixtures/secret_redaction_5h_cases.json";
import fixes5i from "../../../tests/fixtures/secret_redaction_5i_cases.json";
import scaling from "../../../tests/fixtures/secret_redaction_scaling_cases.json";
import failClosed from "../../../tests/fixtures/secret_redaction_5e_cases.json";
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


it.each(stress)("keeps adversarial secret shape $name within the deadline", (testCase) => {
  const text = testCase.prefix + testCase.repeat.repeat(Math.floor(40_000 / testCase.repeat.length) + 1) + testCase.suffix;
  const start = performance.now();
  logs.emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: text, fields: {} });
  expect(performance.now() - start).toBeLessThan(500);
});

it("scales adversarial secret shapes linearly to 400k characters", () => {
  // Aggregate time: quadratic growth would cost ~100x for ten times the input.
  const total = (size: number) => stress.reduce((sum, testCase) => {
    const text = testCase.prefix + testCase.repeat.repeat(Math.floor(size / testCase.repeat.length) + 1) + testCase.suffix;
    const start = performance.now();
    logs.emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: text, fields: {} });
    logs.clearAppLogRecords();
    return sum + performance.now() - start;
  }, 0);
  const small = total(40_000);
  expect(total(400_000)).toBeLessThan(25 * small + 1_000);
}, 120_000);

function scaled(testCase: (typeof scaling)[number], size: number): string {
  // A staircase indents each repeat one column deeper (or shallower).
  const stair = "staircase" in testCase ? testCase.staircase : undefined;
  if (!stair) return testCase.prefix + testCase.repeat.repeat(Math.floor(size / testCase.repeat.length) + 1) + testCase.suffix;
  const steps = Math.floor(Math.sqrt(2 * size));
  let body = "";
  for (let step = 0; step < steps; step += 1) body += " ".repeat(stair === "down" ? steps - 1 - step : step) + testCase.repeat;
  return testCase.prefix + body + testCase.suffix;
}

it.each(scaling)("scales shape $name linearly to 400k characters", (testCase) => {
  // Each shape on its own: quadratic growth costs ~100x for ten times the input.
  const cost = (size: number) => {
    const text = scaled(testCase, size);
    const start = performance.now();
    logs.emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: text, fields: {} });
    logs.clearAppLogRecords();
    return performance.now() - start;
  };
  cost(4_000);
  const small = cost(40_000);
  expect(small).toBeLessThan(500);
  expect(cost(400_000)).toBeLessThan(25 * small + 50);
}, 60_000);

it("masks oversized records whole, counting code points like Python", () => {
  const emit = (msg: string) => logs.emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg, fields: {} })!.msg;
  expect(emit("😀".repeat(600_000))).toBe("😀".repeat(600_000));
  expect(emit("password=Pw9Leak " + "x".repeat(1_000_000))).toBe("[redacted-oversized]");
  logs.clearAppLogRecords();
});

it.each([...failClosed, ...fixes5f, ...fixes5g, ...fixes5h, ...fixes5i])("fails closed for review regression $id", (testCase) => {
  const output = logs.emitAppLog({ level: "info", module: "test", file: "test", line: 1, msg: testCase.input, fields: {} })!.msg;
  if ("expected" in testCase) expect(output).toBe(testCase.expected);
  if (testCase.sentinel) {
    expect(output).not.toContain(testCase.sentinel);
    for (let start = 0; start < testCase.sentinel.length - 3; start += 1) {
      expect(output).not.toContain(testCase.sentinel.slice(start, start + 4));
    }
  }
  for (const keep of testCase.preserved) expect(output).toContain(keep);
});
