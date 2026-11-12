import { ANALYSIS_NETWORKS, type AnalysisNetwork } from "@/store/ui";

/**
 * The one environment an unbound book's connections already show, or mainnet
 * when they show none or disagree. A suggestion for display and defaults only:
 * it never binds the book or counts as source evidence.
 */
export function bookEvidenceNetwork(
  wallets: { environments: string[] }[] | undefined,
): AnalysisNetwork {
  const seen = new Set((wallets ?? []).flatMap((wallet) => wallet.environments));
  const [only] = seen;
  return seen.size === 1 && ANALYSIS_NETWORKS.includes(only as AnalysisNetwork)
    ? (only as AnalysisNetwork)
    : "main";
}
