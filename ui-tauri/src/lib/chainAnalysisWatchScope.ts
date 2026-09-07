import { analysisNetworkInput, type AnalysisQuery } from "./chainAnalysis";

export interface WatchBinding {
  state: "unbound" | "bound";
  environment?: string;
  domains: { chain: "bitcoin" | "liquid"; network: string }[];
}

/** Keep generic saved investigations across all bound chains. */
export function watchQueryScope(query: AnalysisQuery | undefined, binding: WatchBinding | undefined): AnalysisQuery | undefined {
  if (!query || binding?.state !== "bound") return query;
  const parts = query.subject?.split(":") ?? [];
  const subjectChain = parts.length >= 4 && ["tx", "out"].includes(parts[2]) && (parts[0] === "bitcoin" || parts[0] === "liquid") ? parts[0] : undefined;
  const chain = query.chain ?? subjectChain;
  if (!chain) return query;
  const network = query.network ?? (subjectChain ? analysisNetworkInput(parts[1]) : undefined) ?? binding.domains.find(domain => domain.chain === chain)?.network;
  return { ...query, chain, ...(network ? { network: analysisNetworkInput(network) } : {}) };
}
