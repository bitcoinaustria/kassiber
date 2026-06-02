import { isLikelyTransactionLookupQuery } from "./ranking";

export type ResolvedTransactionLookup = {
  transaction?: {
    id: string;
    externalId?: string | null;
    explorerId?: string | null;
    account?: string | null;
    type?: string | null;
    counter?: string | null;
    date?: string | null;
  } | null;
  query?: string | null;
};

export type TransactionLookupState =
  | "idle"
  | "looking_up"
  | "matched"
  | "not_found";

export type TransactionLookupLabel = {
  state: TransactionLookupState;
  label: string;
};

export function transactionLookupStateForQuery(
  query: string,
  options: {
    isFetching?: boolean;
    resolved?: ResolvedTransactionLookup | null;
  } = {},
): TransactionLookupState {
  if (!isLikelyTransactionLookupQuery(query)) return "idle";
  if (options.isFetching) return "looking_up";
  if (resolvedMatchesQuery(query, options.resolved)) return "matched";
  return "not_found";
}

export function transactionLookupLabelForState(
  state: TransactionLookupState,
): TransactionLookupLabel {
  switch (state) {
    case "looking_up":
      return { state, label: "Looking up transaction" };
    case "matched":
      return { state, label: "Transaction found" };
    case "not_found":
      return { state, label: "No local transaction match" };
    case "idle":
    default:
      return { state: "idle", label: "Search pages, actions, and local data" };
  }
}

function resolvedMatchesQuery(
  query: string,
  resolved: ResolvedTransactionLookup | null | undefined,
) {
  const transaction = resolved?.transaction;
  if (!transaction) return false;
  const resolvedQuery = resolved?.query?.trim().toLowerCase();
  const normalizedQuery = query.trim().toLowerCase();
  if (resolvedQuery && resolvedQuery !== normalizedQuery) return false;
  return [
    transaction.id,
    transaction.externalId,
    transaction.explorerId,
  ].some((value) => value?.trim().toLowerCase() === normalizedQuery);
}
