import { describe, expect, it } from "vitest";
import { recordConnectionSetupMutation, type ConnectionSetupOutcome } from "./connectionSetupOutcome";

describe("connection setup completion", () => {
  it("does not turn probes, previews or file selection into saved evidence", () => {
    const outcome: ConnectionSetupOutcome = { status: "success", mutations: [], synced: false };
    for (const kind of ["ui.backends.bitcoinrpc.test", "ui.wallets.preview_descriptor", "ui.wallets.detect_script_types"]) {
      recordConnectionSetupMutation(outcome, kind, { reachable: true });
    }
    expect(outcome.mutations).toEqual([]);
  });
  it("records actual save and completed sync separately", () => {
    const outcome: ConnectionSetupOutcome = { status: "success", mutations: [], synced: false };
    recordConnectionSetupMutation(outcome, "ui.wallets.create", { wallet: { label: "Wallet" } });
    expect(outcome.synced).toBe(false);
    recordConnectionSetupMutation(outcome, "ui.wallets.sync", { results: [{ status: "synced" }] });
    expect(outcome).toEqual({ status: "success", mutations: ["ui.wallets.create", "ui.wallets.sync"], synced: true });
  });
  it("preserves the saved wallet when a terminal sync result contains errors", () => {
    for (const results of [[{ status: "error" }], [{ status: "synced" }, { status: "skipped" }], []]) {
      const outcome: ConnectionSetupOutcome = { status: "success", mutations: ["ui.wallets.create"], synced: false };
      expect(() => recordConnectionSetupMutation(outcome, "ui.wallets.sync", { results })).toThrow("connection was saved");
      expect(outcome.status).toBe("partial");
      expect(outcome.mutations).toEqual(["ui.wallets.create"]);
    }
  });
});
