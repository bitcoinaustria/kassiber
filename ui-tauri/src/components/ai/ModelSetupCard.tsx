/**
 * The empty chat's first step when no model is chosen: the coding agents
 * (Claude, Codex, Copilot, OpenCode) as rows with **Use**. Nothing is
 * contacted until the user picks one; that runs the same check Settings →
 * Assistant runs, asking first for an off-device one. A ready agent becomes
 * the default provider and this chat's model. One that is not ready says why
 * (not installed, sign-in needed) in its row. Other providers stay in
 * Settings.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";

import { ProviderGlyph } from "@/components/ai/ProviderGlyph";
import { providerDisplayName } from "@/components/ai/providerIdentity";
import {
  hintSegments,
  runtimeSummary,
} from "@/components/kb/settings/aiProviderStatus";
import { openSettingsSection } from "@/components/kb/settings/settingsModal";
import { useAiProviderChecks } from "@/components/kb/settings/useAiProviderChecks";
import { Button } from "@/components/ui/button";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  isNativeAiProviderLocator,
  providerEnabled,
  type AiProviderRow,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { confirmAction } from "@/lib/confirmAction";
import { useOfflineMode } from "@/lib/offlineMode";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

function Hint({ text }: { text: string }) {
  return (
    <>
      {hintSegments(text).map((segment, index) =>
        segment.code ? (
          <code key={index} className="rounded-sm bg-muted px-1 py-px font-mono text-2xs text-foreground">
            {segment.text}
          </code>
        ) : (
          <React.Fragment key={index}>{segment.text}</React.Fragment>
        ),
      )}
    </>
  );
}

export function ModelSetupCard({ className }: { className?: string }) {
  const { t } = useTranslation("assistant");
  const { t: tSettings } = useTranslation("settings");
  const aiFeaturesEnabled = useUiStore((state) => state.aiFeaturesEnabled);
  const setSelection = useUiStore((state) => state.setAssistantModelSelection);
  const providersQuery = useDaemon<AiProvidersListData>("ai.providers.list");
  const { refetch } = providersQuery;
  const agents = React.useMemo(
    () =>
      (providersQuery.data?.kind === "ai.providers.list"
        ? (providersQuery.data.data?.providers ?? [])
        : []
      ).filter((row) => isNativeAiProviderLocator(row.base_url)),
    [providersQuery.data],
  );
  const { offline } = useOfflineMode();
  const confirmChecks = React.useCallback(
    (rows: AiProviderRow[]) =>
      confirmAction(
        tSettings("ai.checkConfirm", {
          count: rows.length,
          names: rows.map(providerDisplayName).join(", "),
        }),
      ),
    [tSettings],
  );
  const checks = useAiProviderChecks({
    providers: agents,
    enabled: aiFeaturesEnabled,
    offline,
    confirm: confirmChecks,
    onAcknowledged: refetch,
    onChecked: refetch,
  });
  const update = useDaemonMutation("ai.providers.update");
  const setDefault = useDaemonMutation("ai.providers.set_default");
  const [starting, setStarting] = React.useState<string | null>(null);
  // Set once a chosen agent's check has finished; its result decides.
  const [checked, setChecked] = React.useState<string | null>(null);
  const [failure, setFailure] = React.useState<{ name: string; message: string } | null>(null);

  const use = async (row: AiProviderRow) => {
    setStarting(row.name);
    setFailure(null);
    try {
      // A switched-off provider is never contacted, so it is switched on first.
      if (!providerEnabled(row)) await update.mutateAsync({ name: row.name, enabled: true });
      await checks.check({ ...row, enabled: true });
      setChecked(row.name);
    } catch (error) {
      setFailure({ name: row.name, message: error instanceof Error ? error.message : String(error) });
    } finally {
      setStarting(null);
    }
  };

  React.useEffect(() => {
    if (!checked) return;
    const row = agents.find((candidate) => candidate.name === checked);
    if (!row) return;
    const state = checks.stateFor(row);
    if (state.pending) return;
    setChecked(null);
    if (runtimeSummary(state.runtime).tone !== "ready") return;
    // The row now says the agent works; make it the one chats start with.
    setDefault.mutate({ name: row.name });
    setSelection({ provider: row.name, model: row.default_model || "default" });
  }, [agents, checked, checks, setDefault, setSelection]);

  if (!aiFeaturesEnabled || agents.length === 0) return null;

  return (
    <section
      className={cn("w-full overflow-hidden rounded-lg border bg-background text-left", className)}
      aria-labelledby="assistant-model-setup-title"
      data-model-setup=""
    >
      <div className="space-y-0.5 px-4 py-3">
        <h3 id="assistant-model-setup-title" className="text-sm font-medium text-foreground">
          {t("modelSetup.title")}
        </h3>
        <p className="text-sm text-muted-foreground">{t("modelSetup.body")}</p>
      </div>
      <ul className="divide-y border-t">
        {agents.map((row) => {
          const name = providerDisplayName(row);
          const state = checks.stateFor(row);
          const refusal = checks.refusalFor(row);
          const summary = runtimeSummary(state.runtime);
          const pending = state.pending || starting === row.name;
          const error =
            failure?.name === row.name ? failure.message : state.pending ? null : state.error;
          const status = pending
            ? tSettings("ai.status.checking")
            : refusal === "offline"
              ? tSettings("ai.status.offline")
              : state.runtime && summary.tone !== "ready"
                ? tSettings(`ai.status.${summary.headlineKey}`)
                : null;
          return (
            <li key={row.name} className="flex items-center gap-3 px-4 py-3" data-model-setup-row={row.name}>
              <ProviderGlyph provider={row} className="size-5 shrink-0" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-foreground">{name}</span>
                {status || summary.detail || error ? (
                  <span
                    className={cn(
                      "block text-sm [overflow-wrap:anywhere]",
                      error || summary.tone === "error" ? "text-destructive" : "text-muted-foreground",
                    )}
                    aria-live="polite"
                  >
                    {status}
                    {status && (summary.detail || error) ? " · " : null}
                    {error ? <Hint text={error} /> : summary.detail && !pending ? <Hint text={summary.detail} /> : null}
                  </span>
                ) : null}
              </span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={pending || Boolean(starting)}
                aria-label={t("modelSetup.useAria", { name })}
                onClick={() => void use(row)}
              >
                {state.runtime && summary.tone !== "ready" ? t("modelSetup.retry") : t("modelSetup.use")}
              </Button>
            </li>
          );
        })}
      </ul>
      <div className="flex items-center justify-between gap-3 border-t px-4 py-3">
        <p className="text-sm text-muted-foreground">{t("modelSetup.other")}</p>
        <Button type="button" size="sm" variant="outline" onClick={() => openSettingsSection("assistant-ai")}>
          {t("modelSetup.openSettings")}
        </Button>
      </div>
    </section>
  );
}
