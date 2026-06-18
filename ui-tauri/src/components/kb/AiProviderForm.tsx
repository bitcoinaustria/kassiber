/**
 * Add/edit form for AI provider records.
 *
 * Used inline in the Settings modal. The form holds local state and
 * surfaces validation errors from the daemon. The "Test connection"
 * button issues `ai.test_connection` against the entered credentials and
 * reports the result without persisting.
 */

import { Loader2, ShieldAlert, Sparkles } from "lucide-react";
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
import { useDaemonMutation } from "@/daemon/client";
import { getTransport } from "@/daemon/transport";
import { cn } from "@/lib/utils";

export interface AiProviderInput {
  name: string;
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

const CLI_LOCATORS = ["claude-cli://default", "codex-cli://default"] as const;
const PROVIDER_PRESETS = [
  {
    name: "ollama",
    label: "Ollama",
    base_url: "http://localhost:11434/v1",
    kind: "local" as const,
    default_model: "qwen3.6:35b",
  },
  {
    name: "claude-cli",
    label: "Claude CLI",
    base_url: "claude-cli://default",
    kind: "remote" as const,
    default_model: "default",
  },
  {
    name: "codex-cli",
    label: "Codex CLI",
    base_url: "codex-cli://default",
    kind: "remote" as const,
    default_model: "default",
  },
];

function isCliLocator(value: string) {
  return CLI_LOCATORS.some((locator) => value.trim().toLowerCase() === locator);
}

export function AiProviderForm({
  open,
  initial,
  onClose,
  onSaved,
}: AiProviderFormProps) {
  const { t } = useTranslation(["settings", "common"]);
  const [name, setName] = React.useState(initial?.name ?? "");
  const [baseUrl, setBaseUrl] = React.useState(initial?.base_url ?? "");
  const [apiKey, setApiKey] = React.useState(initial?.api_key ?? "");
  const [defaultModel, setDefaultModel] = React.useState(initial?.default_model ?? "");
  const [kind, setKind] = React.useState<AiProviderInput["kind"]>(initial?.kind ?? "local");
  const [notes, setNotes] = React.useState(initial?.notes ?? "");
  const [remoteAcknowledged, setRemoteAcknowledged] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [testStatus, setTestStatus] = React.useState<
    | { state: "idle" }
    | { state: "running" }
    | { state: "ok"; modelCount: number; checkKind?: string }
    | { state: "fail"; message: string }
  >({ state: "idle" });

  const editing = Boolean(initial);

  React.useEffect(() => {
    if (!open) return;
    setName(initial?.name ?? "");
    setBaseUrl(initial?.base_url ?? "");
    setApiKey("");
    setDefaultModel(initial?.default_model ?? "");
    setKind(initial?.kind ?? "local");
    setNotes(initial?.notes ?? "");
    setRemoteAcknowledged(false);
    setError(null);
    setTestStatus({ state: "idle" });
  }, [open, initial]);

  const createProvider = useDaemonMutation("ai.providers.create");
  const updateProvider = useDaemonMutation("ai.providers.update");
  const setApiKeyMutation = useDaemonMutation("ai.providers.set_api_key");

  const handleTest = async () => {
    setTestStatus({ state: "running" });
    try {
      const trimmedUrl = baseUrl.trim();
      if (!trimmedUrl) {
        throw new Error(t("aiProvider.errorBaseUrlRequired"));
      }
      if (!/^https?:\/\//.test(trimmedUrl) && !isCliLocator(trimmedUrl)) {
        throw new Error(t("aiProvider.errorUrlScheme"));
      }
      const args: Record<string, unknown> = { base_url: trimmedUrl };
      const trimmedKey = apiKey.trim();
      if (trimmedKey) {
        throw new Error(t("aiProvider.errorSaveBeforeTest"));
      }
      if (editing && initial) {
        // Empty API-key field means "keep current key" — let the daemon
        // fall back to the stored value so the test exercises real creds.
        args.provider = initial.name;
      }
      const envelope = await getTransport().invoke<{
        check_kind?: string;
        model_count?: number;
      }>({ kind: "ai.test_connection", args });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? t("aiProvider.errorTestFailed"));
      }
      setTestStatus({
        state: "ok",
        checkKind: envelope.data?.check_kind,
        modelCount: envelope.data?.model_count ?? 0,
      });
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      setTestStatus({ state: "fail", message });
    }
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      const needsRemoteAck =
        kind !== "local" && (!initial || initial.kind === "local" || !initial.acknowledged_at);
      if (kind === "local" && isCliLocator(baseUrl)) {
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
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || null,
          kind,
          notes: notes.trim() || null,
        };
        if (needsRemoteAck) {
          args.acknowledged = true;
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
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || undefined,
          kind,
          notes: notes.trim() || undefined,
        };
        if (needsRemoteAck) {
          args.acknowledged = true;
        }
        await createProvider.mutateAsync(args);
        if (apiKey.trim()) {
          await setApiKeyMutation.mutateAsync({
            name: name.trim(),
            api_key: apiKey.trim(),
          });
        }
        onSaved?.(name.trim());
      }
      onClose();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  };

  const isBusy = createProvider.isPending || updateProvider.isPending || setApiKeyMutation.isPending;
  const needsRemoteAck =
    kind !== "local" && (!initial || initial.kind === "local" || !initial.acknowledged_at);
  const canSubmit = !isBusy && (!needsRemoteAck || remoteAcknowledged);

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-lg">
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
              <Label>{t("aiProvider.presetLabel")}</Label>
              <div className="grid grid-cols-3 gap-2">
                {PROVIDER_PRESETS.map((preset) => (
                  <button
                    key={preset.name}
                    type="button"
                    onClick={() => {
                      setName(preset.name);
                      setBaseUrl(preset.base_url);
                      setKind(preset.kind);
                      setDefaultModel(preset.default_model);
                      setNotes(
                        preset.kind === "local"
                          ? t("aiProvider.presetNoteLocal")
                          : t("aiProvider.presetNoteRemote", {
                              label: preset.label,
                            }),
                      );
                      setRemoteAcknowledged(false);
                    }}
                    className="rounded-md border border-border bg-background px-3 py-2 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted"
                  >
                    {preset.label}
                  </button>
                ))}
              </div>
            </div>
          ) : null}
          <div className="grid gap-2">
            <Label htmlFor="ai-form-name">{t("aiProvider.nameLabel")}</Label>
            <Input
              id="ai-form-name"
              value={name}
              onChange={(event) => setName(event.target.value.toLowerCase())}
              placeholder="ollama"
              required
              disabled={editing}
              autoFocus={!editing}
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-url">{t("aiProvider.urlLabel")}</Label>
            <Input
              id="ai-form-url"
              value={baseUrl}
              onChange={(event) => setBaseUrl(event.target.value)}
              placeholder="http://localhost:11434/v1 or claude-cli://default"
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
                  onClick={() => setKind(option)}
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
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-key">
              {t("aiProvider.apiKeyLabel")}{" "}
              <span className="text-xs text-muted-foreground">
                {kind === "local"
                  ? t("aiProvider.apiKeyLocalHint")
                  : t("aiProvider.apiKeyRemoteHint")}
              </span>
            </Label>
            <Input
              id="ai-form-key"
              type="password"
              autoComplete="off"
              value={apiKey}
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
            <p
              className={cn(
                "text-xs",
                testStatus.state === "fail"
                  ? "text-destructive"
                  : testStatus.state === "ok"
                    ? "text-emerald-600 dark:text-emerald-400"
                    : "text-muted-foreground",
              )}
            >
              {testStatus.state === "running" && (
                <span className="inline-flex items-center gap-1">
                  <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                  {t("aiProvider.testing")}
                </span>
              )}
              {testStatus.state === "ok" &&
                (testStatus.checkKind === "binary_presence"
                  ? t("aiProvider.testOkBinary")
                  : t("aiProvider.testOkConnected", {
                      count: testStatus.modelCount,
                    }))}
              {testStatus.state === "fail" &&
                t("aiProvider.testFailed", { message: testStatus.message })}
            </p>
          ) : null}
          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => void handleTest()}
              disabled={isBusy}
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
