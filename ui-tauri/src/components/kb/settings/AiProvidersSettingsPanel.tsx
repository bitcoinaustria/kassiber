import * as React from "react";
import { ChevronRight, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { AiProviderForm, type ExistingAiProvider } from "@/components/kb/AiProviderForm";
import { ChatHistorySettings } from "@/components/kb/settings/ChatHistorySettings";
import { ExternalAgentsSettings } from "@/components/kb/settings/ExternalAgentsSettings";
import { ProviderGlyph } from "@/components/ai/ProviderGlyph";
import { providerDisplayName } from "@/components/ai/providerIdentity";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { confirmAction } from "@/lib/confirmAction";
import { cn } from "@/lib/utils";
import {
  isNativeAiProviderLocator,
  providerEnabled,
  type AiProviderRow,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { useOfflineMode } from "@/lib/offlineMode";
import { AiProviderCard } from "./AiProviderCard";
import { AssistantPermissionSettings } from "./AssistantPermissionSettings";
import { SettingsGroup, SettingsRow } from "./SettingsControls";
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
    <div className="space-y-6">
      <SettingsGroup>
        <SettingsRow
          label={t("ai.featuresLabel")}
          description={t("ai.featuresDescription")}
          htmlFor="settings-ai-features"
        >
          <Switch
            id="settings-ai-features"
            checked={aiFeaturesEnabled}
            onCheckedChange={setAiFeaturesEnabled}
            aria-label={t("ai.featuresAria")}
          />
        </SettingsRow>
        <AssistantPermissionSettings />
        <ChatHistorySettings />
      </SettingsGroup>

      <SettingsGroup
        title={<span id="settings-ai-models">{t("ai.modelsHeading")}</span>}
        aria-labelledby="settings-ai-models"
        footer={t("ai.checkPrivacy")}
        actions={
          <>
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
              {t("ai.checkAll")}
            </Button>
            <Button type="button" size="sm" variant="outline" onClick={() => setAddOpen(true)}>
              {t("ai.addProvider")}
            </Button>
          </>
        }
      >
        {policyWarning ? (
          <p className="px-4 py-3 text-sm text-amber-700 dark:text-amber-300">{policyWarning}</p>
        ) : null}
        {updateError ? (
          <p role="alert" className="px-4 py-3 text-sm text-destructive">
            {updateError}
          </p>
        ) : null}
        {providersQuery.isLoading ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">{t("ai.loadingProviders")}</p>
        ) : providersQuery.isError ? (
          <p className="px-4 py-3 text-sm text-destructive">{t("ai.loadError")}</p>
        ) : inUse.length === 0 && notSetUp.length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">{t("ai.noProviders")}</p>
        ) : (
          <>
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
          </>
        )}
      </SettingsGroup>

      <ExternalAgentsSettings aiFeaturesEnabled={aiFeaturesEnabled} />

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
    <section data-not-set-up>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={listId}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-3 px-4 py-3 text-left text-sm text-muted-foreground outline-none hover:bg-muted/40 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <ChevronRight
          className={cn("size-4 shrink-0 transition-transform", open && "rotate-90")}
          aria-hidden="true"
        />
        <span>{t("ai.notSetUp.title", { count: rows.length })}</span>
      </button>
      <ul id={listId} hidden={!open} className="divide-y border-t">
        {rows.map((row) => {
          const name = providerDisplayName(row);
          const pending = pendingFor(row);
          const error = errorFor(row);
          return (
            <li
              key={row.name}
              className="group flex items-center gap-3 py-3 pr-4 pl-11"
              data-provider-row={row.name}
            >
              <ProviderGlyph provider={row} className="size-5 shrink-0" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">{name}</span>
                <span className="block truncate text-sm text-muted-foreground">
                  {isNativeAiProviderLocator(row.base_url)
                    ? t("ai.where.cli")
                    : t(`ai.where.${row.kind}`)}
                </span>
                {error ? (
                  <span role="alert" className="block text-sm text-destructive">
                    {error}
                  </span>
                ) : null}
              </span>
              {/* Hiding a built-in is rare; the button only shows on hover or focus. */}
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                className="text-muted-foreground opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                disabled={deleting || row.is_default}
                aria-label={t("ai.notSetUp.remove", { name })}
                onClick={() => onDelete(row)}
              >
                <Trash2 className="size-4" aria-hidden="true" />
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={Boolean(blockedReason) || pending}
                title={blockedReason ?? undefined}
                aria-label={t("ai.notSetUp.setUpAria", { name })}
                onClick={() => onSetUp(row)}
              >
                {pending ? t("ai.status.checking") : t("ai.notSetUp.setUp")}
              </Button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
