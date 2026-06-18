import * as React from "react";
import { Database, Pencil, Plus, ShieldCheck, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { AiProviderForm, type ExistingAiProvider } from "@/components/kb/AiProviderForm";
import { ChatHistorySettingsCard } from "@/components/kb/settings/ChatHistorySettingsCard";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import type { AiModelsListData } from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";
import {
  AI_KIND_BADGE,
  aiSecretStateLabel,
  aiSecretStoreLabel,
  formatModelSummary,
  isCliAiProvider,
  type AiProviderRow,
  type AiProvidersListData,
} from "./SettingsModel";

function AiProviderModelSummary({ row }: { row: AiProviderRow }) {
  const { t } = useTranslation("settings");
  const isCli = isCliAiProvider(row);
  const modelsQuery = useDaemon<AiModelsListData>(
    "ai.list_models",
    { provider: row.name },
    {
      enabled: isCli,
      refetchOnMount: "always",
      staleTime: 5 * 60 * 1000,
    },
  );

  if (!isCli) return <>{row.default_model ?? "-"}</>;

  const models =
    modelsQuery.data?.kind === "ai.list_models" && modelsQuery.data.data
      ? modelsQuery.data.data.models
      : [];
  const summary = formatModelSummary(models);
  if (summary) return <>{summary}</>;
  if (modelsQuery.isFetching) {
    return (
      <span className="text-muted-foreground">{t("ai.loadingModels")}</span>
    );
  }
  return <>{row.default_model ?? "-"}</>;
}

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
  const [editingName, setEditingName] = React.useState<string | null>(null);
  const [addOpen, setAddOpen] = React.useState(false);
  const nativeStoreId = data.secret_store_policy?.default?.native_store_id ?? null;
  const nativeAvailable = data.secret_store_policy?.default?.native_available === true;
  const policyWarning = data.secret_store_policy?.default?.warning;

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
    };
  }, [data.providers, editingName]);

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
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

      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h3 className="text-sm font-semibold">
            {t("ai.providerConfigHeading")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("ai.providerConfigDescription")}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          className="shrink-0"
          onClick={() => setAddOpen(true)}
        >
          <Plus className="size-4" aria-hidden="true" />
          {t("ai.addProvider")}
        </Button>
      </div>

      {policyWarning ? (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
          {policyWarning}
        </div>
      ) : null}

      {providersQuery.isLoading ? (
        <div className="rounded-md border bg-background p-4 text-sm text-muted-foreground">
          {t("ai.loadingProviders")}
        </div>
      ) : providersQuery.isError ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
          {t("ai.loadError")}
        </div>
      ) : data.providers.length === 0 ? (
        <div className="rounded-md border border-dashed bg-muted/20 p-6 text-center text-sm text-muted-foreground">
          {t("ai.noProviders")}
        </div>
      ) : (
        <div className="grid gap-3">
          {data.providers.map((row) => {
            const showNativeMove = Boolean(
              nativeStoreId &&
                nativeAvailable &&
                row.secret_ref?.store_id === "sqlcipher_inline" &&
                row.has_api_key,
            );
            const showSqlcipherMove = Boolean(
              row.secret_ref?.store_id &&
                row.secret_ref.store_id !== "sqlcipher_inline",
            );
            const showFooter =
              !row.is_default || showNativeMove || showSqlcipherMove;
            return (
              <div
                key={row.name}
                className="space-y-3 rounded-md border bg-background p-4"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{row.name}</span>
                      <span
                        className={cn(
                          "inline-flex rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide",
                          AI_KIND_BADGE[row.kind],
                        )}
                      >
                        {row.kind === "tee"
                          ? "TEE"
                          : t(`aiProvider.posture.${row.kind}`)}
                      </span>
                      {row.is_default ? (
                        <span className="inline-flex items-center rounded-md border border-primary/25 bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary">
                          {t("ai.default")}
                        </span>
                      ) : null}
                    </div>
                    <p className="truncate font-mono text-xs text-muted-foreground">
                      {row.base_url}
                    </p>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button
                      type="button"
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("ai.editProvider", { name: row.name })}
                      onClick={() => setEditingName(row.name)}
                    >
                      <Pencil className="size-3.5" aria-hidden="true" />
                    </Button>
                    <Button
                      type="button"
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("ai.deleteProvider", { name: row.name })}
                      disabled={row.is_default || deleteProvider.isPending}
                      onClick={() => {
                        const ok = window.confirm(
                          t("ai.deleteConfirm", { name: row.name }),
                        );
                        if (!ok) return;
                        deleteProvider.mutate({ name: row.name });
                      }}
                    >
                      <Trash2 className="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                </div>

                <div className="grid gap-x-4 gap-y-2 text-xs sm:grid-cols-3">
                  <div className="min-w-0 space-y-0.5">
                    <p className="text-muted-foreground">
                      {t("ai.defaultModel")}
                    </p>
                    <p className="break-words font-mono">
                      <AiProviderModelSummary row={row} />
                    </p>
                  </div>
                  <div className="space-y-0.5">
                    <p className="text-muted-foreground">{t("ai.auth")}</p>
                    <p>{row.has_api_key ? t("ai.authBearer") : t("ai.authNone")}</p>
                  </div>
                  <div className="min-w-0 space-y-0.5">
                    <p className="text-muted-foreground">{t("ai.keyStorage")}</p>
                    <p>
                      {aiSecretStoreLabel(row.secret_ref?.store_id)}{" "}
                      <span className="font-mono text-muted-foreground">
                        · {aiSecretStateLabel(row.secret_ref?.state)}
                      </span>
                    </p>
                  </div>
                </div>

                {showFooter ? (
                  <div className="flex flex-wrap gap-2 border-t pt-3">
                    {!row.is_default ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={setDefault.isPending}
                        onClick={() => setDefault.mutate({ name: row.name })}
                      >
                        {t("ai.setAsDefault")}
                      </Button>
                    ) : null}
                    {nativeStoreId &&
                    nativeAvailable &&
                    row.secret_ref?.store_id === "sqlcipher_inline" &&
                    row.has_api_key ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        disabled={moveProviderKey.isPending}
                        onClick={() =>
                          moveProviderKey.mutate({
                            name: row.name,
                            store_id: nativeStoreId,
                          })
                        }
                      >
                        <ShieldCheck className="size-4" aria-hidden="true" />
                        {t("ai.moveKeyToNative", {
                          store: aiSecretStoreLabel(nativeStoreId),
                        })}
                      </Button>
                    ) : null}
                    {row.secret_ref?.store_id &&
                    row.secret_ref.store_id !== "sqlcipher_inline" ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        disabled={moveProviderKey.isPending || !row.has_api_key}
                        onClick={() =>
                          moveProviderKey.mutate({
                            name: row.name,
                            store_id: "sqlcipher_inline",
                          })
                        }
                      >
                        <Database className="size-4" aria-hidden="true" />
                        {t("ai.moveKeyToDatabase")}
                      </Button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
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
