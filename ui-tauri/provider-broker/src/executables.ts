import { spawn } from "node:child_process";
import { access } from "node:fs/promises";
import { constants } from "node:fs";
import { delimiter, join } from "node:path";
import { homedir } from "node:os";
import type { ProviderId } from "./protocol.js";

const FALLBACK_DIRS = [
  "/opt/homebrew/bin",
  "/usr/local/bin",
  join(homedir(), ".local", "bin"),
  join(homedir(), ".opencode", "bin"),
  "/opt/local/bin",
];

async function executable(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

export async function resolveExecutable(command: string): Promise<string | undefined> {
  const directories = [
    ...(process.env.PATH?.split(delimiter).filter(Boolean) ?? []),
    ...FALLBACK_DIRS,
  ];
  for (const directory of [...new Set(directories)]) {
    const candidate = join(directory, command);
    if (await executable(candidate)) return candidate;
  }
  return undefined;
}

/**
 * Run a provider CLI to completion and return its stdout.
 *
 * Settles on `close`, not `exit`: `exit` fires as soon as the child terminates,
 * while its stdout data events can still be queued behind it. With several
 * probes running concurrently that race is routinely lost, and the caller sees
 * empty output for a command that actually succeeded — which then reads as
 * "provider not configured".
 *
 * stderr is discarded deliberately; provider CLIs put credentials and machine
 * detail there, and `safeErrorMessage` is the only sanctioned path for
 * surfacing provider failures.
 */
export async function runProvider(
  provider: ProviderId,
  executable: string,
  args: string[],
  options: { cwd?: string; limit?: number } = {},
): Promise<{ code: number | null; stdout: string }> {
  const limit = options.limit ?? 512_000;
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      ...(options.cwd === undefined ? {} : { cwd: options.cwd }),
      env: providerEnvironment(provider),
      stdio: ["ignore", "pipe", "ignore"],
    });
    let stdout = "";
    child.stdout.on("data", (chunk) => {
      if (stdout.length < limit) stdout += String(chunk);
    });
    child.once("error", reject);
    child.once("close", (code) => resolve({ code, stdout }));
  });
}

export function providerEnvironment(provider: ProviderId): NodeJS.ProcessEnv {
  const allowed = new Set<string>([
    "HOME",
    "USER",
    "USERNAME",
    "PATH",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
  ]);
  const providerPrefixes: string[] = [];
  if (provider === "codex" || provider === "opencode") {
    [
      "CODEX_HOME",
      "CODEX_API_KEY",
      "CODEX_ACCESS_TOKEN",
      "OPENAI_BASE_URL",
      "OPENAI_API_KEY",
    ].forEach((name) => allowed.add(name));
  }
  if (provider === "claude" || provider === "opencode") {
    [
      "ANTHROPIC_BASE_URL",
      "ANTHROPIC_API_KEY",
      "ANTHROPIC_AUTH_TOKEN",
      "CLAUDE_CODE_OAUTH_TOKEN",
    ].forEach((name) => allowed.add(name));
    providerPrefixes.push("ANTHROPIC_", "CLAUDE_", "AWS_", "GOOGLE_", "CLOUDSDK_");
  }
  if (provider === "opencode") providerPrefixes.push("OPENCODE_");
  const env: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (allowed.has(name) || providerPrefixes.some((prefix) => name.startsWith(prefix))) {
      env[name] = value;
    }
  }
  env.KASSIBER_AI_BROKER = "1";
  return env;
}
