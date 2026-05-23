import { Minus, Monitor, Moon, Plus, Sun } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  DEFAULT_APP_SCALE,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  type ThemePreference,
} from "@/store/ui";
import { PlannedBadge } from "./SettingsControls";

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
}: {
  theme: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
  appScale: number;
  increaseAppScale: () => void;
  decreaseAppScale: () => void;
  resetAppScale: () => void;
  currency: CurrencyMode;
  setCurrency: (currency: CurrencyMode) => void;
}) {
  const scalePercent = Math.round(appScale * 100);
  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">Theme</h3>
          <p className="text-sm text-muted-foreground">
            Follow the system setting or pin a light or dark appearance.
          </p>
        </div>
        <Tabs
          value={theme}
          onValueChange={(value) => setTheme(value as ThemePreference)}
        >
          <TabsList>
            <TabsTrigger value="system">
              <Monitor className="size-4" aria-hidden="true" />
              System
            </TabsTrigger>
            <TabsTrigger value="light">
              <Sun className="size-4" aria-hidden="true" />
              Light
            </TabsTrigger>
            <TabsTrigger value="dark">
              <Moon className="size-4" aria-hidden="true" />
              Dark
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">Denomination</h3>
          <p className="text-sm text-muted-foreground">
            Choose how balances and reports are shown across the app.
          </p>
        </div>
        <Tabs
          value={currency}
          onValueChange={(value) => setCurrency(value as CurrencyMode)}
        >
          <TabsList>
            <TabsTrigger value="eur">
              <span aria-hidden="true">€</span>
              Euro
            </TabsTrigger>
            <TabsTrigger value="btc">
              <span aria-hidden="true">₿</span>
              Bitcoin
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">Interface scale</h3>
          <p className="text-sm text-muted-foreground">
            Make every screen denser or larger. Applies across the whole app.
          </p>
        </div>
        <div className="flex max-w-md items-center gap-2 rounded-md border bg-background p-2">
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="Decrease interface scale"
            disabled={appScale <= MIN_APP_SCALE}
            onClick={decreaseAppScale}
          >
            <Minus className="size-4" aria-hidden="true" />
          </Button>
          <div className="flex-1 text-center font-mono text-sm tabular-nums">
            {scalePercent}%
          </div>
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="Increase interface scale"
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
            Reset
          </Button>
        </div>
      </section>

      <section className="space-y-2">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold">Language</h3>
          <PlannedBadge />
        </div>
        <p className="text-sm text-muted-foreground">
          Kassiber is English-only today. German (Deutsch) translations are in
          progress.
        </p>
        <Tabs value="en">
          <TabsList>
            <TabsTrigger value="en" disabled>
              English
            </TabsTrigger>
            <TabsTrigger value="de" disabled>
              Deutsch
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>
    </div>
  );
}
