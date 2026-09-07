import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";
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
  return <details className="space-y-3 rounded border p-3">
    <summary className="cursor-pointer text-sm font-medium">{t("bookNetwork.partition.title")}</summary>
    <p className="text-xs text-muted-foreground">{t("bookNetwork.partition.help")}</p>
    {wallets.map(wallet => <label className="flex gap-2 text-sm" key={wallet.wallet_id}><input type="checkbox" checked={selected.includes(wallet.wallet_id)} onChange={event => setSelected(values => event.target.checked ? [...values, wallet.wallet_id] : values.filter(id => id !== wallet.wallet_id))} />{wallet.label}</label>)}
    <Button variant="outline" disabled={!selected.length || plan.isPending} onClick={() => void run(false)}>{t("bookNetwork.preview")}</Button>
    {current && <>
      {current.blockers.map((blocker, index) => <p role="alert" className="text-sm text-destructive" key={index}>{t(`bookNetwork.partition.blockers.${blocker.code as "relation_crosses_partition" | "incompatible_wallet_history" | "unresolved_relation" | "scope_declaration_required" | "accounting_partition_unsupported"}`)} {(blocker.wallet_ids ?? [blocker.wallet_id]).map(id => wallets.find(wallet => wallet.wallet_id === id)?.label).filter(Boolean).join(", ")}</p>)}
      {current.can_apply && <>
        <p className="text-xs">{t("bookNetwork.partition.counts", { wallets: current.counts.wallets ?? 0, transactions: current.counts.transactions ?? 0 })}</p>
        <label className="flex flex-col gap-1 text-sm">{t("bookNetwork.partition.passphrase")}<input type="password" autoComplete="new-password" className="rounded border bg-background p-2 text-xs" value={passphrase} onChange={event => setPassphrase(event.target.value)} /></label>
        <label className="flex flex-col gap-1 text-sm">{t("bookNetwork.partition.confirmPassphrase")}<input type="password" autoComplete="new-password" className="rounded border bg-background p-2 text-xs" value={confirmation} onChange={event => setConfirmation(event.target.value)} /></label>
        <label className="flex flex-col gap-1 text-sm">{t("bookNetwork.partition.destination")}<input className="rounded border bg-background p-2 text-xs" value={destination} onChange={event => setDestination(event.target.value)} /></label>
        {isFileSaveAvailable && <Button variant="outline" onClick={async () => { const path = await saveFile({ defaultPath: "network-partition.kassiber" }); if (path) setDestination(path); }}>{t("common:actions.save")}</Button>}
        <Button disabled={passphrase.length < 12 || passphrase !== confirmation || !destination || exportPartition.isPending} onClick={() => void run(true)}>{t("bookNetwork.partition.export")}</Button>
      </>}
    </>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {saved && <p role="status" className="text-sm">{t("bookNetwork.partition.saved")}</p>}
  </details>;
}
