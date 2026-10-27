import * as React from "react";
import { ChevronRight, Loader2, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { AiProviderForm, type ExistingAiProvider } from "@/components/kb/AiProviderForm";
import { ChatHistorySettingsCard } from "@/components/kb/settings/ChatHistorySettingsCard";
import { ExternalAgentsSettings } from "@/components/kb/settings/ExternalAgentsSettings";
import { ProviderGlyph } from "@/components/ai/ProviderGlyph";
import { providerDisplayName } from "@/components/ai/providerIdentity";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { confirmAction } from "@/lib/confirmAction";
import { cn } from "@/lib/utils";
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
    // A check is what moves a built-in provider out of "Not set up".
    onChecked: refetchProviders,
  });
  const checkBlockedReason = aiFeaturesEnabled ? null : t("ai.status.aiOff");
  const enabledProviders = data.providers.filter(providerEnabled);
  const inUse = data.providers.filter((row) => row.set_up !== false);
  const notSetUp = data.providers.filter((row) => row.set_up === false);
  const confirmDelete = async (row: AiProviderRow) => {
    const ok = await confirmAction(t("ai.deleteConfirm", { name: providerDisplayName(row) }));
    if (ok) deleteProvider.mutate({ name: row.name });
  };
  // Setting one up is checking it; a switched-off one is switched on first,
  // since a switched-off provider is never contacted.
  const setUp = async (row: AiProviderRow) => {
    if (!providerEnabled(row)) {
      setUpdatingName(row.name);
      setUpdateError(null);
      try {
        await updateProvider.mutateAsync({ name: row.name, enabled: true });
      } catch (error) {
        setUpdateError(error instanceof Error ? error.message : String(error));
        return;
      } finally {
        setUpdatingName(null);
      }
    }
    await checks.check({ ...row, enabled: true });
  };

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
      api_format: row.api_format,
      detected_api_format: row.detected_api_format ?? null,
      omit_store_field: row.omit_store_field ?? false,
      agent_access: row.agent_access ?? "full",
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
      ) : inUse.length === 0 && notSetUp.length === 0 ? (
        <div className="rounded-lg border border-dashed bg-muted/20 p-6 text-center text-sm text-muted-foreground">
          {t("ai.noProviders")}
        </div>
      ) : (
        <div className="grid gap-3">
          {inUse.map((row) => (
            <AiProviderCard
              key={row.name}
              provider={row}
              check={checks.stateFor(row)}
              refusal={checks.refusalFor(row)}
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
              onDelete={() => void confirmDelete(row)}
              onMoveKey={(storeId) =>
                moveProviderKey.mutate({ name: row.name, store_id: storeId })
              }
            />
          ))}
          {notSetUp.length > 0 ? (
            <NotSetUpProviders
              rows={notSetUp}
              pendingFor={(row) => checks.stateFor(row).pending || updatingName === row.name}
              errorFor={(row) => checks.stateFor(row).error}
              blockedReason={checkBlockedReason}
              deleting={deleteProvider.isPending}
              onSetUp={(row) => void setUp(row)}
              onDelete={(row) => void confirmDelete(row)}
            />
          ) : null}
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

/**
 * Built-in providers the user has not used yet, folded into one quiet group
 * so the page shows what is in use. Nothing here contacts a provider until
 * **Set up**, which runs that provider's check (asking for the off-device
 * acknowledgement first, as any check does).
 */
function NotSetUpProviders({
  rows,
  pendingFor,
  errorFor,
  blockedReason,
  deleting,
  onSetUp,
  onDelete,
}: {
  rows: AiProviderRow[];
  pendingFor: (row: AiProviderRow) => boolean;
  errorFor: (row: AiProviderRow) => string | null;
  blockedReason: string | null;
  deleting: boolean;
  onSetUp: (row: AiProviderRow) => void;
  onDelete: (row: AiProviderRow) => void;
}) {
  const { t } = useTranslation("settings");
  const [open, setOpen] = React.useState(false);
  const listId = React.useId();
  return (
    <section className="rounded-lg border bg-background" data-not-set-up>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={listId}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 rounded-lg px-4 py-3 text-left text-sm outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronRight
          className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
          aria-hidden="true"
        />
        <span className="font-medium">{t("ai.notSetUp.title")}</span>
        <span className="tabular-nums text-muted-foreground">{rows.length}</span>
      </button>
      <div id={listId} hidden={!open} className="border-t px-4 pt-2 pb-3">
        <p className="pb-2 text-xs text-muted-foreground">{t("ai.notSetUp.hint")}</p>
        <ul className="grid gap-1">
          {rows.map((row) => {
            const name = providerDisplayName(row);
            const pending = pendingFor(row);
            const error = errorFor(row);
            return (
              <li key={row.name} className="grid gap-1 py-1" data-provider-row={row.name}>
                <div className="flex items-center gap-3">
                  <ProviderGlyph provider={row} className="size-5" />
                  <span className="min-w-0 flex-1 truncate text-sm">{name}</span>
                  <span className="shrink-0 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                    {t(`aiProvider.posture.${row.kind}`)}
                  </span>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={Boolean(blockedReason) || pending}
                    title={blockedReason ?? undefined}
                    aria-label={t("ai.notSetUp.setUpAria", { name })}
                    onClick={() => onSetUp(row)}
                  >
                    {pending ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
                    {t("ai.notSetUp.setUp")}
                  </Button>
                  <Button
                    type="button"
                    size="icon-sm"
                    variant="ghost"
                    disabled={deleting || row.is_default}
                    aria-label={t("ai.notSetUp.remove", { name })}
                    onClick={() => onDelete(row)}
                  >
                    <Trash2 className="size-4" aria-hidden="true" />
                  </Button>
                </div>
                {error ? (
                  <p role="alert" className="pl-8 text-xs text-destructive">
                    {error}
                  </p>
                ) : null}
              </li>
            );
          })}
        </ul>
      </div>
    </section>
  );
}
