import * as React from "react";
import { Plus, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { AiProviderForm, type ExistingAiProvider } from "@/components/kb/AiProviderForm";
import { ChatHistorySettingsCard } from "@/components/kb/settings/ChatHistorySettingsCard";
import { ExternalAgentsSettings } from "@/components/kb/settings/ExternalAgentsSettings";
import { providerDisplayName } from "@/components/ai/providerIdentity";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { confirmAction } from "@/lib/confirmAction";
import {
  providerEnabled,
  type AiProviderRow,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { useOfflineMode } from "@/lib/offlineMode";
import { AiProviderCard } from "./AiProviderCard";
import { useAiProviderChecks } from "./useAiProviderChecks";

export function AiProvidersSettingsPanel({
  aiFeaturesEnabled,
  setAiFeaturesEnabled,
}: {
  aiFeaturesEnabled: boolean;
  setAiFeaturesEnabled: (enabled: boolean) => void;
}) {
  const { t } = useTranslation("settings");
  const providersQuery = useDaemon<AiProvidersListData>("ai.providers.list");
  const data = React.useMemo<AiProvidersListData>(
    () =>
      providersQuery.data?.kind === "ai.providers.list" &&
      providersQuery.data.data
        ? providersQuery.data.data
        : { providers: [], default: null },
    [providersQuery.data],
  );
  const setDefault = useDaemonMutation("ai.providers.set_default");
  const deleteProvider = useDaemonMutation("ai.providers.delete");
  const moveProviderKey = useDaemonMutation("ai.providers.move_api_key");
  const updateProvider = useDaemonMutation("ai.providers.update");
  const [updatingName, setUpdatingName] = React.useState<string | null>(null);
  const [updateError, setUpdateError] = React.useState<string | null>(null);
  const [editingName, setEditingName] = React.useState<string | null>(null);
  const [addOpen, setAddOpen] = React.useState(false);
  const nativeStoreId = data.secret_store_policy?.default?.native_store_id ?? null;
  const nativeAvailable = data.secret_store_policy?.default?.native_available === true;
  const policyWarning = data.secret_store_policy?.default?.warning;
  const { offline } = useOfflineMode();
  const { refetch: refetchProviders } = providersQuery;

  const confirmChecks = React.useCallback(
    (rows: AiProviderRow[]) =>
      confirmAction(
        t("ai.checkConfirm", {
          count: rows.length,
          names: rows.map(providerDisplayName).join(", "),
        }),
      ),
    [t],
  );
  const checks = useAiProviderChecks({
    providers: data.providers,
    enabled: aiFeaturesEnabled,
    offline,
    confirm: confirmChecks,
    onAcknowledged: refetchProviders,
  });
  const checkBlockedReason = aiFeaturesEnabled ? null : t("ai.status.aiOff");
  const enabledProviders = data.providers.filter(providerEnabled);

  const editingProvider = React.useMemo<ExistingAiProvider | null>(() => {
    if (!editingName) return null;
    const row = data.providers.find((provider) => provider.name === editingName);
    if (!row) return null;
    return {
      name: row.name,
      base_url: row.base_url,
      default_model: row.default_model ?? undefined,
      kind: row.kind,
      notes: row.notes ?? undefined,
      has_api_key: row.has_api_key,
      secret_ref: row.secret_ref,
      acknowledged_at: row.acknowledged_at ?? null,
      enabled: providerEnabled(row),
    };
  }, [data.providers, editingName]);

  const update = (name: string, args: Record<string, unknown>) => {
    setUpdatingName(name);
    setUpdateError(null);
    updateProvider
      .mutateAsync({ name, ...args })
      .catch((error: unknown) =>
        setUpdateError(error instanceof Error ? error.message : String(error)),
      )
      .finally(() => setUpdatingName(null));
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 rounded-lg border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0 space-y-1">
          <Label htmlFor="settings-ai-features">{t("ai.featuresLabel")}</Label>
          <p className="text-sm text-muted-foreground">
            {t("ai.featuresDescription")}
          </p>
        </div>
        <Switch
          id="settings-ai-features"
          checked={aiFeaturesEnabled}
          onCheckedChange={setAiFeaturesEnabled}
          aria-label={t("ai.featuresAria")}
          className="shrink-0"
        />
      </div>

      <ExternalAgentsSettings aiFeaturesEnabled={aiFeaturesEnabled} />

      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h3 className="text-sm font-semibold">
            {t("ai.providerConfigHeading")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("ai.providerConfigDescription")}
          </p>
          <p className="text-xs text-muted-foreground">{t("ai.checkPrivacy")}</p>
        </div>
        <div className="flex shrink-0 gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={
              Boolean(checkBlockedReason) ||
              checks.anyPending ||
              enabledProviders.length === 0
            }
            title={checkBlockedReason ?? t("ai.checkAllHint")}
            onClick={() => void checks.checkAll(enabledProviders)}
          >
            <RefreshCw
              className={checks.anyPending ? "size-4 animate-spin" : "size-4"}
              aria-hidden="true"
            />
            {t("ai.checkAll")}
          </Button>
          <Button type="button" size="sm" onClick={() => setAddOpen(true)}>
            <Plus className="size-4" aria-hidden="true" />
            {t("ai.addProvider")}
          </Button>
        </div>
      </div>

      {policyWarning ? (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
          {policyWarning}
        </div>
      ) : null}

      {updateError ? (
        <div
          role="alert"
          className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
        >
          {updateError}
        </div>
      ) : null}

      {providersQuery.isLoading ? (
        <div className="rounded-lg border bg-background p-4 text-sm text-muted-foreground">
          {t("ai.loadingProviders")}
        </div>
      ) : providersQuery.isError ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
          {t("ai.loadError")}
        </div>
      ) : data.providers.length === 0 ? (
        <div className="rounded-lg border border-dashed bg-muted/20 p-6 text-center text-sm text-muted-foreground">
          {t("ai.noProviders")}
        </div>
      ) : (
        <div className="grid gap-3">
          {data.providers.map((row) => (
            <AiProviderCard
              key={row.name}
              provider={row}
              check={checks.stateFor(row)}
              refusal={checks.refusals[row.name]}
              checkBlockedReason={checkBlockedReason}
              updating={updatingName === row.name}
              nativeStoreId={nativeStoreId}
              nativeAvailable={nativeAvailable}
              movingKey={moveProviderKey.isPending}
              settingDefault={setDefault.isPending}
              deleting={deleteProvider.isPending}
              onCheck={() => void checks.check(row)}
              onEnabledChange={(enabled) => update(row.name, { enabled })}
              onDefaultEffortChange={(effort) =>
                update(
                  row.name,
                  effort
                    ? { default_reasoning_effort: effort }
                    : { clear: ["default_reasoning_effort"] },
                )
              }
              onSetDefault={() => setDefault.mutate({ name: row.name })}
              onEdit={() => setEditingName(row.name)}
              onDelete={() => {
                void (async () => {
                  const ok = await confirmAction(
                    t("ai.deleteConfirm", { name: providerDisplayName(row) }),
                  );
                  if (!ok) return;
                  deleteProvider.mutate({ name: row.name });
                })();
              }}
              onMoveKey={(storeId) =>
                moveProviderKey.mutate({ name: row.name, store_id: storeId })
              }
            />
          ))}
        </div>
      )}

      <ChatHistorySettingsCard />

      <AiProviderForm
        open={addOpen}
        initial={null}
        onClose={() => setAddOpen(false)}
        onSaved={() => setAddOpen(false)}
      />
      <AiProviderForm
        open={Boolean(editingProvider)}
        initial={editingProvider}
        onClose={() => setEditingName(null)}
        onSaved={() => setEditingName(null)}
      />
    </div>
  );
}
