import { chmodSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { resolveBridgeDaemonBinary } from "./bridgeDaemonBinary";

const EXECUTABLE = process.platform === "win32" ? "kassiber.exe" : "kassiber";

function writeBinary(file: string) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, "#!/bin/sh\nexit 0\n");
  chmodSync(file, 0o755);
}

describe("resolveBridgeDaemonBinary", () => {
  let repoRoot: string;

  beforeEach(() => {
    repoRoot = realpathSync(mkdtempSync(path.join(os.tmpdir(), "kassiber-bridge-bin-")));
  });

  afterEach(() => {
    rmSync(repoRoot, { recursive: true, force: true });
  });

  it("refuses with build instructions when no binary exists", () => {
    const result = resolveBridgeDaemonBinary(repoRoot, {});
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.message).toContain("KASSIBER_BIN");
    expect(result.message).toContain("cargo build -p kassiber");
    expect(result.message).not.toMatch(/python|uv\b/i);
  });

  it("prefers target/release over target/debug", () => {
    const debug = path.join(repoRoot, "target", "debug", EXECUTABLE);
    writeBinary(debug);
    expect(resolveBridgeDaemonBinary(repoRoot, {})).toEqual({
      ok: true,
      binary: { command: debug, label: `checkout build (${debug})` },
    });

    const release = path.join(repoRoot, "target", "release", EXECUTABLE);
    writeBinary(release);
    expect(resolveBridgeDaemonBinary(repoRoot, {})).toMatchObject({
      ok: true,
      binary: { command: release },
    });
  });

  it("uses KASSIBER_BIN before the checkout build and never falls back past it", () => {
    writeBinary(path.join(repoRoot, "target", "release", EXECUTABLE));
    const configured = path.join(repoRoot, "elsewhere", EXECUTABLE);
    writeBinary(configured);
    expect(resolveBridgeDaemonBinary(repoRoot, { KASSIBER_BIN: ` ${configured} ` })).toEqual({
      ok: true,
      binary: { command: configured, label: `KASSIBER_BIN (${configured})` },
    });

    const missing = path.join(repoRoot, "missing", EXECUTABLE);
    const refused = resolveBridgeDaemonBinary(repoRoot, { KASSIBER_BIN: missing });
    expect(refused.ok).toBe(false);
    if (refused.ok) return;
    expect(refused.message).toContain(JSON.stringify(missing));
  });

  it("ignores a directory where the binary should be", () => {
    mkdirSync(path.join(repoRoot, "target", "release", EXECUTABLE), { recursive: true });
    expect(resolveBridgeDaemonBinary(repoRoot, {}).ok).toBe(false);
  });

  it.skipIf(process.platform === "win32")("ignores a file that is not executable", () => {
    const debug = path.join(repoRoot, "target", "debug", EXECUTABLE);
    writeBinary(debug);
    chmodSync(debug, 0o644);
    expect(resolveBridgeDaemonBinary(repoRoot, {}).ok).toBe(false);
  });
});
