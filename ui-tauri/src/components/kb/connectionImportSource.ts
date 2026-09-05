import type { ConnectionCategory, ConnectionSource } from "@/lib/connectionCatalog";

export function isHistoryImportSource(source: ConnectionSource): boolean {
  return Boolean(source.sourceFormat) && !source.forwardTo
    && ["file-wallet", "file-enrichment"].includes(source.setupKind ?? "");
}
export function sourceForConnectionCategory(sources: readonly ConnectionSource[], category: ConnectionCategory,
  historyImport: boolean): ConnectionSource | null {
  return sources.find((source) => source.category === category
    && (!historyImport || isHistoryImportSource(source))) ?? null;
}

/** A stored provider-specific kind can select a parser; a generic wallet cannot. */
export function knownWalletImportSource(
  walletKind: string | undefined,
  sources: readonly ConnectionSource[],
): ConnectionSource | null {
  if (!walletKind || ["custom", "address", "descriptor"].includes(walletKind)) return null;
  const matches = sources.filter((source) => source.walletKind === walletKind
    && source.status === "ready" && isHistoryImportSource(source));
  return matches.length === 1 ? matches[0] : null;
}
