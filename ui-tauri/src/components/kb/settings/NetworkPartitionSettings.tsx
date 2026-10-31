import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { SettingsDisclosure, SettingsRow } from "./SettingsControls";
import { isFileSaveAvailable, saveFile } from "@/lib/filePicker";

type Plan = { profile_id: string; plan_id: string; inventory_digest: string; can_apply: boolean; blockers: { code: string; wallet_id?: string; wallet_ids?: string[] }[]; counts: Record<string, number> };
export function NetworkPartitionSettings({ profileId, inventoryDigest, environment, instance, wallets, declared }: {
  profileId: string; inventoryDigest: string; environment: string; instance: string;
  wallets: { wallet_id: string; label: string }[]; declared: string[];
}) {
  const { t } = useTranslation(["settings", "common"]);
  const [selected, setSelected] = useState<string[]>([]);
  const [destination, setDestination] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [preview, setPreview] = useState<{ key: string; value: Plan } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const plan = useDaemonMutation<Plan>("ui.networks.partition_plan");
  const exportPartition = useDaemonMutation("ui.networks.partition_export");
  const args = { environment, chain_instance_id: environment === "regtest" ? instance : null, wallet_ids: selected, declared_wallet_ids: declared.filter(id => selected.includes(id)) };
  const key = JSON.stringify([profileId, inventoryDigest, args]);
  const current = preview?.key === key && preview.value.profile_id === profileId && preview.value.inventory_digest === inventoryDigest ? preview.value : null;
  const run = async (exportFile: boolean) => {
    setError(null); setSaved(false);
    try {
      if (exportFile) {
        if (!current?.can_apply) return;
        await exportPartition.mutateAsync({ ...args, plan_id: current.plan_id, file_path: destination, backup_passphrase: passphrase });
        setPassphrase(""); setConfirmation(""); setSaved(true);
      } else {
        const result = await plan.mutateAsync(args);
        if (result.data) setPreview({ key, value: result.data });
      }
    } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
  };
  const actions = (children: ReactNode) => <div className="flex flex-wrap justify-end gap-2 px-4 py-3">{children}</div>;
  return <SettingsDisclosure title={t("bookNetwork.partition.title")}>
    <p className="px-4 py-3 text-sm text-muted-foreground">{t("bookNetwork.partition.help")}</p>
    {wallets.map(wallet => <SettingsRow key={wallet.wallet_id} label={wallet.label} htmlFor={`book-network-split-${wallet.wallet_id}`}>
      <Switch id={`book-network-split-${wallet.wallet_id}`} checked={selected.includes(wallet.wallet_id)} onCheckedChange={checked => setSelected(values => checked ? [...values, wallet.wallet_id] : values.filter(id => id !== wallet.wallet_id))} />
    </SettingsRow>)}
    {actions(<Button type="button" variant="outline" size="sm" disabled={!selected.length || plan.isPending} onClick={() => void run(false)}>{t("bookNetwork.preview")}</Button>)}
    {current && <>
      {current.blockers.map((blocker, index) => <p role="alert" className="px-4 py-3 text-sm text-destructive" key={index}>{t(`bookNetwork.partition.blockers.${blocker.code as "relation_crosses_partition" | "incompatible_wallet_history" | "unresolved_relation" | "scope_declaration_required" | "legacy_ledger_present"}`)} {(blocker.wallet_ids ?? [blocker.wallet_id]).map(id => wallets.find(wallet => wallet.wallet_id === id)?.label).filter(Boolean).join(", ")}</p>)}
      {current.can_apply && <>
        <p className="px-4 py-3 text-sm">{t("bookNetwork.partition.counts", { wallets: current.counts.wallets ?? 0, transactions: current.counts.transactions ?? 0 })}</p>
        <SettingsRow label={t("bookNetwork.partition.passphrase")} htmlFor="book-network-split-passphrase">
          <Input id="book-network-split-passphrase" className="h-8 w-48" type="password" autoComplete="new-password" value={passphrase} onChange={event => setPassphrase(event.target.value)} />
        </SettingsRow>
        <SettingsRow label={t("bookNetwork.partition.confirmPassphrase")} htmlFor="book-network-split-confirm">
          <Input id="book-network-split-confirm" className="h-8 w-48" type="password" autoComplete="new-password" value={confirmation} onChange={event => setConfirmation(event.target.value)} />
        </SettingsRow>
        <SettingsRow label={t("bookNetwork.partition.destination")} htmlFor="book-network-split-destination">
          <Input id="book-network-split-destination" className="h-8 w-72 font-mono text-xs" value={destination} onChange={event => setDestination(event.target.value)} />
          {isFileSaveAvailable && <Button type="button" variant="outline" size="sm" onClick={async () => { const path = await saveFile({ defaultPath: "network-partition.kassiber" }); if (path) setDestination(path); }}>{t("common:actions.save")}</Button>}
        </SettingsRow>
        {actions(<Button type="button" variant="outline" size="sm" disabled={passphrase.length < 12 || passphrase !== confirmation || !destination || exportPartition.isPending} onClick={() => void run(true)}>{t("bookNetwork.partition.export")}</Button>)}
      </>}
    </>}
    {error && <p role="alert" className="px-4 py-3 text-sm text-destructive">{error}</p>}
    {saved && <p role="status" className="px-4 py-3 text-sm">{t("bookNetwork.partition.saved")}</p>}
  </SettingsDisclosure>;
}
