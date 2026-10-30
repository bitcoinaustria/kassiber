import { useTranslation } from "react-i18next";

import { SettingsGroup, SettingsRow, SettingsSelect } from "./SettingsControls";
import { SUPPORTED_LANGUAGES, type LanguageCode } from "@/i18n/config";
import {
  APP_SCALE_STEP,
  DEFAULT_APP_SCALE,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  useUiStore,
  type AssistantClosedBarMode,
  type AssistantDockPosition,
  type ThemePreference,
} from "@/store/ui";

export type CurrencyMode = "btc" | "eur";

const percent = (scale: number) => Math.round(scale * 100);

/** Every scale the store keeps, as whole percents: 80, 85, … 120. */
const SCALE_PERCENTS = Array.from(
  { length: Math.round((MAX_APP_SCALE - MIN_APP_SCALE) / APP_SCALE_STEP) + 1 },
  (_, index) => percent(MIN_APP_SCALE + index * APP_SCALE_STEP),
);

export function AppearanceSettingsPanel({
  theme,
  setTheme,
  appScale,
  setAppScale,
  currency,
  setCurrency,
  lang,
  setLang,
}: {
  theme: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
  appScale: number;
  setAppScale: (appScale: number) => void;
  currency: CurrencyMode;
  setCurrency: (currency: CurrencyMode) => void;
  lang: LanguageCode;
  setLang: (lang: LanguageCode) => void;
}) {
  const { t } = useTranslation("settings");
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const assistantClosedBar = useUiStore((s) => s.assistantClosedBar);
  const setAssistantClosedBar = useUiStore((s) => s.setAssistantClosedBar);
  const assistantDockPosition = useUiStore((s) => s.assistantDockPosition);
  const setAssistantDockPosition = useUiStore(
    (s) => s.setAssistantDockPosition,
  );
  return (
    <div className="space-y-6">
      <SettingsGroup>
        <SettingsRow
          label={t("appearance.theme.title")}
          description={t("appearance.theme.description")}
          htmlFor="settings-theme"
        >
          <SettingsSelect<ThemePreference>
            value={theme}
            onValueChange={setTheme}
            options={[
              { value: "system", label: t("appearance.theme.system") },
              { value: "light", label: t("appearance.theme.light") },
              { value: "dark", label: t("appearance.theme.dark") },
            ]}
            triggerProps={{ id: "settings-theme" }}
          />
        </SettingsRow>
        <SettingsRow
          label={t("appearance.denomination.title")}
          description={t("appearance.denomination.description")}
          htmlFor="settings-denomination"
        >
          <SettingsSelect<CurrencyMode>
            value={currency}
            onValueChange={setCurrency}
            options={[
              { value: "eur", label: t("appearance.denomination.euro") },
              { value: "btc", label: t("appearance.denomination.bitcoin") },
            ]}
            triggerProps={{ id: "settings-denomination" }}
          />
        </SettingsRow>
        <SettingsRow
          label={t("appearance.scale.title")}
          description={t("appearance.scale.description")}
          htmlFor="settings-scale"
        >
          <SettingsSelect
            value={String(percent(appScale))}
            onValueChange={(value) => setAppScale(Number(value) / 100)}
            options={SCALE_PERCENTS.map((value) => ({
              value: String(value),
              label:
                value === percent(DEFAULT_APP_SCALE)
                  ? t("appearance.scale.defaultValue", { percent: value })
                  : t("appearance.scale.value", { percent: value }),
            }))}
            triggerProps={{ id: "settings-scale" }}
          />
        </SettingsRow>
        <SettingsRow
          label={t("appearance.language.title")}
          description={t("appearance.language.description")}
          htmlFor="settings-language"
        >
          <SettingsSelect<LanguageCode>
            value={lang}
            onValueChange={setLang}
            options={SUPPORTED_LANGUAGES.map((language) => ({
              value: language.code,
              label: language.label,
            }))}
            triggerProps={{ id: "settings-language" }}
          />
        </SettingsRow>
      </SettingsGroup>

      {aiFeaturesEnabled ? (
        <SettingsGroup title={t("appearance.assistantDock.title")}>
          <SettingsRow
            label={t("appearance.assistantDock.closedBar")}
            description={t("appearance.assistantDock.closedBarDescription")}
            htmlFor="settings-assistant-closed-bar"
          >
            <SettingsSelect<AssistantClosedBarMode>
              value={assistantClosedBar}
              onValueChange={setAssistantClosedBar}
              options={[
                { value: "composer", label: t("appearance.assistantDock.closedBarComposer") },
                { value: "launcher", label: t("appearance.assistantDock.closedBarLauncher") },
                { value: "hidden", label: t("appearance.assistantDock.closedBarHidden") },
              ]}
              triggerProps={{ id: "settings-assistant-closed-bar" }}
            />
          </SettingsRow>
          {assistantClosedBar === "hidden" ? null : (
            <SettingsRow
              label={t("appearance.assistantDock.position")}
              htmlFor="settings-assistant-position"
            >
              <SettingsSelect<AssistantDockPosition>
                value={assistantDockPosition}
                onValueChange={setAssistantDockPosition}
                options={[
                  { value: "left", label: t("appearance.assistantDock.positionLeft") },
                  { value: "center", label: t("appearance.assistantDock.positionCenter") },
                  { value: "right", label: t("appearance.assistantDock.positionRight") },
                ]}
                triggerProps={{ id: "settings-assistant-position" }}
              />
            </SettingsRow>
          )}
        </SettingsGroup>
      ) : null}
    </div>
  );
}
