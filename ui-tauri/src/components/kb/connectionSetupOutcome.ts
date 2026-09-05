export interface ConnectionSetupOutcome {
  status: "success" | "partial";
  mutations: string[];
  synced: boolean;
}
const BOOK_WRITES = new Set([
  "ui.wallets.create", "ui.wallets.import_file", "ui.wallets.import_samourai",
  "ui.documents.import_report", "ui.connections.btcpay.create",
  "ui.connections.bullbitcoin_wallet.create", "ui.metadata.bip329.import",
  "ui.backends.create",
]);
export function recordConnectionSetupMutation(outcome: ConnectionSetupOutcome, kind: string, data: unknown): void {
  if (BOOK_WRITES.has(kind)) outcome.mutations.push(kind);
  if (kind !== "ui.wallets.sync") return;
  const results = data && typeof data === "object" && "results" in data ? data.results : null;
  if (!Array.isArray(results) || results.length === 0
    || results.some((row) => !row || row.status !== "synced")) {
    outcome.status = "partial";
    throw new Error("The connection was saved, but its history was not fully synced.");
  }
  outcome.synced = true;
  outcome.mutations.push(kind);
}
