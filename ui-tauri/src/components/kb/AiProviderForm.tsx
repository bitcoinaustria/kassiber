/**
 * Add/edit form for AI provider records.
 *
 * Used inline in the Settings modal. The form holds local state and
 * surfaces validation errors from the daemon. The "Test connection"
 * button issues `ai.test_connection` against the entered credentials and
 * reports the result without persisting.
 */

import {
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  Info,
  Loader2,
  ShieldAlert,
  Sparkles,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  aiProviderTestOutcome,
  aiTestConnectionArgs,
  type AiProviderTestOutcome,
  type AiTestConnectionData,
} from "@/components/kb/aiProviderTest";
import {
  AI_PROVIDER_PRESETS,
  autoRemoteReason,
  presetNeedsApiKey,
  type AiProviderPreset,
  type AutoRemoteReason,
} from "@/components/kb/aiProviderFormModel";
import { useDaemonMutation } from "@/daemon/client";
import { getTransport } from "@/daemon/transport";
import {
  isLoopbackAiHttpUrl,
  isNativeAiProviderLocator,
  type AiApiFormat,
  type AiWireApiFormat,
} from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";

export interface AiProviderInput {
  name: string;
  display_name?: string | null;
  base_url: string;
  api_key?: string;
  default_model?: string;
  kind: "local" | "remote" | "tee";
  notes?: string;
}

export interface ExistingAiProvider extends AiProviderInput {
  has_api_key: boolean;
  secret_ref?: {
    store_id: "macos_keychain" | "windows_dpapi" | "linux_secret_service" | "sqlcipher_inline";
    state: "ok" | "missing" | "needs_reauth" | "unavailable";
  };
  acknowledged_at?: string | null;
  /** false when switched off in Settings: such a provider is never contacted. */
  enabled?: boolean;
  /** The saved wire-format setting; "auto" when absent. */
  api_format?: AiApiFormat;
  /** What the saved URL was last seen to answer. */
  detected_api_format?: AiWireApiFormat | null;
}

interface AiProviderFormProps {
  open: boolean;
  initial?: ExistingAiProvider | null;
  onClose: () => void;
  onSaved?: (name: string) => void;
}

const PROVIDER_KIND_HINT_KEYS = {
  local: "aiProvider.kindHint.local",
  remote: "aiProvider.kindHint.remote",
  tee: "aiProvider.kindHint.tee",
} as const satisfies Record<AiProviderInput["kind"], string>;

const AUTO_REMOTE_NOTE_KEYS = {
  url: "aiProvider.autoRemote.url",
  cli: "aiProvider.autoRemote.cli",
} as const satisfies Record<AutoRemoteReason, string>;

const API_FORMATS: readonly AiApiFormat[] = ["auto", "responses", "chat_completions"];

