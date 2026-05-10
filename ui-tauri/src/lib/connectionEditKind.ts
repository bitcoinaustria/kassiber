import type { Connection } from "@/mocks/seed";

export type ConnectionEditKind = "descriptor" | "btcpay" | "file-wallet" | null;

export function editConfigKindForConnection(
  connection: Pick<
    Connection,
    "kind" | "syncMode" | "syncSource" | "sourceFormat"
  >,
): ConnectionEditKind {
  const syncSource = connection.syncSource ?? "";
  const syncMode = connection.syncMode ?? "";
  const sourceFormat = connection.sourceFormat ?? "";

  if (
    connection.kind === "btcpay" ||
    syncSource === "btcpay" ||
    syncMode === "btcpay"
  ) {
    return "btcpay";
  }
  if (connection.kind === "descriptor" || connection.kind === "xpub") {
    return "descriptor";
  }
  if (
    syncMode === "file_import" ||
    sourceFormat ||
    connection.kind === "river" ||
    connection.kind === "phoenix" ||
    connection.kind === "csv"
  ) {
    return "file-wallet";
  }
  return null;
}
