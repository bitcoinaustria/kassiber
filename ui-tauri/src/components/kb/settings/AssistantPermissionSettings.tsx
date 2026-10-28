/**
 * Settings → AI: the permission mode a new chat starts in. Each chat can
 * still switch its own mode in the composer; this only sets where it starts.
 */

import { useTranslation } from "react-i18next";

import {
  PERMISSION_MODE_OPTIONS,
  permissionModeOption,
} from "@/components/ai/permissionMode";
import type { AiPermissionMode } from "@/daemon/stream";
import { useUiStore } from "@/store/ui";

import { SegmentedControl, SettingsGroup, SettingsRow } from "./SettingsControls";

export function AssistantPermissionSettings() {
  const { t } = useTranslation("settings");
  const { t: tAssistant } = useTranslation("assistant");
  const mode = useUiStore((s) => s.assistantDefaultPermissionMode);
  const setMode = useUiStore((s) => s.setAssistantDefaultPermissionMode);
  const current = permissionModeOption(mode);

  return (
    <SettingsGroup title={t("ai.permissionMode.heading")}>
      <SettingsRow
        label={t("ai.permissionMode.label")}
        description={t("ai.permissionMode.description")}
      >
        <SegmentedControl<AiPermissionMode>
          label={t("ai.permissionMode.label")}
          value={mode}
          onValueChange={setMode}
          options={PERMISSION_MODE_OPTIONS.map((option) => {
            const Icon = option.icon;
            return {
              value: option.value,
              label: tAssistant(option.labelKey),
              icon: <Icon aria-hidden="true" />,
            };
          })}
        />
      </SettingsRow>
      <p
        className="px-4 py-3 text-sm text-muted-foreground"
        data-permission-mode-description={mode}
      >
        {tAssistant(current.descriptionKey)}
      </p>
    </SettingsGroup>
  );
}
