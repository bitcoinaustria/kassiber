import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { trackDirectory } from "./cleanup.js";
import type { ProviderId } from "./protocol.js";

export async function withWorkingDirectory<T>(
  provider: ProviderId,
  run: (cwd: string) => Promise<T>,
): Promise<T> {
  const cwd = await mkdtemp(join(tmpdir(), `kassiber-ai-${provider}-`));
  const untrack = trackDirectory(cwd);
  try {
    return await run(cwd);
  } finally {
    await rm(cwd, { recursive: true, force: true });
    untrack();
  }
}
