import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";
import type {
  TransactionSwapRoute,
  TransactionSwapRouteLegKey,
} from "./TransactionGraphModel";

function normalizedReferenceSet(references: Array<string | null | undefined>) {
  return new Set(
    references
      .map((reference) => reference?.trim().toLowerCase())
      .filter((reference): reference is string => Boolean(reference)),
  );
}

function swapRouteLegReference(
  route: TransactionSwapRoute | null | undefined,
  leg: TransactionSwapRouteLegKey,
) {
  const routeLeg = route?.[leg];
  return routeLeg?.id || routeLeg?.txid || routeLeg?.externalId || null;
}

function swapRouteLegHasLocalRow(
  route: TransactionSwapRoute | null | undefined,
  leg: TransactionSwapRouteLegKey,
) {
  const routeLeg = route?.[leg];
  const id = routeLeg?.id?.trim();
  if (!id) return false;
  return swapRouteLegReference(route, leg)?.trim().toLowerCase() === id.toLowerCase();
}

export function preloadableSwapLegGraphReference(
  route: TransactionSwapRoute | null | undefined,
  leg: TransactionSwapRouteLegKey,
  currentReferences: Array<string | null | undefined>,
) {
  const reference = swapRouteLegReference(route, leg)?.trim();
  if (!reference) return null;
  if (normalizedReferenceSet(currentReferences).has(reference.toLowerCase())) {
    return null;
  }
  return reference;
}

export function preloadableSwapLegGraphLookupArgs(
  route: TransactionSwapRoute | null | undefined,
  leg: TransactionSwapRouteLegKey,
  currentReferences: Array<string | null | undefined>,
) {
  const transaction = preloadableSwapLegGraphReference(route, leg, currentReferences);
  return {
    transaction: transaction ?? "",
    allowPublicLookup: Boolean(transaction && swapRouteLegHasLocalRow(route, leg)),
  };
}

function looksLikeTxid(value: string | null | undefined) {
  return /^[0-9a-f]{64}$/i.test(value?.trim() ?? "");
}

function hasPublicGraphLookupReference(
  transaction: TransactionDetailTabContext["transaction"] | null | undefined,
) {
  if (!transaction || !looksLikeTxid(transaction.explorerId)) return false;
  return transaction.paymentMethod === "On-chain" || transaction.paymentMethod === "Liquid";
}

export function transactionGraphLookupArgs(
  transaction: TransactionDetailTabContext["transaction"] | null | undefined,
) {
  return {
    transaction: transaction?.id ?? "",
    allowPublicLookup: hasPublicGraphLookupReference(transaction),
  };
}

export function transactionGraphLookupReferenceArgs(
  transactionRef: string | null | undefined,
  allowPublicLookup = false,
) {
  return {
    transaction: transactionRef ?? "",
    allowPublicLookup,
  };
}
