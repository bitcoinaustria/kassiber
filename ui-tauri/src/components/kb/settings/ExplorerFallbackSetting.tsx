import { useTranslation } from "react-i18next";

import { Switch } from "@/components/ui/switch";
import { useUiStore } from "@/store/ui";

import { SettingsRow } from "./SettingsControls";

/**
 * Whether explorer links may open on a public website when no explorer of
 * the user's own is configured. A row of the Privacy panel's outgoing
 * connections. Manual and regtest setups never fall back, so the switch is
 * shown but inert there. Opening a link stays a click; nothing is fetched.
 */
export function ExplorerFallbackSetting() {
  const { t } = useTranslation("settings");
  const setupAllows = useUiStore((s) => s.explorerSettings.publicFallbacks);
  const allowed = useUiStore(
    (s) => s.explorerSettings.publicFallbacksAllowed !== false,
  );
  const setExplorerSettings = useUiStore((s) => s.setExplorerSettings);

  return (
    <SettingsRow
      anchorId="explorerFallbacks"
      label={t("privacy.explorerFallbacksLabel")}
      description={
        !setupAllows
          ? t("privacy.explorerFallbacksUnavailable")
          : allowed
            ? t("privacy.explorerFallbacksOn")
            : t("privacy.explorerFallbacksOff")
      }
      htmlFor="settings-explorer-fallbacks"
    >
      <Switch
        id="settings-explorer-fallbacks"
        checked={setupAllows && allowed}
        disabled={!setupAllows}
        onCheckedChange={(checked) =>
          setExplorerSettings({ publicFallbacksAllowed: checked })
        }
      />
    </SettingsRow>
  );
}
