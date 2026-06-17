import { AlertTriangle, CheckCircle2, RefreshCw, Terminal, XCircle } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import type { TerminalCommandStatus } from "@/daemon/transport";
import { cn } from "@/lib/utils";
import { CommandLine, PathField } from "./SettingsControls";

export function TerminalCommandSettingsPanel({
  status,
  error,
  pending,
  onRefresh,
  onInstall,
  onRemove,
}: {
  status: TerminalCommandStatus | null;
  error: string | null;
  pending: boolean;
  onRefresh: () => void;
  onInstall: () => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const actionLabel = status?.needsRepair
    ? t("terminal.repairAction")
    : status?.installed
      ? t("terminal.reinstallAction")
      : t("terminal.installAction");
  return (
    <section className="space-y-4">
      <p className="max-w-2xl text-sm text-muted-foreground">
        <Trans
          i18nKey="terminal.intro"
          ns="settings"
          components={[<span className="font-mono" />]}
        />
      </p>

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          onClick={onInstall}
          disabled={pending || status?.conflict || status?.available === false}
        >
          {pending ? (
            <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
          ) : (
            <Terminal className="size-4" aria-hidden="true" />
          )}
          {actionLabel}
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={onRefresh}
          disabled={pending}
        >
          <RefreshCw className="size-4" aria-hidden="true" />
          {t("common:actions.refresh")}
        </Button>
        {status?.managed ? (
          <Button
            type="button"
            variant="ghost"
            onClick={onRemove}
            disabled={pending}
          >
            {t("common:actions.remove")}
          </Button>
        ) : null}
      </div>

      {error ? (
        <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
          <XCircle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}

      {status ? (
        <div className="space-y-3">
          <div
            className={cn(
              "flex items-start gap-2 rounded-md border p-3 text-sm",
              status.installed && status.pathOnPath
                ? "border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300"
                : "border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-200",
            )}
          >
            {status.installed && status.pathOnPath ? (
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            ) : (
              <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            )}
            <span>{status.message}</span>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <PathField
              id="settings-terminal-command"
              label={t("terminal.commandLabel")}
              value={status.commandPath || null}
            />
            <PathField
              id="settings-terminal-target"
              label={t("terminal.targetLabel")}
              value={status.targetPath || null}
            />
          </div>

          <div className="space-y-1.5">
            <Label>{t("terminal.verifyLabel")}</Label>
            <CommandLine command="kassiber status" />
          </div>

          {!status.pathOnPath ? (
            <PathField
              id="settings-terminal-path"
              label={t("terminal.pathLabel")}
              value={status.pathHint || null}
            />
          ) : null}
        </div>
      ) : (
        <div className="rounded-md border bg-muted/30 p-3 text-sm text-muted-foreground">
          {t("terminal.inspecting")}
        </div>
      )}
    </section>
  );
}
