import { Minus, Monitor, Moon, Plus, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SUPPORTED_LANGUAGES, type LanguageCode } from "@/i18n/config";
import {
  DEFAULT_APP_SCALE,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
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
  const scalePercent = Math.round(appScale * 100);
  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">
            {t("appearance.theme.title")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("appearance.theme.description")}
          </p>
        </div>
        <Tabs
          value={theme}
          onValueChange={(value) => setTheme(value as ThemePreference)}
        >
          <TabsList>
            <TabsTrigger value="system">
              <Monitor className="size-4" aria-hidden="true" />
              {t("appearance.theme.system")}
            </TabsTrigger>
            <TabsTrigger value="light">
              <Sun className="size-4" aria-hidden="true" />
              {t("appearance.theme.light")}
            </TabsTrigger>
            <TabsTrigger value="dark">
              <Moon className="size-4" aria-hidden="true" />
              {t("appearance.theme.dark")}
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">
            {t("appearance.denomination.title")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("appearance.denomination.description")}
          </p>
        </div>
        <Tabs
          value={currency}
          onValueChange={(value) => setCurrency(value as CurrencyMode)}
        >
          <TabsList>
            <TabsTrigger value="eur">
              <span aria-hidden="true">€</span>
              {t("appearance.denomination.euro")}
            </TabsTrigger>
            <TabsTrigger value="btc">
              <span aria-hidden="true">₿</span>
              {t("appearance.denomination.bitcoin")}
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">
            {t("appearance.scale.title")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("appearance.scale.description")}
          </p>
        </div>
        <div className="flex max-w-md items-center gap-2 rounded-md border bg-background p-2">
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label={t("appearance.scale.decrease")}
            disabled={appScale <= MIN_APP_SCALE}
            onClick={decreaseAppScale}
          >
            <Minus className="size-4" aria-hidden="true" />
          </Button>
          <div className="flex-1 text-center font-mono text-sm tabular-nums">
            {t("appearance.scale.value", { percent: scalePercent })}
          </div>
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label={t("appearance.scale.increase")}
            disabled={appScale >= MAX_APP_SCALE}
            onClick={increaseAppScale}
          >
            <Plus className="size-4" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={resetAppScale}
            disabled={appScale === DEFAULT_APP_SCALE}
          >
            {t("common:actions.reset")}
          </Button>
        </div>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">
            {t("appearance.language.title")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("appearance.language.description")}
          </p>
        </div>
        <Tabs
          value={lang}
          onValueChange={(value) => setLang(value as LanguageCode)}
        >
          <TabsList>
            {SUPPORTED_LANGUAGES.map((language) => (
              <TabsTrigger key={language.code} value={language.code}>
                {language.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </section>
    </div>
  );
}
