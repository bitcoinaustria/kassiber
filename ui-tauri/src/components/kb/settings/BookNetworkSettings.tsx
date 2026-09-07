import { NetworkPartitionSettings } from "./NetworkPartitionSettings";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";

type WalletScope = { wallet_id: string; label: string; environments: string[]; transaction_count: number; unknown_count: number; conflict_count: number; requires_declaration: boolean };
type Inventory = { profile_id: string; state: string; wallets: WalletScope[]; inventory_digest: string; binding: { state: string; environment?: string; chain_instance_id?: string | null; domains: { chain: string; network: string }[] } };
type Plan = { plan_id: string; profile_id: string; inventory_digest: string; environment: string; chain_instance_id: string | null; declared_wallet_ids: string[]; can_apply: boolean; blockers: { code: string; wallet_id?: string }[] };

export function BookNetworkSettings({ children }: { children?: ReactNode } = {}) {
  const { t } = useTranslation(["settings", "common"]);
  const query = useDaemon<Inventory>("ui.networks.inventory");
  const preview = useDaemonMutation<Plan>("ui.networks.plan");
  const bind = useDaemonMutation("ui.networks.bind");
  const [environment, setEnvironment] = useState("main");
  const [instance, setInstance] = useState("");
  const [declared, setDeclared] = useState<string[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inventory = query.data?.data;
  const recipe = { environment, chain_instance_id: environment === "regtest" ? instance : null, declared_wallet_ids: declared };
  const current = plan && inventory && plan.profile_id === inventory.profile_id && plan.inventory_digest === inventory.inventory_digest && JSON.stringify({ environment: plan.environment, chain_instance_id: plan.chain_instance_id, declared_wallet_ids: [...plan.declared_wallet_ids].sort() }) === JSON.stringify({ ...recipe, declared_wallet_ids: [...declared].sort() });
  const bound = inventory?.binding.state === "bound";
  const invoke = async (apply: boolean) => {
    setError(null);
    try {
      if (apply && current && plan?.can_apply) {
        await bind.mutateAsync({ ...recipe, plan_id: plan.plan_id });
        setPlan(null);
      } else if (!apply) {
        const result = await preview.mutateAsync(recipe);
        setPlan(result.data ?? null);
      }
    } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
  };
  return <section className="space-y-3 rounded-md border bg-background p-4">
    <h3 className="text-sm font-medium">{t("bookNetwork.title")}</h3>
    {query.isLoading && <p>{t("common:state.loading")}</p>}
    {query.isError && <Button variant="outline" onClick={() => void query.refetch()}>{t("common:actions.retry")}</Button>}
    {inventory && (bound ? <>
      <p>{t(`bookNetwork.environments.${inventory.binding.environment as "main" | "test" | "signet" | "regtest"}`)}</p>
      <p className="text-xs text-muted-foreground">{inventory.binding.domains.map(domain => `${domain.chain}: ${domain.network}`).join(" · ")}</p>
      {inventory.binding.chain_instance_id && <p className="break-all font-mono text-xs">{inventory.binding.chain_instance_id}</p>}
      <p className="text-xs text-muted-foreground">{t("bookNetwork.immutable")}</p>
    </> : <>
      <p className="text-xs text-muted-foreground">{t("bookNetwork.help")}</p>
      <label className="flex max-w-sm flex-col gap-1 text-sm">{t("bookNetwork.environment")}
        <select className="rounded border bg-background p-2" value={environment} onChange={event => setEnvironment(event.target.value)}>
          {(["main", "test", "signet", "regtest"] as const).map(value => <option key={value} value={value}>{t(`bookNetwork.environments.${value}`)}</option>)}
        </select>
      </label>
      {environment === "regtest" && <label className="flex flex-col gap-1 text-sm">{t("bookNetwork.instance")}
        <div className="flex gap-2"><input className="min-w-0 flex-1 rounded border bg-background p-2 font-mono text-xs" value={instance} onChange={event => setInstance(event.target.value)} /><Button variant="outline" onClick={() => setInstance(crypto.randomUUID())}>{t("bookNetwork.newInstance")}</Button></div>
      </label>}
      {inventory.wallets.map(wallet => <div key={wallet.wallet_id} className="text-sm">
        <span>{wallet.label}</span><span className="ml-2 text-xs text-muted-foreground">{wallet.environments.join(", ") || t("bookNetwork.unknown")}</span>
        {wallet.requires_declaration && <label className="mt-1 flex items-start gap-2 text-xs"><input type="checkbox" checked={declared.includes(wallet.wallet_id)} onChange={event => setDeclared(values => event.target.checked ? [...values, wallet.wallet_id] : values.filter(id => id !== wallet.wallet_id))} />{t("bookNetwork.declare")}</label>}
      </div>)}
      <Button variant="outline" disabled={preview.isPending || bind.isPending} onClick={() => void invoke(false)}>{t("bookNetwork.preview")}</Button>
      {current && plan && <div className="space-y-2">
        {plan.blockers.map((blocker, index) => <p className="text-sm text-destructive" key={index}>{t(`bookNetwork.blockers.${blocker.code as "already_bound" | "contradictory_evidence" | "different_environment" | "different_instance" | "scope_declaration_required" | "reference_domain_mismatch"}`)} {inventory.wallets.find(wallet => wallet.wallet_id === blocker.wallet_id)?.label}</p>)}
        {plan.can_apply && <><p className="text-xs text-muted-foreground">{t("bookNetwork.confirmHelp")}</p><Button disabled={bind.isPending} onClick={() => void invoke(true)}>{t("bookNetwork.bind")}</Button></>}
      </div>}
    </>)}
    {inventory && !bound && <NetworkPartitionSettings key={inventory.profile_id} profileId={inventory.profile_id} inventoryDigest={inventory.inventory_digest} environment={environment} instance={instance} wallets={inventory.wallets} declared={declared} />}
    {inventory && !bound && children}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </section>;
}
