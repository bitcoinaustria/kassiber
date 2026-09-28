import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  Minus,
  Monitor,
  Moon,
  Plus,
  Sun,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  SegmentedControl,
  SettingsGroup,
  SettingsRow,
} from "./SettingsControls";
import { SUPPORTED_LANGUAGES, type LanguageCode } from "@/i18n/config";
import {
  DEFAULT_APP_SCALE,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  useUiStore,
  type AssistantDockPosition,
  type ThemePreference,
} from "@/store/ui";

export type CurrencyMode = "btc" | "eur";

export function AppearanceSettingsPanel({
  theme,
  setTheme,
  appScale,
  increaseAppScale,
  decreaseAppScale,
  resetAppScale,
  currency,
  setCurrency,
  lang,
  setLang,
}: {
  theme: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
  appScale: number;
  increaseAppScale: () => void;
  decreaseAppScale: () => void;
  resetAppScale: () => void;
  currency: CurrencyMode;
  setCurrency: (currency: CurrencyMode) => void;
  lang: LanguageCode;
  setLang: (lang: LanguageCode) => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const assistantDockAutoHide = useUiStore((s) => s.assistantDockAutoHide);
  const setAssistantDockAutoHide = useUiStore(
    (s) => s.setAssistantDockAutoHide,
  );
  const assistantDockPosition = useUiStore((s) => s.assistantDockPosition);
  const setAssistantDockPosition = useUiStore(
    (s) => s.setAssistantDockPosition,
  );
  const scalePercent = Math.round(appScale * 100);
  return (
    <div className="space-y-6">
      <SettingsGroup>
        <SettingsRow
          label={t("appearance.theme.title")}
          description={t("appearance.theme.description")}
        >
          <SegmentedControl
            label={t("appearance.theme.title")}
            value={theme}
            onValueChange={setTheme}
            options={[
              { value: "system", label: t("appearance.theme.system"), icon: <Monitor aria-hidden="true" /> },
              { value: "light", label: t("appearance.theme.light"), icon: <Sun aria-hidden="true" /> },
              { value: "dark", label: t("appearance.theme.dark"), icon: <Moon aria-hidden="true" /> },
            ]}
          />
        </SettingsRow>
        <SettingsRow
          label={t("appearance.denomination.title")}
          description={t("appearance.denomination.description")}
        >
          <SegmentedControl<CurrencyMode>
            label={t("appearance.denomination.title")}
            value={currency}
            onValueChange={setCurrency}
            options={[
              { value: "eur", label: t("appearance.denomination.euro"), icon: <span aria-hidden="true">€</span> },
              { value: "btc", label: t("appearance.denomination.bitcoin"), icon: <span aria-hidden="true">₿</span> },
            ]}
          />
        </SettingsRow>
        <SettingsRow
          label={t("appearance.scale.title")}
          description={t("appearance.scale.description")}
        >
          <div className="inline-flex items-center rounded-lg border">
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="rounded-r-none"
              aria-label={t("appearance.scale.decrease")}
              disabled={appScale <= MIN_APP_SCALE}
              onClick={decreaseAppScale}
            >
              <Minus className="size-4" aria-hidden="true" />
            </Button>
            <span className="w-14 text-center font-mono text-sm tabular-nums">
              {t("appearance.scale.value", { percent: scalePercent })}
            </span>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="rounded-l-none"
              aria-label={t("appearance.scale.increase")}
              disabled={appScale >= MAX_APP_SCALE}
              onClick={increaseAppScale}
            >
              <Plus className="size-4" aria-hidden="true" />
            </Button>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={resetAppScale}
            disabled={appScale === DEFAULT_APP_SCALE}
          >
            {t("common:actions.reset")}
          </Button>
        </SettingsRow>
        <SettingsRow
          label={t("appearance.language.title")}
          description={t("appearance.language.description")}
        >
          <SegmentedControl<LanguageCode>
            label={t("appearance.language.title")}
            value={lang}
            onValueChange={setLang}
            options={SUPPORTED_LANGUAGES.map((language) => ({
              value: language.code,
              label: language.label,
            }))}
          />
        </SettingsRow>
      </SettingsGroup>

      {aiFeaturesEnabled ? (
        <SettingsGroup title={t("appearance.assistantDock.title")}>
          <SettingsRow
            label={t("appearance.assistantDock.autoHide")}
            description={t("appearance.assistantDock.description")}
            htmlFor="assistant-dock-auto-hide"
          >
            <Switch
              id="assistant-dock-auto-hide"
              checked={assistantDockAutoHide}
              onCheckedChange={setAssistantDockAutoHide}
            />
          </SettingsRow>
          <SettingsRow label={t("appearance.assistantDock.position")}>
            <SegmentedControl<AssistantDockPosition>
              label={t("appearance.assistantDock.position")}
              value={assistantDockPosition}
              onValueChange={setAssistantDockPosition}
              options={[
                { value: "left", label: t("appearance.assistantDock.positionLeft"), icon: <AlignLeft aria-hidden="true" /> },
                { value: "center", label: t("appearance.assistantDock.positionCenter"), icon: <AlignCenter aria-hidden="true" /> },
                { value: "right", label: t("appearance.assistantDock.positionRight"), icon: <AlignRight aria-hidden="true" /> },
              ]}
            />
          </SettingsRow>
        </SettingsGroup>
      ) : null}
    </div>
  );
}
