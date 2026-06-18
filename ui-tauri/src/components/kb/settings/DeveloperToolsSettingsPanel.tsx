import * as React from "react";
import { ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  APP_LOG_MAX_BYTES,
  APP_LOG_MAX_RECORDS,
  getAppLogBufferSize,
  subscribeAppLogRecords,
} from "@/lib/appLogs";
import { SettingsSwitchRow } from "./SettingsControls";
import { formatBytes } from "./SettingsModel";

export function DeveloperToolsSettingsPanel({
  enabled,
  setEnabled,
  onOpenLogs,
}: {
  enabled: boolean;
  setEnabled: (enabled: boolean) => void;
  onOpenLogs: () => void;
}) {
  const { t } = useTranslation("settings");
  const bytes = useAppLogBufferSize();
  return (
    <section className="space-y-3">
      <p className="max-w-2xl text-sm text-muted-foreground">
        {t("developer.intro")}
      </p>
      <SettingsSwitchRow
        label={t("developer.enableLabel")}
        description={
          enabled ? t("developer.enableOn") : t("developer.enableOff")
        }
        checked={enabled}
        onCheckedChange={setEnabled}
      />
      {enabled ? (
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="w-fit"
          onClick={onOpenLogs}
        >
          <ExternalLink className="size-4" aria-hidden="true" />
          {t("developer.openLogs")}
        </Button>
      ) : null}
      <div className="rounded-md border bg-background p-3 text-sm">
        <p className="font-medium">{t("developer.bufferHeading")}</p>
        <p className="text-muted-foreground">
          {t("developer.bufferDescription", {
            retained: formatBytes(bytes),
            records: APP_LOG_MAX_RECORDS.toLocaleString(),
            bytes: formatBytes(APP_LOG_MAX_BYTES),
          })}
        </p>
      </div>
    </section>
  );
}

export function useAppLogBufferSize(): number {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogBufferSize,
    getAppLogBufferSize,
  );
}
