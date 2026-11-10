import { accessSync, constants, statSync } from "node:fs";
import path from "node:path";

/** How the dev bridge starts the daemon: the Rust `kassiber` binary only. */
export interface BridgeDaemonBinary {
  command: string;
  label: string;
}

export type BridgeDaemonBinaryResult =
  | { ok: true; binary: BridgeDaemonBinary }
  | { ok: false; message: string };

function isExecutableFile(candidate: string): boolean {
  try {
    if (!statSync(candidate).isFile()) return false;
    if (process.platform !== "win32") accessSync(candidate, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Pick the Rust binary for the dev bridge: `KASSIBER_BIN` when set, else the
 * checkout's `target/release/kassiber`, else `target/debug/kassiber`.
 *
 * The lookup only checks files on disk. It never runs the candidate (no
 * `--version` probe) and never falls back to Python; a missing binary is an
 * error the preview shows, naming how to build one.
 */
export function resolveBridgeDaemonBinary(
  repoRoot: string,
  env: NodeJS.ProcessEnv = process.env,
): BridgeDaemonBinaryResult {
  const configured = env.KASSIBER_BIN?.trim();
  if (configured) {
    const resolved = path.resolve(repoRoot, configured);
    if (isExecutableFile(resolved)) {
      return {
        ok: true,
        binary: { command: resolved, label: `KASSIBER_BIN (${resolved})` },
      };
    }
    return {
      ok: false,
      message:
        `KASSIBER_BIN=${JSON.stringify(configured)} is not an executable file. ` +
        "Point it at a built kassiber binary, or unset it to use the checkout's target/ build.",
    };
  }

  const executable = process.platform === "win32" ? "kassiber.exe" : "kassiber";
  const candidates = ["release", "debug"].map((profile) =>
    path.join(repoRoot, "target", profile, executable),
  );
  for (const candidate of candidates) {
    if (isExecutableFile(candidate)) {
      return {
        ok: true,
        binary: { command: candidate, label: `checkout build (${candidate})` },
      };
    }
  }
  return {
    ok: false,
    message:
      "Kassiber bridge found no kassiber binary: KASSIBER_BIN is unset and neither " +
      `${candidates.join(" nor ")} exists. Build one with ` +
      "`cargo build -p kassiber --bin kassiber` (add --release for target/release) " +
      "or set KASSIBER_BIN, then restart the preview.",
  };
}
