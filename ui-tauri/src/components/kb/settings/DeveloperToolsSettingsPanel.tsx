import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  APP_LOG_MAX_BYTES,
  APP_LOG_MAX_RECORDS,
  getAppLogBufferSize,
  subscribeAppLogRecords,
} from "@/lib/appLogs";
import { formatCount } from "@/lib/localeFormat";
import { SettingsGroup, SettingsRow } from "./SettingsControls";
import { formatBytes } from "./SettingsModel";

export function DeveloperToolsSettingsPanel({
  enabled,
  setEnabled,
  bannerVisible,
  setBannerVisible,
  onOpenLogs,
}: {
  enabled: boolean;
  setEnabled: (enabled: boolean) => void;
  bannerVisible: boolean;
  setBannerVisible: (visible: boolean) => void;
  onOpenLogs: () => void;
}) {
  const { t } = useTranslation("settings");
  const bytes = useAppLogBufferSize();
  return (
    <SettingsGroup footer={t("developer.intro")}>
      <SettingsRow
        label={t("developer.enableLabel")}
        description={enabled ? t("developer.enableOn") : t("developer.enableOff")}
        htmlFor="settings-developer-tools"
      >
        <Switch
          id="settings-developer-tools"
          checked={enabled}
          onCheckedChange={setEnabled}
        />
      </SettingsRow>
      <SettingsRow
        label={t("developer.alphaBannerLabel")}
        description={t("developer.alphaBannerDescription")}
        htmlFor="settings-alpha-banner"
      >
        <Switch
          id="settings-alpha-banner"
          checked={bannerVisible}
          onCheckedChange={setBannerVisible}
        />
      </SettingsRow>
      <SettingsRow
        label={t("developer.bufferHeading")}
        description={t("developer.bufferDescription", {
          retained: formatBytes(bytes),
          records: formatCount(APP_LOG_MAX_RECORDS),
          bytes: formatBytes(APP_LOG_MAX_BYTES),
        })}
      >
        {enabled ? (
          <Button type="button" variant="outline" size="sm" onClick={onOpenLogs}>
            {t("developer.openLogs")}
          </Button>
        ) : null}
      </SettingsRow>
    </SettingsGroup>
  );
}

export function useAppLogBufferSize(): number {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogBufferSize,
    getAppLogBufferSize,
  );
}
