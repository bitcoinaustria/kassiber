import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { BridgeDataRoot, devDataRootSeed } from "./bridgeDataRoot";
import { runImportProjectAction } from "./importProject";

const RUNNER_ARGS = ["-m", "kassiber"];

// A plaintext database only has to carry the SQLite header and the table
// names the inspector looks for; no daemon ever opens it here.
function writeProject(root: string) {
  const dataRoot = path.join(root, "data");
  mkdirSync(dataRoot, { recursive: true });
  writeFileSync(
    path.join(dataRoot, "kassiber.sqlite3"),
    Buffer.concat([
      Buffer.from("SQLite format 3\0", "ascii"),
      Buffer.from(
        "CREATE TABLE settings; CREATE TABLE workspaces; CREATE TABLE profiles (workspace_id, fiat_currency);",
      ),
    ]),
  );
  return realpathSync(dataRoot);
}

describe("devDataRootSeed", () => {
  it("reads KASSIBER_DEV_DATA_ROOT and treats a blank value as unset", () => {
    expect(devDataRootSeed({ KASSIBER_DEV_DATA_ROOT: " /books/scratch/data " })).toBe(
      "/books/scratch/data",
    );
    expect(devDataRootSeed({ KASSIBER_DEV_DATA_ROOT: "  " })).toBeNull();
    expect(devDataRootSeed({})).toBeNull();
  });
});

describe("runImportProjectAction", () => {
  let tmp: string;
  let importedDataRoot: string;
  let restarts: number;
  let approvedDataRoots: Set<string>;

  beforeEach(() => {
    tmp = mkdtempSync(path.join(os.tmpdir(), "kassiber-bridge-"));
    importedDataRoot = writeProject(path.join(tmp, "imported"));
    restarts = 0;
    approvedDataRoots = new Set();
  });

  afterEach(() => {
    rmSync(tmp, { recursive: true, force: true });
  });

  function bridge(seed: string | null) {
    const dataRoot = new BridgeDataRoot(seed, () => {
      restarts += 1;
    });
    const run = (request: Record<string, unknown>) =>
      runImportProjectAction(request, {
        dataRoot,
        approvedDataRoots,
        pickDirectory: async () => path.dirname(importedDataRoot),
      });
    return { dataRoot, run };
  }

  it("clear returns to the env-seeded book, not the implicit default root", async () => {
    const seed = path.join(tmp, "scratch", "data");
    const { dataRoot, run } = bridge(seed);
    expect(dataRoot.daemonArgs(RUNNER_ARGS)).toEqual([
      ...RUNNER_ARGS,
      "--data-root",
      seed,
      "daemon",
    ]);

    const picked = await run({ action: "select" });
    expect(picked).toMatchObject({ selection: { dataRoot: importedDataRoot } });
    expect(dataRoot.get()).toBe(importedDataRoot);
    expect(restarts).toBe(1);

    await expect(run({ action: "clear" })).resolves.toEqual({ ok: true });
    expect(dataRoot.get()).toBe(seed);
    expect(restarts).toBe(2);
    expect(dataRoot.daemonArgs(RUNNER_ARGS)).toEqual([
      ...RUNNER_ARGS,
      "--data-root",
      seed,
      "daemon",
    ]);

    // Clearing while already on the launch book keeps the running daemon.
    await run({ action: "clear" });
    expect(restarts).toBe(2);
  });

  it("clear drops --data-root only when the bridge was never seeded", async () => {
    const { dataRoot, run } = bridge(null);
    await run({ action: "select" });
    expect(dataRoot.daemonArgs(RUNNER_ARGS)).toContain(importedDataRoot);

    await run({ action: "clear" });
    expect(dataRoot.get()).toBeNull();
    expect(dataRoot.daemonArgs(RUNNER_ARGS)).toEqual([...RUNNER_ARGS, "daemon"]);
  });

  it("activate refuses a root the picker never approved", async () => {
    const seed = path.join(tmp, "scratch", "data");
    const { dataRoot, run } = bridge(seed);
    await expect(
      run({ action: "activate", dataRoot: importedDataRoot }),
    ).rejects.toThrow(/native folder picker/);
    expect(dataRoot.get()).toBe(seed);
    expect(restarts).toBe(0);

    await run({ action: "select" });
    await run({ action: "clear" });
    await expect(
      run({ action: "activate", dataRoot: importedDataRoot }),
    ).resolves.toMatchObject({ selection: { dataRoot: importedDataRoot } });
    expect(dataRoot.get()).toBe(importedDataRoot);
  });

  it("returns null for an unknown action", async () => {
    const { run } = bridge(null);
    await expect(run({ action: "delete" })).resolves.toBeNull();
  });
});
