import { kindKey } from "./linkModel";

/** What a rule matches and the terms its links record. */
export interface RuleTerms {
  predicate: Record<string, unknown>;
  kind: string;
  policy: string;
}

/** The translator, loosely: rule labels come from several namespaces' keys. */
export type Translate = (key: string, options?: Record<string, unknown>) => string;

/**
 * A rule as one sentence: what it matches, then what the link records, e.g.
 * "Liquid → on-chain, exact matches only: record as Peg-out from Liquid, tax-neutral".
 */
export function ruleSentence(rule: RuleTerms, t: Translate): string {
  const p = rule.predicate;
  const side = (asset: unknown, walletKind: unknown) => {
    const parts: string[] = [];
    if (typeof walletKind === "string" && walletKind) parts.push(t(`swap.createRule.walletKind.${walletKind}`, { defaultValue: walletKind }));
    if (asset === "LBTC") parts.push(t("swap.rules.sentence.liquid"));
    else if (asset === "BTC") parts.push(t("swap.rules.sentence.bitcoin"));
    return parts.join(" ") || t("swap.rules.sentence.anyWallet");
  };
  const matches = [`${side(p.out_asset, p.out_wallet_kind)} → ${side(p.in_asset, p.in_wallet_kind)}`];
  matches.push(p.min_confidence === "exact" ? t("swap.rules.sentence.exactOnly") : t("swap.rules.sentence.likelyOrExact"));
  if (typeof p.max_fee_pct === "number") matches.push(t("swap.rules.sentence.maxFee", { pct: p.max_fee_pct }));
  return t("swap.rules.sentence.full", {
    matches: matches.join(", "),
    kind: t(`transfersPage.kind.${kindKey(rule.kind)}`),
    policy: t(`transfersPage.policyShort.${rule.policy === "taxable" ? "taxable" : "carrying"}`),
  });
}
