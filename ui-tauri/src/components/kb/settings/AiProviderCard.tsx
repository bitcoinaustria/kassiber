/**
 * One provider in Settings → Assistant, after T3 Code's provider instance
 * cards (MIT, Copyright (c) 2026 T3 Tools Inc.): brand mark, name, a one-line
 * status and the enable switch in one row. The row opens to the provider's
 * details and actions, so the list stays a list. It uses the page's controls
 * only: the switch, the settings dropdown and plain outline buttons.
 *
 * The status line only reflects explicit checks (see useAiProviderChecks);
 * rendering this card never contacts the provider.
 */

import * as React from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ProviderGlyph } from "@/components/ai/ProviderGlyph";
import { TOOL_PROFILE_OPTIONS } from "@/components/ai/toolProfile";
import { providerDisplayName } from "@/components/ai/providerIdentity";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  isNativeAiProviderLocator,
  providerEnabled,
  type AiProviderRow,
  type AiSecretStoreId,
} from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";
import { DEFAULT_ASSISTANT_TOOL_PROFILE, useUiStore } from "@/store/ui";
import {
  aiSecretStateLabel,
  aiSecretStoreLabel,
} from "./SettingsModel";
import {
  checkedAgo,
  hintSegments,
  providerApiFormatSummary,
  providerEffortOptions,
  providerOffersFastMode,
  runtimeSummary,
  type ProviderStatusTone,
} from "./aiProviderStatus";
import { SettingsSelect } from "./SettingsControls";
import type { ProviderCheckState } from "./useAiProviderChecks";

const TONE_TEXT: Record<ProviderStatusTone, string> = {
  ready: "text-emerald-600 dark:text-emerald-400",
  attention: "text-amber-600 dark:text-amber-400",
  error: "text-destructive",
  muted: "text-muted-foreground",
};

/** Stands in for "no stored default" in the effort select. */
const AUTO_EFFORT = "auto";

function Hint({ text }: { text: string }) {
  return (
    <>
      {hintSegments(text).map((segment, index) =>
        segment.code ? (
          <code
            key={index}
            className="rounded-sm bg-muted px-1 py-px font-mono text-2xs text-foreground"
          >
            {segment.text}
          </code>
        ) : (
          <React.Fragment key={index}>{segment.text}</React.Fragment>
        ),
      )}
    </>
  );
}

export interface AiProviderCardProps {
  provider: AiProviderRow;
  check: ProviderCheckState;
  refusal?: "offline" | "declined";
  /** Why Check is unavailable right now (AI features off), if it is. The
   * card adds its own reason when the provider is switched off. */
  checkBlockedReason: string | null;
  updating: boolean;
  nativeStoreId: AiSecretStoreId | null;
  nativeAvailable: boolean;
  movingKey: boolean;
  settingDefault: boolean;
  deleting: boolean;
  onCheck: () => void;
  onEnabledChange: (enabled: boolean) => void;
  onDefaultEffortChange: (effort: string | null) => void;
  onSetDefault: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onMoveKey: (storeId: AiSecretStoreId) => void;
}