export function AiProviderForm({
  open,
  initial,
  onClose,
  onSaved,
}: AiProviderFormProps) {
  const { t } = useTranslation(["settings", "common"]);
  const [name, setName] = React.useState(initial?.display_name ?? initial?.name ?? "");
  const [baseUrl, setBaseUrl] = React.useState(initial?.base_url ?? "");
  const [apiKey, setApiKey] = React.useState(initial?.api_key ?? "");
  const [defaultModel, setDefaultModel] = React.useState(initial?.default_model ?? "");
  const [kind, setKind] = React.useState<AiProviderInput["kind"]>(initial?.kind ?? "local");
  const [notes, setNotes] = React.useState(initial?.notes ?? "");
  const [remoteAcknowledged, setRemoteAcknowledged] = React.useState(false);
  // Set when the form moved the posture to remote on its own, to say why.
  const [autoRemote, setAutoRemote] = React.useState<AutoRemoteReason | null>(null);
  const [preset, setPreset] = React.useState<AiProviderPreset | null>(null);
  const [apiFormat, setApiFormat] = React.useState<AiApiFormat>(initial?.api_format ?? "auto");
  // Open when a format other than auto was saved, so the setting is seen.
  const [advancedOpen, setAdvancedOpen] = React.useState(
    Boolean(initial?.api_format && initial.api_format !== "auto"),
  );
  const [error, setError] = React.useState<string | null>(null);
  // Each run remembers the values it tested (`target`). A result for values
  // the form no longer holds is never shown or saved: a format detected at
  // one URL must not be stored with another.
  const [testRun, setTestRun] = React.useState<
    | { state: "idle" }
    | { state: "running"; target: string }
    | (AiProviderTestOutcome & { target: string })
    | { state: "fail"; target: string; message: string }
  >({ state: "idle" });

  const editing = Boolean(initial);

  React.useEffect(() => {
    if (!open) return;
    setName(initial?.display_name ?? initial?.name ?? "");
    setBaseUrl(initial?.base_url ?? "");
    setApiKey("");
    setDefaultModel(initial?.default_model ?? "");
    setKind(initial?.kind ?? "local");
    setNotes(initial?.notes ?? "");
    setRemoteAcknowledged(false);
    setAutoRemote(null);
    setPreset(null);
    setApiFormat(initial?.api_format ?? "auto");
    setAdvancedOpen(Boolean(initial?.api_format && initial.api_format !== "auto"));
    setError(null);
    setTestRun({ state: "idle" });
  }, [open, initial]);

  const applyPreset = (next: AiProviderPreset) => {
    setPreset(next);
    setName(next.display_name);
    setBaseUrl(next.base_url);
    setKind(next.kind);
    setAutoRemote(null);
    setDefaultModel(next.default_model);
    setNotes(
      next.custom
        ? ""
        : next.kind === "local"
          ? t("aiProvider.presetNoteLocal", { label: next.label })
          : t("aiProvider.presetNoteRemote", { label: next.label }),
    );
    setRemoteAcknowledged(false);
  };

  /**
   * Move a local provider to remote when `url` cannot be local, as the daemon
   * would, and return the posture that applies. A fresh acknowledgement is
   * needed after the move; the user's own remote or TEE choice is kept.
   */
  const applyUrlPosture = (url: string): AiProviderInput["kind"] => {
    const reason = autoRemoteReason(kind, url);
    if (!reason) return kind;
    setKind("remote");
    setAutoRemote(reason);
    setRemoteAcknowledged(false);
    return "remote";
  };

  const createProvider = useDaemonMutation("ai.providers.create");
  const updateProvider = useDaemonMutation("ai.providers.update");
  const setApiKeyMutation = useDaemonMutation("ai.providers.set_api_key");
  const testFingerprint = React.useMemo(
    () =>
      JSON.stringify({
        provider: editing ? (initial?.name ?? null) : null,
        baseUrl: baseUrl.trim(),
        apiKey: apiKey.trim(),
        // The test asks this model, so a different one needs a new test.
        model: defaultModel.trim(),
        // A detection found under one format setting is not the other's.
        apiFormat,
      }),
    [apiFormat, apiKey, baseUrl, defaultModel, editing, initial?.name],
  );
  // CLI agents speak their own protocol and take no API format.
  const isCliUrl = isNativeAiProviderLocator(baseUrl);
  const formatToSave = isCliUrl ? null : apiFormat;
  // OpenAI and OpenRouter take no request without a key. Only a new provider
  // is held to it: a saved one keeps its stored key when the field is blank.
  const keyRequired = presetNeedsApiKey(preset, baseUrl);
  const testStatus =
    testRun.state !== "idle" && testRun.target === testFingerprint
      ? testRun
      : ({ state: "idle" } as const);

  const handleTest = async () => {
    const target = testFingerprint;
    // Only the run for these values may report; a later edit or a newer run
    // makes this one's answer moot.
    const settle = (next: Exclude<typeof testRun, { state: "idle" | "running" }>) =>
      setTestRun((current) =>
        current.state === "running" && current.target === target ? next : current,
      );
    setTestRun({ state: "running", target });
    try {
      const trimmedUrl = baseUrl.trim();
      if (!trimmedUrl) {
        throw new Error(t("aiProvider.errorBaseUrlRequired"));
      }
      if (!/^https?:\/\//.test(trimmedUrl) && !isNativeAiProviderLocator(trimmedUrl)) {
        throw new Error(t("aiProvider.errorUrlScheme"));
      }
      // Offline mode lets only local providers through, so an address off
      // this device is tested as the remote provider it will be saved as.
      const testKind = applyUrlPosture(trimmedUrl);
      // Empty API-key field means "keep current key": naming the saved
      // provider lets the daemon use the stored value so the test exercises
      // real credentials. The default model field is the model the test asks.
      const args = aiTestConnectionArgs({
        baseUrl: trimmedUrl,
        kind: testKind,
        apiKey,
        model: defaultModel,
        provider: editing && initial ? initial.name : null,
        apiFormat: formatToSave,
      });
      const envelope = await getTransport().invoke<AiTestConnectionData>({
        kind: "ai.test_connection",
        args,
      });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? t("aiProvider.errorTestFailed"));
      }
      // A listing without a reply (no_model) is shown as unchecked, never
      // as success, and saves no detection.
      settle({ ...aiProviderTestOutcome(envelope.data), target });
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      settle({ state: "fail", target, message });
    }
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      if (applyUrlPosture(baseUrl) !== kind) {
        // Moved to remote just now: the acknowledgement is still to be given.
        throw new Error(t("aiProvider.errorAckRequired"));
      }
      if (!editing && keyRequired && !apiKey.trim()) {
        throw new Error(t("aiProvider.errorApiKeyRequired", { label: preset?.label ?? "" }));
      }
      const needsRemoteAck =
        kind !== "local" && (!initial || initial.kind === "local" || !initial.acknowledged_at);
      if (kind === "local" && isNativeAiProviderLocator(baseUrl)) {
        throw new Error(t("aiProvider.errorCliLocalPosture"));
      }
      if (needsRemoteAck) {
        if (!remoteAcknowledged) {
          throw new Error(t("aiProvider.errorAckRequired"));
        }
      }
      if (editing && initial) {
        const args: Record<string, unknown> = {
          name: initial.name,
          display_name: name.trim(),
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || null,
          kind,
          notes: notes.trim() || null,
        };
        if (formatToSave) {
          args.api_format = formatToSave;
        }
        if (needsRemoteAck) {
          args.acknowledged = true;
        }
        if (testStatus.state === "ok" && testStatus.detectedApiFormat) {
          // The test of these exact values found the format; a retargeted
          // provider would otherwise forget it with its old URL.
          args.detected_api_format = testStatus.detectedApiFormat;
        }
        await updateProvider.mutateAsync(args);
        if (apiKey.trim()) {
          await setApiKeyMutation.mutateAsync({
            name: initial.name,
            api_key: apiKey.trim(),
          });
        }
        onSaved?.(initial.name);
      } else {
        const args: Record<string, unknown> = {
          name: name.trim(),
          display_name: name.trim(),
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || undefined,
          kind,
          notes: notes.trim() || undefined,
        };
        if (formatToSave) {
          args.api_format = formatToSave;
        }
        if (needsRemoteAck) {
          args.acknowledged = true;
        }
        if (testStatus.state === "ok" && testStatus.detectedApiFormat) {
          args.detected_api_format = testStatus.detectedApiFormat;
        }
        await createProvider.mutateAsync(args);
        if (apiKey.trim()) {
          await setApiKeyMutation.mutateAsync({
            name: name.trim().toLowerCase(),
            api_key: apiKey.trim(),
          });
        }
        onSaved?.(name.trim().toLowerCase());
      }
      onClose();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  };

  const isBusy = createProvider.isPending || updateProvider.isPending || setApiKeyMutation.isPending;
  // The daemon refuses to test a switched-off provider; say so up front.
  const testUnavailable =
    editing && initial?.enabled === false ? t("aiProvider.errorTestDisabled") : null;
  const needsRemoteAck =
    kind !== "local" && (!initial || initial.kind === "local" || !initial.acknowledged_at);
  const canSubmit = !isBusy && (!needsRemoteAck || remoteAcknowledged);
  // Said while the reason still holds; editing the URL back to this device
  // hides it, though the posture stays remote until the user picks local.
  const autoRemoteShown =
    autoRemote !== null &&
    kind === "remote" &&
    (autoRemote === "cli"
      ? isNativeAiProviderLocator(baseUrl)
      : /^https?:\/\//.test(baseUrl.trim()) && !isLoopbackAiHttpUrl(baseUrl));
  // A detection belongs to the URL it was made at.
  const savedDetection =
    editing && initial?.detected_api_format && baseUrl.trim() === initial.base_url.trim()
      ? initial.detected_api_format
      : null;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="size-4 text-primary" aria-hidden="true" />
            {editing ? t("aiProvider.editTitle") : t("aiProvider.addTitle")}
          </DialogTitle>
          <DialogDescription>
            {t("aiProvider.dialogDescription")}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4">
          {!editing ? (
            <div className="grid gap-2">
              <Label id="ai-form-presets">{t("aiProvider.presetLabel")}</Label>
              <div
                role="group"
                aria-labelledby="ai-form-presets"
                className="flex flex-wrap gap-2"
              >
                {AI_PROVIDER_PRESETS.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    aria-pressed={preset?.id === option.id}
                    data-provider-preset={option.id}
                    onClick={() => applyPreset(option)}
                    className={cn(
                      "rounded-md border px-3 py-2 text-xs font-medium transition-colors",
                      preset?.id === option.id
                        ? "border-primary bg-primary/10 text-foreground"
                        : "border-border bg-background text-muted-foreground hover:bg-muted",
                    )}
                  >
                    {option.custom ? t("aiProvider.presetCustom") : option.label}
                  </button>
                ))}
              </div>
              {preset ? (
                <p className="text-xs text-muted-foreground" data-preset-hint>
                  {t(preset.hintKey, { label: preset.label })}
                </p>
              ) : null}
            </div>
          ) : null}
          <div className="grid gap-2">
            <Label htmlFor="ai-form-name">{t("aiProvider.nameLabel")}</Label>
            <Input
              id="ai-form-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="oMLX"
              required
              autoFocus={!editing}
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-url">{t("aiProvider.urlLabel")}</Label>
            <Input
              id="ai-form-url"
              value={baseUrl}
              onChange={(event) => setBaseUrl(event.target.value)}
              onBlur={() => applyUrlPosture(baseUrl)}
              placeholder="http://127.0.0.1:8000/v1 or claude-cli://default"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-kind">{t("aiProvider.postureLabel")}</Label>
            <div className="grid grid-cols-3 gap-2">
              {(["local", "remote", "tee"] as const).map((option) => (
                <button
                  key={option}
                  type="button"
                  aria-pressed={kind === option}
                  onClick={() => {
                    setKind(option);
                    setAutoRemote(null);
                  }}
                  className={cn(
                    "rounded-md border px-3 py-2 text-xs font-medium uppercase tracking-wide transition-colors",
                    kind === option
                      ? "border-primary bg-primary text-primary-foreground"
                      : "border-border bg-background text-muted-foreground hover:bg-muted",
                  )}
                >
                  {t(`aiProvider.posture.${option}`)}
                </button>
              ))}
            </div>
            <p className="text-xs text-muted-foreground">
              {t(PROVIDER_KIND_HINT_KEYS[kind])}
            </p>
            {autoRemote && autoRemoteShown ? (
              <p
                role="status"
                data-auto-remote={autoRemote}
                className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300"
              >
                <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                <span>{t(AUTO_REMOTE_NOTE_KEYS[autoRemote])}</span>
              </p>
            ) : null}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-key">
              {t("aiProvider.apiKeyLabel")}{" "}
              <span className="text-xs text-muted-foreground">
                {keyRequired
                  ? t("aiProvider.apiKeyRequiredHint")
                  : kind === "local"
                    ? t("aiProvider.apiKeyLocalHint")
                    : t("aiProvider.apiKeyRemoteHint")}
              </span>
            </Label>
            <Input
              id="ai-form-key"
              type="password"
              autoComplete="off"
              value={apiKey}
              aria-required={keyRequired && !editing ? true : undefined}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={
                editing && initial?.has_api_key
                  ? t("aiProvider.apiKeyPlaceholderKeep")
                  : "sk-…"
              }
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-default-model">
              {t("aiProvider.defaultModelLabel")}
            </Label>
            <Input
              id="ai-form-default-model"
              value={defaultModel}
              onChange={(event) => setDefaultModel(event.target.value)}
              placeholder="qwen3.6:35b"
            />
          </div>
          {!isCliUrl ? (
            <details
              className="group rounded-md border"
              open={advancedOpen}
              onToggle={(event) => setAdvancedOpen(event.currentTarget.open)}
              data-provider-advanced
            >
              <summary className="flex cursor-pointer list-none items-center gap-2 rounded-md px-3 py-2 text-xs transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                <ChevronRight
                  className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"
                  aria-hidden="true"
                />
                <span className="font-medium">{t("aiProvider.advancedLabel")}</span>
                <span className="ml-auto truncate text-muted-foreground">
                  {t("aiProvider.advancedSummary", {
                    format: t(`aiProvider.apiFormat.${apiFormat}`),
                  })}
                </span>
              </summary>
              <div className="grid gap-2 border-t px-3 py-2.5">
                <p id="ai-form-api-format" className="text-xs font-medium">
                  {t("aiProvider.apiFormatLabel")}
                </p>
                <div
                  role="radiogroup"
                  aria-labelledby="ai-form-api-format"
                  className="grid gap-1.5"
                >
                  {API_FORMATS.map((option) => (
                    <label
                      key={option}
                      className={cn(
                        "flex cursor-pointer items-start gap-2 rounded-md border px-2.5 py-2 text-xs transition-colors",
                        apiFormat === option
                          ? "border-primary bg-primary/5"
                          : "border-border hover:bg-muted/40",
                      )}
                    >
                      <input
                        type="radio"
                        name="ai-form-api-format"
                        value={option}
                        checked={apiFormat === option}
                        onChange={() => setApiFormat(option)}
                        aria-labelledby={`ai-form-api-format-${option}`}
                        aria-describedby={`ai-form-api-format-${option}-hint`}
                        className="mt-0.5 accent-primary"
                      />
                      <span className="grid gap-0.5">
                        <span id={`ai-form-api-format-${option}`} className="font-medium">
                          {t(`aiProvider.apiFormat.${option}`)}
                        </span>
                        <span
                          id={`ai-form-api-format-${option}-hint`}
                          className="text-muted-foreground"
                        >
                          {t(`aiProvider.apiFormatHint.${option}`)}
                        </span>
                      </span>
                    </label>
                  ))}
                </div>
                {savedDetection ? (
                  <p className="text-xs text-muted-foreground">
                    {t("aiProvider.apiFormatLastDetected", {
                      format: t(`aiProvider.apiFormatName.${savedDetection}`),
                    })}
                  </p>
                ) : null}
              </div>
            </details>
          ) : null}
          <div className="grid gap-2">
            <Label htmlFor="ai-form-notes">{t("aiProvider.notesLabel")}</Label>
            <Input
              id="ai-form-notes"
              value={notes}
              onChange={(event) => setNotes(event.target.value)}
              placeholder={t("aiProvider.notesPlaceholder")}
            />
          </div>
          {kind !== "local" ? (
            <div className="grid gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
              <div className="flex items-start gap-2">
                <ShieldAlert
                  className="mt-0.5 size-4 shrink-0"
                  aria-hidden="true"
                />
                <span>{t("aiProvider.remoteWarning")}</span>
              </div>
              {needsRemoteAck ? (
                <label
                  htmlFor="ai-form-remote-ack"
                  className="flex cursor-pointer items-start gap-2 text-amber-800 dark:text-amber-200"
                >
                  <Checkbox
                    id="ai-form-remote-ack"
                    checked={remoteAcknowledged}
                    onCheckedChange={(checked) => {
                      setRemoteAcknowledged(checked === true);
                    }}
                    className="mt-0.5"
                  />
                  <span>{t("aiProvider.remoteAck")}</span>
                </label>
              ) : null}
            </div>
          ) : null}
          {error ? (
            <p className="text-xs text-destructive">{error}</p>
          ) : null}
          {testStatus.state !== "idle" ? (
            <div
              aria-live="polite"
              data-test-result={testStatus.state}
              className="grid gap-1 text-xs"
            >
              {testStatus.state === "running" ? (
                <p className="inline-flex items-center gap-1 text-muted-foreground">
                  <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                  {t("aiProvider.testing")}
                </p>
              ) : null}
              {testStatus.state === "fail" ? (
                <p className="text-destructive [overflow-wrap:anywhere]">
                  {t("aiProvider.testFailed", { message: testStatus.message })}
                </p>
              ) : null}
              {testStatus.state === "no_model" ? (
                <>
                  <p className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300">
                    <AlertTriangle className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                    <span>
                      {t("aiProvider.testNoReply", { count: testStatus.modelCount })}
                    </span>
                  </p>
                  <p className="pl-5 text-muted-foreground">{t("aiProvider.testNoModel")}</p>
                </>
              ) : null}
              {testStatus.state === "ok" && testStatus.checkKind === "binary_presence" ? (
                <p className="text-emerald-600 dark:text-emerald-400">
                  {t("aiProvider.testOkBinary")}
                </p>
              ) : null}
              {testStatus.state === "ok" && testStatus.checkKind !== "binary_presence" ? (
                <>
                  <p className="flex items-start gap-1.5 text-emerald-600 dark:text-emerald-400">
                    <CheckCircle2 className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                    <span>
                      {t("aiProvider.testOkConnected", { count: testStatus.modelCount })}
                    </span>
                  </p>
                  <ul className="grid gap-0.5 pl-5 text-muted-foreground">
                    {testStatus.probeModel ? (
                      <li>{t("aiProvider.testReplyChecked", { model: testStatus.probeModel })}</li>
                    ) : null}
                    {testStatus.detectedApiFormat ? (
                      <li data-detected-api-format={testStatus.detectedApiFormat}>
                        {t(
                          testStatus.testedApiFormat === "auto"
                            ? "aiProvider.testFormatDetected"
                            : "aiProvider.testFormatSet",
                          {
                            format: t(`aiProvider.apiFormatName.${testStatus.detectedApiFormat}`),
                          },
                        )}
                      </li>
                    ) : null}
                    {testStatus.toolsAccepted === true ? (
                      <li>{t("aiProvider.testToolsAccepted")}</li>
                    ) : null}
                  </ul>
                  {testStatus.toolsAccepted === false ? (
                    <p
                      data-tools-refused
                      className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300"
                    >
                      <ShieldAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                      <span>{t("aiProvider.testToolsRefused")}</span>
                    </p>
                  ) : null}
                </>
              ) : null}
            </div>
          ) : null}
          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => void handleTest()}
              disabled={isBusy || Boolean(testUnavailable)}
              title={testUnavailable ?? undefined}
            >
              {t("aiProvider.testConnection")}
            </Button>
            <Button type="button" variant="ghost" onClick={onClose} disabled={isBusy}>
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {editing ? t("common:actions.save") : t("aiProvider.addProviderButton")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
