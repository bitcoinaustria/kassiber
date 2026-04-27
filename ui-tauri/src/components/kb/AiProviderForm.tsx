/**
 * Add/edit form for AI provider records.
 *
 * Used inline in the Settings modal. The form holds local state and
 * surfaces validation errors from the daemon. The "Test connection"
 * button issues `ai.list_models` against the entered credentials and
 * reports the result without persisting.
 */

import { Loader2, ShieldAlert, Sparkles } from "lucide-react";
import * as React from "react";

import { Button } from "@/components/ui/button";
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
}

interface AiProviderFormProps {
  open: boolean;
  initial?: ExistingAiProvider | null;
  onClose: () => void;
  onSaved?: (name: string) => void;
}

const PROVIDER_KIND_HINTS: Record<AiProviderInput["kind"], string> = {
  local: "Runs on this machine. No data leaves the device.",
  remote: "Cloud or LAN provider. Prompts leave the device.",
  tee: "Encrypted attestation provider (e.g. Maple AI). Off-device but with documented confidentiality guarantees.",
};

export function AiProviderForm({
  open,
  initial,
  onClose,
  onSaved,
}: AiProviderFormProps) {
  const [name, setName] = React.useState(initial?.name ?? "");
  const [baseUrl, setBaseUrl] = React.useState(initial?.base_url ?? "");
  const [apiKey, setApiKey] = React.useState(initial?.api_key ?? "");
  const [defaultModel, setDefaultModel] = React.useState(initial?.default_model ?? "");
  const [kind, setKind] = React.useState<AiProviderInput["kind"]>(initial?.kind ?? "local");
  const [notes, setNotes] = React.useState(initial?.notes ?? "");
  const [error, setError] = React.useState<string | null>(null);
  const [testStatus, setTestStatus] = React.useState<
    | { state: "idle" }
    | { state: "running" }
    | { state: "ok"; modelCount: number }
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
    setError(null);
    setTestStatus({ state: "idle" });
  }, [open, initial]);

  const createProvider = useDaemonMutation("ai.providers.create");
  const updateProvider = useDaemonMutation("ai.providers.update");

  const handleTest = async () => {
    setTestStatus({ state: "running" });
    try {
      const trimmedUrl = baseUrl.trim();
      if (!trimmedUrl) {
        throw new Error("Base URL is required");
      }
      if (!/^https?:\/\//.test(trimmedUrl)) {
        throw new Error("Base URL needs a scheme (http:// or https://)");
      }
      const args: Record<string, unknown> = { base_url: trimmedUrl };
      const trimmedKey = apiKey.trim();
      if (trimmedKey) {
        args.api_key = trimmedKey;
      } else if (editing && initial) {
        // Empty API-key field means "keep current key" — let the daemon
        // fall back to the stored value so the test exercises real creds.
        args.provider = initial.name;
      }
      const envelope = await getTransport().invoke<{
        model_count?: number;
      }>({ kind: "ai.test_connection", args });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? "Connection test failed");
      }
      setTestStatus({
        state: "ok",
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
      if (editing && initial) {
        const args: Record<string, unknown> = {
          name: initial.name,
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || null,
          kind,
          notes: notes.trim() || null,
        };
        if (apiKey.trim()) {
          args.api_key = apiKey.trim();
        }
        await updateProvider.mutateAsync(args);
        onSaved?.(initial.name);
      } else {
        const args: Record<string, unknown> = {
          name: name.trim(),
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || undefined,
          kind,
          notes: notes.trim() || undefined,
        };
        if (apiKey.trim()) {
          args.api_key = apiKey.trim();
        }
        await createProvider.mutateAsync(args);
        onSaved?.(name.trim());
      }
      onClose();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  };

  const isBusy = createProvider.isPending || updateProvider.isPending;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="size-4 text-primary" aria-hidden="true" />
            {editing ? "Edit AI provider" : "Add AI provider"}
          </DialogTitle>
          <DialogDescription>
            OpenAI-compatible endpoints only. Local Ollama runs on
            <code className="mx-1 rounded bg-muted px-1 py-0.5 text-[11px]">http://localhost:11434/v1</code>.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4">
          <div className="grid gap-2">
            <Label htmlFor="ai-form-name">Name</Label>
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
            <Label htmlFor="ai-form-url">Base URL</Label>
            <Input
              id="ai-form-url"
              value={baseUrl}
              onChange={(event) => setBaseUrl(event.target.value)}
              placeholder="http://localhost:11434/v1"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-kind">Privacy posture</Label>
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
                  {option}
                </button>
              ))}
            </div>
            <p className="text-xs text-muted-foreground">
              {PROVIDER_KIND_HINTS[kind]}
            </p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-key">
              API key{" "}
              <span className="text-xs text-muted-foreground">
                {kind === "local" ? "(usually not required)" : "(optional for keyless deployments)"}
              </span>
            </Label>
            <Input
              id="ai-form-key"
              type="password"
              autoComplete="off"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={
                editing && initial?.has_api_key ? "Leave blank to keep current key" : "sk-…"
              }
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-default-model">Default model (optional)</Label>
            <Input
              id="ai-form-default-model"
              value={defaultModel}
              onChange={(event) => setDefaultModel(event.target.value)}
              placeholder="qwen3.6:35b"
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ai-form-notes">Notes (optional)</Label>
            <Input
              id="ai-form-notes"
              value={notes}
              onChange={(event) => setNotes(event.target.value)}
              placeholder="Notes for your reference"
            />
          </div>
          {kind !== "local" ? (
            <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
              <ShieldAlert
                className="mt-0.5 size-4 shrink-0"
                aria-hidden="true"
              />
              <span>
                Prompts will leave this device. Do not paste raw credentials,
                wallet exports, or private descriptors unless your threat model
                allows it.
              </span>
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
                  Testing…
                </span>
              )}
              {testStatus.state === "ok" &&
                `Connected — ${testStatus.modelCount} model${testStatus.modelCount === 1 ? "" : "s"} reachable.`}
              {testStatus.state === "fail" && `Test failed: ${testStatus.message}`}
            </p>
          ) : null}
          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => void handleTest()}
              disabled={isBusy}
            >
              Test connection
            </Button>
            <Button type="button" variant="ghost" onClick={onClose} disabled={isBusy}>
              Cancel
            </Button>
            <Button type="submit" disabled={isBusy}>
              {editing ? "Save" : "Add provider"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
