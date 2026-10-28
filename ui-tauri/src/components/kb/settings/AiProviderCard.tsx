/**
 * One provider in Settings → Assistant, after T3 Code's provider instance
 * cards (MIT, Copyright (c) 2026 T3 Tools Inc.): brand mark, name, a one-line
 * status and the enable switch in one row. The row opens to the provider's
 * details and actions, so the list stays a list.
 *
 * The status line only reflects explicit checks (see useAiProviderChecks);
 * rendering this card never contacts the provider.
 */

import * as React from "react";
import {
  ChevronRight,
  Database,
  Loader2,
  Pencil,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  Zap,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { ProviderGlyph } from "@/components/ai/ProviderGlyph";
import { TOOL_PROFILE_OPTIONS } from "@/components/ai/toolProfile";
import { providerDisplayName } from "@/components/ai/providerIdentity";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  isNativeAiProviderLocator,
  providerEnabled,
  type AiProviderRow,
  type AiSecretStoreId,
} from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";
import { isAiToolProfile } from "@/daemon/stream";
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
import type { ProviderCheckState } from "./useAiProviderChecks";

const TONE_DOT: Record<ProviderStatusTone, string> = {
  ready: "bg-emerald-500",
  attention: "bg-amber-500",
  error: "bg-destructive",
  muted: "bg-muted-foreground/40",
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
      <div className="flex items-center gap-3 px-4 py-2.5">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={detailId}
          aria-label={t("ai.showDetails", { name })}
          onClick={() => setOpen((value) => !value)}
          className="-mx-2 flex min-w-0 flex-1 items-center gap-3 rounded-md px-2 py-1 text-left outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span
            className={cn(
              "flex size-8 shrink-0 items-center justify-center rounded-md border bg-muted/40",
              !enabled && "opacity-50",
            )}
          >
            <ProviderGlyph provider={provider} className="size-4" />
          </span>
          <span className={cn("min-w-0 flex-1", !enabled && "opacity-70")}>
            <span className="flex items-center gap-2">
              <span id={nameId} className="truncate text-sm font-medium">
                {name}
              </span>
              {provider.is_default ? (
                <span className="shrink-0 rounded-md bg-primary/10 px-1.5 py-px text-2xs font-medium text-primary">
                  {t("ai.default")}
                </span>
              ) : null}
            </span>
            <span className="block truncate text-xs text-muted-foreground">
              {[where, modelLabel].filter(Boolean).join(" · ")}
            </span>
          </span>
          <span
            className="flex max-w-[45%] min-w-0 shrink items-center gap-1.5 text-xs text-muted-foreground"
            aria-live="polite"
            data-provider-status={isCli ? (check.runtime?.state ?? "unknown") : check.reachable ? "reachable" : "unknown"}
          >
            {check.pending ? (
              <Loader2 className="size-3 shrink-0 animate-spin" aria-hidden="true" />
            ) : (
              <span
                className={cn("size-1.5 shrink-0 rounded-full", TONE_DOT[statusTone])}
                aria-hidden="true"
              />
            )}
            <span className="truncate">{statusText}</span>
          </span>
          <ChevronRight
            className={cn(
              "size-4 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-90",
            )}
            aria-hidden="true"
          />
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

      <div id={detailId} hidden={!open} className="space-y-3 border-t bg-muted/20 px-4 py-3 text-xs">
        {/* The row shows the status; what the last check found besides it. */}
        {(statusDetail || ago) && !check.pending ? (
          <p className="leading-normal text-muted-foreground [overflow-wrap:anywhere]">
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
          <p data-restricted-access-note className="flex items-start gap-1.5 text-muted-foreground">
            <ShieldCheck className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            <span>{t("ai.restrictedAccessNote")}</span>
          </p>
        ) : null}
        {!isCli && provider.omit_store_field ? (
          <p data-omit-store-note className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300">
            <ShieldAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            <span>{t("ai.omitStoreNote")}</span>
          </p>
        ) : null}

        <dl className="grid gap-x-4 gap-y-2 sm:grid-cols-[8rem_minmax(0,1fr)] sm:items-center">
          {!isCli ? (
            <>
              <dt className="text-muted-foreground">{t("ai.address")}</dt>
              <dd className="flex min-w-0 items-baseline gap-1.5">
                <span className="truncate font-mono">{provider.base_url}</span>
                {apiFormat ? (
                  <span
                    className="shrink-0 text-muted-foreground"
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
          <dd className="break-words font-mono">{modelLabel ?? "-"}</dd>
          <dt className="text-muted-foreground" id={`${nameId}-effort`}>
            {t("ai.defaultEffort")}
          </dt>
          <dd>
            <Select
              value={provider.default_reasoning_effort ?? AUTO_EFFORT}
              disabled={updating}
              onValueChange={(value) =>
                onDefaultEffortChange(value === AUTO_EFFORT ? null : value)
              }
            >
              <SelectTrigger
                size="sm"
                className="w-44 text-xs data-[size=sm]:h-7"
                aria-labelledby={`${nameId}-effort`}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={AUTO_EFFORT}>
                  {t("assistant:composer.effort.auto")}
                </SelectItem>
                {effortOptions.map((effort) => (
                  <SelectItem key={effort} value={effort}>
                    {t(`assistant:composer.effort.${effort}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </dd>
          <dt className="text-muted-foreground" id={`${nameId}-tools`}>
            {t("ai.toolProfile")}
          </dt>
          <dd>
            <Select
              value={toolProfile}
              onValueChange={(value) => {
                if (!isAiToolProfile(value)) return;
                setToolProfile(
                  provider.name,
                  value === DEFAULT_ASSISTANT_TOOL_PROFILE ? null : value,
                );
              }}
            >
              <SelectTrigger
                size="sm"
                className="w-44 text-xs data-[size=sm]:h-7"
                aria-labelledby={`${nameId}-tools`}
                data-provider-tool-profile={toolProfile}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TOOL_PROFILE_OPTIONS.map((option) => (
                  <SelectItem
                    key={option.value}
                    value={option.value}
                    title={t(`assistant:${option.descriptionKey}`)}
                  >
                    {t(`assistant:${option.labelKey}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </dd>
          <dt className="text-muted-foreground">{isCli ? t("ai.signIn") : t("ai.keyStorage")}</dt>
          <dd>
            {isCli ? (
              t("ai.signInCli")
            ) : provider.has_api_key ? (
              <>
                {aiSecretStoreLabel(provider.secret_ref?.store_id)}{" "}
                <span className="font-mono text-muted-foreground">
                  · {aiSecretStateLabel(provider.secret_ref?.state)}
                </span>
              </>
            ) : (
              t("ai.noKey")
            )}
          </dd>
        </dl>
        {offersFast ? (
          <p className="flex items-start gap-1.5 text-muted-foreground">
            <Zap className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            <span>{t("ai.fastModeNote")}</span>
          </p>
        ) : null}

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
            <RefreshCw
              className={cn("size-3.5", check.pending && "animate-spin")}
              aria-hidden="true"
            />
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
            variant="ghost"
            aria-label={t("ai.editProvider", { name })}
            onClick={onEdit}
          >
            <Pencil className="size-3.5" aria-hidden="true" />
            {t("ai.edit")}
          </Button>
          {showNativeMove && nativeStoreId ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={movingKey}
              onClick={() => onMoveKey(nativeStoreId)}
            >
              <ShieldCheck className="size-4" aria-hidden="true" />
              {t("ai.moveKeyToNative", { store: aiSecretStoreLabel(nativeStoreId) })}
            </Button>
          ) : null}
          {showSqlcipherMove ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={movingKey || !provider.has_api_key}
              onClick={() => onMoveKey("sqlcipher_inline")}
            >
              <Database className="size-4" aria-hidden="true" />
              {t("ai.moveKeyToDatabase")}
            </Button>
          ) : null}
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="ml-auto text-muted-foreground hover:text-destructive"
            aria-label={t("ai.deleteProvider", { name })}
            disabled={provider.is_default || deleting}
            onClick={onDelete}
          >
            <Trash2 className="size-3.5" aria-hidden="true" />
            {t("ai.remove")}
          </Button>
        </div>
      </div>
    </div>
  );
}
