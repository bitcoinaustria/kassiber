/**
 * Settings → Assistant: the permission mode a new chat starts in. Each chat
 * can still switch its own mode in the composer; this only sets where it
 * starts. A row of the Assistant group.
 */

import { useTranslation } from "react-i18next";

import {
  PERMISSION_MODE_OPTIONS,
  permissionModeOption,
} from "@/components/ai/permissionMode";
import { useUiStore } from "@/store/ui";

import { SettingsRow, SettingsSelect } from "./SettingsControls";

export function AssistantPermissionSettings() {
  const { t } = useTranslation("settings");
  const { t: tAssistant } = useTranslation("assistant");
  const mode = useUiStore((s) => s.assistantDefaultPermissionMode);
  const setMode = useUiStore((s) => s.setAssistantDefaultPermissionMode);
  const current = permissionModeOption(mode);

  return (
    <SettingsRow
      label={t("ai.permissionMode.label")}
      description={
        <span data-permission-mode-description={mode}>
          {tAssistant(current.descriptionKey)}
        </span>
      }
      htmlFor="settings-permission-mode"
    >
      <SettingsSelect
        value={mode}
        onValueChange={setMode}
        options={PERMISSION_MODE_OPTIONS.map((option) => ({
          value: option.value,
          label: tAssistant(option.labelKey),
        }))}
        triggerProps={{ id: "settings-permission-mode" }}
      />
    </SettingsRow>
  );
}