export function AiProviderCard({
  provider,
  check,
  refusal,
  checkBlockedReason,
  updating,
  nativeStoreId,
  nativeAvailable,
  movingKey,
  settingDefault,
  deleting,
  onCheck,
  onEnabledChange,
  onDefaultEffortChange,
  onSetDefault,
  onEdit,
  onDelete,
  onMoveKey,
}: AiProviderCardProps) {
  const { t, i18n } = useTranslation(["settings", "assistant"]);
  const name = providerDisplayName(provider);
  const isCli = isNativeAiProviderLocator(provider.base_url);
  const enabled = providerEnabled(provider);
  // A switched-off provider is never contacted, so it cannot be checked either.
  const checkUnavailable =
    checkBlockedReason ?? (enabled ? null : t("ai.status.switchedOff"));
  const summary = runtimeSummary(check.runtime);
  const ago = checkedAgo(check.checkedAt, i18n.language);
  const effortOptions = providerEffortOptions(provider, check.models);
  const offersFast = providerOffersFastMode(provider, check.models);
  const apiFormat = providerApiFormatSummary(provider);
  const nameId = `ai-provider-${provider.name}-name`;
  const [open, setOpen] = React.useState(false);
  // Kept in this app's settings by provider name; all tools unless the user
  // picked fewer, whatever the model.
  const toolProfile =
    useUiStore((s) => s.assistantProviderToolProfiles[provider.name]) ??
    DEFAULT_ASSISTANT_TOOL_PROFILE;
  const setToolProfile = useUiStore((s) => s.setAssistantProviderToolProfile);

  const showNativeMove = Boolean(
    nativeStoreId &&
      nativeAvailable &&
      provider.secret_ref?.store_id === "sqlcipher_inline" &&
      provider.has_api_key,
  );
  const showSqlcipherMove = Boolean(
    provider.secret_ref?.store_id &&
      provider.secret_ref.store_id !== "sqlcipher_inline",
  );

  let statusTone: ProviderStatusTone = "muted";
  let statusText: React.ReactNode;
  let statusDetail: string | null = null;
  if (check.pending) {
    statusText = t("ai.status.checking");
  } else if (refusal === "offline") {
    statusTone = "attention";
    statusText = t("ai.status.offline");
  } else if (isCli && check.modelsListed && !check.error) {
    // Only Check models from the picker is known: models, but no runtime
    // check with a version or sign-in state yet.
    statusTone = check.models.length > 0 ? "ready" : "muted";
    statusText = t("ai.status.modelsListed", { count: check.models.length });
  } else if (isCli) {
    statusTone = summary.tone;
    statusText =
      summary.version && summary.headlineKey === "ready"
        ? t("ai.status.readyWithVersion", { version: summary.version })
        : summary.version && summary.headlineKey === "signInNeeded"
          ? t("ai.status.signInNeededWithVersion", { version: summary.version })
          : !check.runtime && check.error
            ? t("ai.status.error")
            : t(`ai.status.${summary.headlineKey}`);
    statusDetail = summary.detail;
  } else if (check.reachable) {
    statusTone = "ready";
    statusText = t("ai.status.reachable", { count: check.models.length });
  } else if (check.error) {
    statusText = t("ai.status.unreachable");
  } else {
    statusText = t("ai.status.notChecked");
  }
  // A failed check after an earlier good one keeps the old result visible.
  const checkError = check.pending || refusal ? null : check.error;
  if (checkError && !check.stale) statusTone = "error";

  const where = isCli
    ? t("ai.where.cli")
    : t(`ai.where.${provider.kind}`);
  const modelLabel = provider.default_model
    ? isCli && provider.default_model === "default"
      ? t("assistant:modelPicker.cliDefault")
      : provider.default_model
    : null;
  const detailId = `${nameId}-details`;

  return (
    <div
      role="group"
      aria-labelledby={nameId}
      data-ai-provider-card={provider.name}
      data-enabled={enabled}
      data-expanded={open}
    >
      <div className="flex items-center gap-3 px-4 py-3">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={detailId}
          aria-label={t("ai.showDetails", { name })}
          onClick={() => setOpen((value) => !value)}
          className="-mx-2 -my-1 flex min-w-0 flex-1 items-center gap-3 rounded-md px-2 py-1 text-left outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ChevronRight
            className={cn(
              "size-4 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-90",
            )}
            aria-hidden="true"
          />
          <ProviderGlyph
            provider={provider}
            className={cn("size-5 shrink-0", !enabled && "opacity-50")}
          />
          <span className={cn("min-w-0 flex-1", !enabled && "opacity-70")}>
            <span id={nameId} className="block truncate text-sm font-medium">
              {name}
            </span>
            <span className="block truncate text-sm text-muted-foreground">
              {[provider.is_default ? t("ai.default") : null, where, modelLabel]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </span>
          <span
            className={cn("max-w-[40%] min-w-0 shrink truncate text-sm", TONE_TEXT[statusTone])}
            aria-live="polite"
            data-provider-status={isCli ? (check.runtime?.state ?? "unknown") : check.reachable ? "reachable" : "unknown"}
          >
            {statusText}
          </span>
        </button>
        <Switch
          checked={enabled}
          // The default provider must stay usable; pick another default first.
          disabled={updating || (provider.is_default && enabled)}
          title={provider.is_default && enabled ? t("ai.defaultCannotDisable") : undefined}
          aria-label={t("ai.enableProvider", { name })}
          onCheckedChange={onEnabledChange}
        />
      </div>

      <div
        id={detailId}
        hidden={!open}
        className="space-y-3 border-t bg-muted/20 px-4 py-3 text-sm sm:pl-19"
      >
        {/* The row shows the status; what the last check found besides it. */}
        {(statusDetail || ago) && !check.pending ? (
          <p className="text-muted-foreground [overflow-wrap:anywhere]">
            {statusDetail ? <Hint text={statusDetail} /> : null}
            {statusDetail && ago ? " · " : null}
            {ago ? t("ai.status.checkedAgo", { ago }) : null}
          </p>
        ) : null}
        {checkError ? (
          <p className="whitespace-pre-line text-destructive [overflow-wrap:anywhere]">
            {check.stale ? `${t("ai.status.stale")} ` : null}
            <Hint text={checkError} />
          </p>
        ) : null}
        {provider.agent_access === "restricted" ? (
          <p data-restricted-access-note className="text-muted-foreground">
            {t("ai.restrictedAccessNote")}
          </p>
        ) : null}
        {!isCli && provider.omit_store_field ? (
          <p data-omit-store-note className="text-amber-700 dark:text-amber-300">
            {t("ai.omitStoreNote")}
          </p>
        ) : null}
        {offersFast ? (
          <p className="text-muted-foreground">{t("ai.fastModeNote")}</p>
        ) : null}

        <dl className="grid gap-x-6 gap-y-2 sm:grid-cols-[9rem_minmax(0,1fr)] sm:items-center">
          {!isCli ? (
            <>
              <dt className="text-muted-foreground">{t("ai.address")}</dt>
              <dd className="min-w-0 [overflow-wrap:anywhere]">
                {provider.base_url}
                {apiFormat ? " " : null}
                {apiFormat ? (
                  <span
                    className="text-muted-foreground"
                    data-api-format={apiFormat.format ?? "auto"}
                    title={t("ai.apiFormatTitle")}
                  >
                    {"· "}
                    {apiFormat.format
                      ? t(`ai.apiFormat.${apiFormat.source}`, {
                          format: t(`aiProvider.apiFormatName.${apiFormat.format}`),
                        })
                      : t("ai.apiFormat.auto")}
                  </span>
                ) : null}
              </dd>
            </>
          ) : null}
          <dt className="text-muted-foreground">{t("ai.defaultModel")}</dt>
          <dd className="break-words">{modelLabel ?? "-"}</dd>
          <dt className="text-muted-foreground" id={`${nameId}-effort`}>
            {t("ai.defaultEffort")}
          </dt>
          <dd>
            <SettingsSelect
              value={provider.default_reasoning_effort ?? AUTO_EFFORT}
              disabled={updating}
              onValueChange={(value) =>
                onDefaultEffortChange(value === AUTO_EFFORT ? null : value)
              }
              options={[
                { value: AUTO_EFFORT, label: t("assistant:composer.effort.auto") },
                ...effortOptions.map((effort) => ({
                  value: effort,
                  label: t(`assistant:composer.effort.${effort}`),
                })),
              ]}
              triggerProps={{ "aria-labelledby": `${nameId}-effort` }}
            />
          </dd>
          <dt className="text-muted-foreground" id={`${nameId}-tools`}>
            {t("ai.toolProfile")}
          </dt>
          <dd>
            <SettingsSelect
              value={toolProfile}
              onValueChange={(value) =>
                setToolProfile(
                  provider.name,
                  value === DEFAULT_ASSISTANT_TOOL_PROFILE ? null : value,
                )
              }
              options={TOOL_PROFILE_OPTIONS.map((option) => ({
                value: option.value,
                label: t(`assistant:${option.labelKey}`),
                title: t(`assistant:${option.descriptionKey}`),
              }))}
              triggerProps={{
                "aria-labelledby": `${nameId}-tools`,
                "data-provider-tool-profile": toolProfile,
              }}
            />
          </dd>
          <dt className="text-muted-foreground">{isCli ? t("ai.signIn") : t("ai.keyStorage")}</dt>
          <dd>
            {isCli
              ? t("ai.signInCli")
              : provider.has_api_key
                ? `${aiSecretStoreLabel(provider.secret_ref?.store_id)} · ${aiSecretStateLabel(provider.secret_ref?.state)}`
                : t("ai.noKey")}
          </dd>
        </dl>

        <div className="flex flex-wrap items-center gap-2 pt-1">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={check.pending || Boolean(checkUnavailable)}
            title={checkUnavailable ?? t("ai.checkHint", { name })}
            aria-label={t("ai.checkProvider", { name })}
            onClick={onCheck}
          >
            {t("ai.check")}
          </Button>
          {!provider.is_default && enabled ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={settingDefault}
              onClick={onSetDefault}
            >
              {t("ai.setAsDefault")}
            </Button>
          ) : null}
          <Button
            type="button"
            size="sm"
            variant="outline"
            aria-label={t("ai.editProvider", { name })}
            onClick={onEdit}
          >
            {t("ai.edit")}
          </Button>
          {showNativeMove && nativeStoreId ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={movingKey}
              onClick={() => onMoveKey(nativeStoreId)}
            >
              {t("ai.moveKeyToNative", { store: aiSecretStoreLabel(nativeStoreId) })}
            </Button>
          ) : null}
          {showSqlcipherMove ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={movingKey || !provider.has_api_key}
              onClick={() => onMoveKey("sqlcipher_inline")}
            >
              {t("ai.moveKeyToDatabase")}
            </Button>
          ) : null}
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="ml-auto hover:text-destructive"
            aria-label={t("ai.deleteProvider", { name })}
            disabled={provider.is_default || deleting}
            onClick={onDelete}
          >
            {t("ai.remove")}
          </Button>
        </div>
      </div>
    </div>
  );
}
