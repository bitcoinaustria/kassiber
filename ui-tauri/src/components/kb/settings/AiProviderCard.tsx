/**
 * One provider in Settings → AI providers, laid out after T3 Code's provider
 * instance cards (MIT, Copyright (c) 2026 T3 Tools Inc.): brand mark, name,
 * version and badges, a one-line status, and the enable switch on the right.
 *
 * The status line only reflects explicit checks (see useAiProviderChecks);
 * rendering this card never contacts the provider.
 */

import * as React from "react";
import {
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
import {
  AI_KIND_BADGE,
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
  const showFooter =
    (!provider.is_default && enabled) || showNativeMove || showSqlcipherMove;

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

  return (
    <div
      role="group"
      aria-labelledby={nameId}
      data-ai-provider-card={provider.name}
      data-enabled={enabled}
      className="rounded-lg border bg-background"
    >
      <div className="flex flex-wrap items-start gap-3 p-4">
        <span
          className={cn(
            "flex size-9 shrink-0 items-center justify-center rounded-md border bg-muted/40",
            !enabled && "opacity-50",
          )}
        >
          <ProviderGlyph provider={provider} className="size-5" />
        </span>
        <div className={cn("min-w-0 flex-1 space-y-1", !enabled && "opacity-70")}>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={nameId} className="font-medium">
              {name}
            </span>
            <span
              className={cn(
                "inline-flex rounded-full border px-2 py-0.5 text-2xs font-medium uppercase tracking-wide",
                AI_KIND_BADGE[provider.kind],
              )}
            >
              {provider.kind === "tee" ? "TEE" : t(`aiProvider.posture.${provider.kind}`)}
            </span>
            <span className="inline-flex rounded-full border px-2 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              {isCli ? t("ai.kindCli") : t("ai.kindHttp")}
            </span>
            {provider.is_default ? (
              <span className="inline-flex items-center rounded-md border border-primary/25 bg-primary/10 px-1.5 py-0.5 text-2xs font-medium uppercase tracking-wide text-primary">
                {t("ai.default")}
              </span>
            ) : null}
            {!enabled ? (
              <span className="inline-flex items-center rounded-md border px-1.5 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                {t("ai.disabledBadge")}
              </span>
            ) : null}
          </div>
          {!isCli ? (
            <p className="flex min-w-0 items-baseline gap-1.5 text-xs text-muted-foreground">
              <span className="truncate font-mono">{provider.base_url}</span>
              {apiFormat ? (
                <span
                  className="shrink-0 text-muted-foreground/80"
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
            </p>
          ) : null}
          {!isCli && provider.omit_store_field ? (
            <p
              data-omit-store-note
              className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300"
            >
              <ShieldAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
              <span>{t("ai.omitStoreNote")}</span>
            </p>
          ) : null}
          <div
            className="flex items-start gap-1.5 text-xs leading-normal text-muted-foreground"
            aria-live="polite"
            data-provider-status={isCli ? (check.runtime?.state ?? "unknown") : check.reachable ? "reachable" : "unknown"}
          >
            <span className="flex h-[1.45em] shrink-0 items-center">
              {check.pending ? (
                <Loader2 className="size-3 animate-spin" aria-hidden="true" />
              ) : (
                <span
                  className={cn("size-1.5 rounded-full", TONE_DOT[statusTone])}
                  aria-hidden="true"
                />
              )}
            </span>
            <span className="min-w-0 [overflow-wrap:anywhere]">
              <span className="text-foreground/90">{statusText}</span>
              {statusDetail && !check.pending ? (
                <>
                  {" · "}
                  <Hint text={statusDetail} />
                </>
              ) : null}
              {ago && !check.pending ? (
                <span className="text-muted-foreground/80">
                  {" · "}
                  {t("ai.status.checkedAgo", { ago })}
                </span>
              ) : null}
            </span>
          </div>
          {checkError ? (
            <p className="whitespace-pre-line text-xs text-destructive [overflow-wrap:anywhere]">
              {check.stale ? `${t("ai.status.stale")} ` : null}
              <Hint text={checkError} />
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-1">
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
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={t("ai.editProvider", { name })}
            onClick={onEdit}
          >
            <Pencil className="size-3.5" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={t("ai.deleteProvider", { name })}
            disabled={provider.is_default || deleting}
            onClick={onDelete}
          >
            <Trash2 className="size-3.5" aria-hidden="true" />
          </Button>
          <Switch
            className="ml-2"
            checked={enabled}
            // The default provider must stay usable; pick another default first.
            disabled={updating || (provider.is_default && enabled)}
            title={provider.is_default && enabled ? t("ai.defaultCannotDisable") : undefined}
            aria-label={t("ai.enableProvider", { name })}
            onCheckedChange={onEnabledChange}
          />
        </div>
      </div>

      <div className="grid gap-x-4 gap-y-3 border-t px-4 py-3 text-xs sm:grid-cols-3">
        <div className="min-w-0 space-y-0.5">
          <p className="text-muted-foreground">{t("ai.defaultModel")}</p>
          <p className="break-words font-mono">
            {provider.default_model
              ? isCli && provider.default_model === "default"
                ? t("assistant:modelPicker.cliDefault")
                : provider.default_model
              : "-"}
          </p>
        </div>
        <div className="min-w-0 space-y-1">
          <p className="text-muted-foreground" id={`${nameId}-effort`}>
            {t("ai.defaultEffort")}
          </p>
          <Select
            value={provider.default_reasoning_effort ?? AUTO_EFFORT}
            disabled={updating}
            onValueChange={(value) =>
              onDefaultEffortChange(value === AUTO_EFFORT ? null : value)
            }
          >
            <SelectTrigger
              size="sm"
              className="w-full max-w-44 text-xs data-[size=sm]:h-7"
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
        </div>
        {isCli ? (
          <div className="min-w-0 space-y-0.5">
            <p className="text-muted-foreground">{t("ai.signIn")}</p>
            <p>{t("ai.signInCli")}</p>
          </div>
        ) : (
          <div className="min-w-0 space-y-0.5">
            <p className="text-muted-foreground">{t("ai.keyStorage")}</p>
            <p>
              {provider.has_api_key ? (
                <>
                  {aiSecretStoreLabel(provider.secret_ref?.store_id)}{" "}
                  <span className="font-mono text-muted-foreground">
                    · {aiSecretStateLabel(provider.secret_ref?.state)}
                  </span>
                </>
              ) : (
                t("ai.noKey")
              )}
            </p>
          </div>
        )}
        {offersFast ? (
          <p className="flex items-start gap-1.5 text-muted-foreground sm:col-span-3">
            <Zap className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            <span>{t("ai.fastModeNote")}</span>
          </p>
        ) : null}
      </div>

      {showFooter ? (
        <div className="flex flex-wrap gap-2 border-t px-4 py-3">
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
        </div>
      ) : null}
    </div>
  );
}
