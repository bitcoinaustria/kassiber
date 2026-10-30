import { Trans, useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import type { TerminalCommandStatus } from "@/daemon/transport";
import { SettingsCopyRow, SettingsGroup, SettingsRow } from "./SettingsControls";

const VERIFY_COMMAND = "kassiber status";

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
  const copyLabel = (label: string) => t("page.copy", { label });
  return (
    <SettingsGroup
      footer={
        <Trans
          i18nKey="terminal.intro"
          ns="settings"
          components={[<span className="font-mono" />]}
        />
      }
    >
      {error ? (
        <p role="alert" className="px-4 py-3 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      <SettingsRow
        label={t("terminal.statusLabel")}
        description={
          status ? (
            <span
              className={
                status.installed && status.pathOnPath
                  ? "text-emerald-600 dark:text-emerald-400"
                  : "text-amber-600 dark:text-amber-400"
              }
            >
              {status.message}
            </span>
          ) : (
            t("terminal.inspecting")
          )
        }
      >
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={onInstall}
          disabled={pending || status?.conflict || status?.available === false}
        >
          {actionLabel}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={onRefresh}
          disabled={pending}
        >
          {t("common:actions.refresh")}
        </Button>
        {status?.managed ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="hover:text-destructive"
            onClick={onRemove}
            disabled={pending}
          >
            {t("common:actions.remove")}
          </Button>
        ) : null}
      </SettingsRow>
      {status ? (
        <>
          <SettingsCopyRow
            label={t("terminal.commandLabel")}
            value={status.commandPath || null}
            copyLabel={copyLabel(t("terminal.commandLabel"))}
            placeholder="—"
          />
          <SettingsCopyRow
            label={t("terminal.targetLabel")}
            value={status.targetPath || null}
            copyLabel={copyLabel(t("terminal.targetLabel"))}
            placeholder="—"
          />
          <SettingsCopyRow
            label={t("terminal.verifyLabel")}
            value={VERIFY_COMMAND}
            copyLabel={copyLabel(VERIFY_COMMAND)}
          />
          {!status.pathOnPath ? (
            <SettingsCopyRow
              label={t("terminal.pathLabel")}
              value={status.pathHint || null}
              copyLabel={copyLabel(t("terminal.pathLabel"))}
              placeholder="—"
            />
          ) : null}
        </>
      ) : null}
    </SettingsGroup>
  );
}
