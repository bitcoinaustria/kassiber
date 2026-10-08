import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join } from "node:path";
import { trackDirectory } from "./cleanup.js";
import type { ProviderId } from "./protocol.js";

/**
 * The daemon gives each broker a temporary root it owns and removes once the
 * broker has exited, however it exited (see `_start_broker` in
 * `kassiber/ai/broker_client.py`). Working directories go there so a killed
 * broker leaves nothing behind. The tool bridge stays in the system temp
 * directory: its socket path must stay short enough for macOS.
 */
export function brokerTempRoot(): string {
  const root = process.env.KASSIBER_AI_BROKER_TMPDIR;
  return root && isAbsolute(root) ? root : tmpdir();
}

export async function withWorkingDirectory<T>(
  provider: ProviderId,
  run: (cwd: string) => Promise<T>,
): Promise<T> {
  const cwd = await mkdtemp(join(brokerTempRoot(), `kassiber-ai-${provider}-`));
  const untrack = trackDirectory(cwd);
  try {
    return await run(cwd);
  } finally {
    await rm(cwd, { recursive: true, force: true });
    untrack();
  }
}
