import { NetworkPartitionSettings } from "./NetworkPartitionSettings";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { SettingsGroup, SettingsRow, SettingsSelect } from "./SettingsControls";

type WalletScope = { wallet_id: string; label: string; environments: string[]; transaction_count: number; unknown_count: number; conflict_count: number; requires_declaration: boolean };
type Binding = { profile_id: string; state: "unbound" | "bound"; environment?: string; chain_instance_id?: string | null; domains: { chain: string; network: string }[] };
type Inventory = { profile_id: string; state: string; wallets: WalletScope[]; inventory_digest: string; binding: Binding };
type Plan = { plan_id: string; profile_id: string; inventory_digest: string; environment: string; chain_instance_id: string | null; declared_wallet_ids: string[]; can_apply: boolean; blockers: { code: string; wallet_id?: string }[] };
const ENVIRONMENTS = ["main", "test", "signet", "regtest"] as const;
type Environment = (typeof ENVIRONMENTS)[number];

export function BookNetworkSettings({ children }: { children?: ReactNode } = {}) {
  const { t } = useTranslation(["settings", "common"]);
  const bindingQuery = useDaemon<Binding>("ui.networks.binding");
  const binding = bindingQuery.data?.data;
  const needsInventory = binding?.state === "unbound" && !bindingQuery.isError;
  const query = useDaemon<Inventory>("ui.networks.inventory", {}, { enabled: needsInventory });
  const preview = useDaemonMutation<Plan>("ui.networks.plan");
  const bind = useDaemonMutation("ui.networks.bind");
  const [environment, setEnvironment] = useState("main");
  const [instance, setInstance] = useState("");
  const [declared, setDeclared] = useState<string[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inventory = needsInventory && query.data?.data?.binding.state === "unbound" && query.data.data.profile_id === binding?.profile_id
    ? query.data.data : undefined;
  const recipe = { environment, chain_instance_id: environment === "regtest" ? instance : null, declared_wallet_ids: declared };
  const current = plan && inventory && plan.profile_id === inventory.profile_id && plan.inventory_digest === inventory.inventory_digest && JSON.stringify({ environment: plan.environment, chain_instance_id: plan.chain_instance_id, declared_wallet_ids: [...plan.declared_wallet_ids].sort() }) === JSON.stringify({ ...recipe, declared_wallet_ids: [...declared].sort() });
  const bound = binding?.state === "bound";
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
  const retryRow = (retry: () => void) => (
    <SettingsRow label={t("bookNetwork.loadError")}>
      <Button type="button" variant="outline" size="sm" onClick={retry}>{t("common:actions.retry")}</Button>
    </SettingsRow>
  );
  return <div className="space-y-3">
    <SettingsGroup title={t("bookNetwork.title")} footer={bound ? t("bookNetwork.immutable") : inventory ? t("bookNetwork.help") : null}>
      {(bindingQuery.isLoading || needsInventory && query.isLoading) && <p className="px-4 py-3 text-sm text-muted-foreground">{t("common:state.loading")}</p>}
      {bindingQuery.isError && retryRow(() => void bindingQuery.refetch())}
      {needsInventory && query.isError && retryRow(() => void query.refetch())}
      {bound ? <SettingsRow
        label={t("bookNetwork.environment")}
        description={<>
          <span className="block">{binding.domains.map(domain => `${domain.chain}: ${domain.network}`).join(" · ")}</span>
          {binding.chain_instance_id && <span className="block break-all font-mono text-xs">{binding.chain_instance_id}</span>}
        </>}
      >
        <span className="text-sm">{t(`bookNetwork.environments.${binding.environment as Environment}`)}</span>
      </SettingsRow> : inventory && <>
        <SettingsRow label={t("bookNetwork.environment")} htmlFor="book-network-environment">
          <SettingsSelect<Environment>
            value={environment as Environment}
            onValueChange={setEnvironment}
            options={ENVIRONMENTS.map(value => ({ value, label: t(`bookNetwork.environments.${value}`) }))}
            triggerProps={{ id: "book-network-environment" }}
          />
        </SettingsRow>
        {environment === "regtest" && <SettingsRow label={t("bookNetwork.instance")} htmlFor="book-network-instance">
          <Input id="book-network-instance" className="h-8 w-72 font-mono text-xs" value={instance} onChange={event => setInstance(event.target.value)} />
          <Button type="button" variant="outline" size="sm" onClick={() => setInstance(crypto.randomUUID())}>{t("bookNetwork.newInstance")}</Button>
        </SettingsRow>}
        {inventory.wallets.map(wallet => <SettingsRow
          key={wallet.wallet_id}
          label={wallet.label}
          htmlFor={wallet.requires_declaration ? `book-network-declare-${wallet.wallet_id}` : undefined}
          description={<>
            <span className="block">{wallet.environments.join(", ") || t("bookNetwork.unknown")}</span>
            {wallet.requires_declaration && <span className="block">{t("bookNetwork.declare")}</span>}
          </>}
        >
          {wallet.requires_declaration && <Switch id={`book-network-declare-${wallet.wallet_id}`} checked={declared.includes(wallet.wallet_id)} onCheckedChange={checked => setDeclared(values => checked ? [...values, wallet.wallet_id] : values.filter(id => id !== wallet.wallet_id))} />}
        </SettingsRow>)}
        {current && plan && plan.blockers.map((blocker, index) => <p className="px-4 py-3 text-sm text-destructive" key={index}>{t(`bookNetwork.blockers.${blocker.code as "already_bound" | "contradictory_evidence" | "different_environment" | "different_instance" | "scope_declaration_required" | "reference_domain_mismatch"}`)} {inventory.wallets.find(wallet => wallet.wallet_id === blocker.wallet_id)?.label}</p>)}
        {current && plan?.can_apply && <p className="px-4 py-3 text-sm text-amber-700 dark:text-amber-300">{t("bookNetwork.confirmHelp")}</p>}
        <div className="flex flex-wrap justify-end gap-2 px-4 py-3">
          <Button type="button" variant="outline" size="sm" disabled={preview.isPending || bind.isPending} onClick={() => void invoke(false)}>{t("bookNetwork.preview")}</Button>
          {current && plan?.can_apply && <Button type="button" variant="outline" size="sm" disabled={bind.isPending} onClick={() => void invoke(true)}>{t("bookNetwork.bind")}</Button>}
        </div>
      </>}
      {error && <p role="alert" className="px-4 py-3 text-sm text-destructive">{error}</p>}
    </SettingsGroup>
    {inventory && !bound && <NetworkPartitionSettings key={inventory.profile_id} profileId={inventory.profile_id} inventoryDigest={inventory.inventory_digest} environment={environment} instance={instance} wallets={inventory.wallets} declared={declared} />}
    {inventory && !bound && children}
  </div>;
}
