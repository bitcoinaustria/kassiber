import { Link } from "@tanstack/react-router";
import * as React from "react";
import { ArrowUpRight, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { formatBtc, formatWholeFiatAmount, type Currency } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";
import {
  DEFAULT_HOLDINGS_VIEW,
  HOLDINGS_LAYOUTS,
  HOLDINGS_LIMITS,
  HOLDINGS_RAILS,
  type HoldingsEmpty,
  type HoldingsLayout,
  type HoldingsSort,
  type HoldingsView,
  useUiStore,
} from "@/store/ui";

import {
  HoldingsListLayout,
  HoldingsTableLayout,
  HoldingsTilesLayout,
  HoldingsTrendLayout,
  type HoldingsLayoutProps,
} from "./HoldingsLayouts";
import {
  activeMarketFiatCurrency,
  blurClass,
  buildHoldingsBySource,
  portfolioCompletenessDetail,
} from "./model";
import { maskSensitive, useSensitive } from "@/lib/sensitive";

/** Layouts that list every source, so none folds into "Other sources". */
const UNFOLDED_LAYOUTS: readonly HoldingsLayout[] = ["table"];

/**
 * Where the bitcoin sits, across the page under the chart: the sources in the
 * layout and order the settings menu picks. Emptied sources fold into one
 * line unless the menu lists or hides them.
 */
export const HoldingsBySourceChart = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const { t } = useTranslation("overview");
  const hideShares = useSensitive("share");
  const view = useUiStore((s) => s.holdingsView) ?? DEFAULT_HOLDINGS_VIEW;
  const setView = useUiStore((s) => s.setHoldingsView);
  const [emptyOpen, setEmptyOpen] = React.useState(false);
  const isBitcoinMode = currency === "btc";
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const holdings = buildHoldingsBySource(
    snapshot,
    UNFOLDED_LAYOUTS.includes(view.layout) ? { ...view, limit: 0 } : view,
  );
  const completenessDetail = portfolioCompletenessDetail(snapshot);
  const unrealizedPercent = snapshot.fiat.eurCostBasis
    ? (snapshot.fiat.eurUnrealized / snapshot.fiat.eurCostBasis) * 100
    : 0;
  const filtered = holdings.filteredOut > 0;
  const totalValue = holdings.items.reduce((sum, item) => sum + item.value, 0);
  // Without a market rate a fiat figure would be a stale or zero placeholder.
  const rateMissing = fiatCompleteness(snapshot.fiat).marketRateMissing;
  const fiatValueUnavailable = !isBitcoinMode && rateMissing;
  const valueLabel = (btc: number, value: number) =>
    maskSensitive(
      isBitcoinMode
        ? formatBtc(btc, { precision: 3 })
        : fiatValueUnavailable
          ? "—"
          : formatWholeFiatAmount(value, fiatCurrency),
      hideSensitive,
    );
  const rows = emptyOpen ? [...holdings.items, ...holdings.folded] : holdings.items;
  const layoutProps: HoldingsLayoutProps = {
    rows,
    hideSensitive,
    hideShares,
    valueLabel,
    fiatLabel: (value) =>
      rateMissing ? null : maskSensitive(formatWholeFiatAmount(value, fiatCurrency), hideSensitive),
    isBitcoinMode,
  };

  return (
    <section className="@container/holdings kb-surface space-y-2.5 p-3 sm:px-4" aria-label={t("holdings.aria")}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5">
          <h2 className="text-sm font-medium">{t("holdings.title")}</h2>
          <span className={cn("text-sm font-semibold tabular-nums", blurClass(hideSensitive))}>
            {valueLabel(holdings.totalBtc, totalValue)}
          </span>
          {/* Tile and chart already say when the basis is incomplete, and a
              book-wide figure does not belong beside a filtered total. */}
          {isBitcoinMode || completenessDetail || filtered ? null : (
            <p className="text-xs text-muted-foreground">
              <span
                className={cn(
                  "font-medium",
                  unrealizedPercent >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-red-600 dark:text-red-400",
                  blurClass(hideSensitive),
                )}
              >
                {maskSensitive(
                  `${unrealizedPercent >= 0 ? "+" : ""}${unrealizedPercent.toFixed(1)}%`,
                  hideShares,
                )}
              </span>{" "}
              {t("holdings.vsCostBasis")}
            </p>
          )}
          {filtered ? (
            <button
              type="button"
              className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => setView({ rails: [...HOLDINGS_RAILS] })}
              title={t("holdings.showAllRails")}
            >
              {t("holdings.filteredOut", { count: holdings.filteredOut })}
            </button>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-3">
          <Link
            to="/connections"
            className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {t("holdings.viewConnections")}
            <ArrowUpRight className="size-3" aria-hidden="true" />
          </Link>
          <HoldingsSettingsMenu view={view} onChange={setView} />
        </div>
      </div>

      {rows.length ? (
        view.layout === "tiles" ? (
          <HoldingsTilesLayout {...layoutProps} />
        ) : view.layout === "table" ? (
          <HoldingsTableLayout {...layoutProps} />
        ) : view.layout === "trend" ? (
          <HoldingsTrendLayout {...layoutProps} />
        ) : (
          <HoldingsListLayout {...layoutProps} />
        )
      ) : (
        <p className="text-xs text-muted-foreground">
          {filtered ? t("holdings.noneListed") : t("holdings.empty")}
        </p>
      )}

      {holdings.folded.length ? (
        <button
          type="button"
          aria-expanded={emptyOpen}
          className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => setEmptyOpen((open) => !open)}
        >
          {emptyOpen
            ? t("holdings.hideEmpty")
            : t("holdings.emptySources", { count: holdings.folded.length })}
        </button>
      ) : null}
    </section>
  );
};

const HoldingsSettingsMenu = ({
  view,
  onChange,
}: {
  view: HoldingsView;
  onChange: (patch: Partial<HoldingsView>) => void;
}) => {
  const { t } = useTranslation("overview");
  const keepOpen = (event: Event) => event.preventDefault();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={t("holdings.settingsAria")}
          className="inline-flex items-center rounded-md p-0.5 text-muted-foreground transition-colors hover:bg-muted/45 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-muted data-[state=open]:text-foreground"
        >
          <Settings className="size-3.5" aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel className="text-2xs text-muted-foreground">{t("holdings.layoutLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={view.layout}
          onValueChange={(layout) => onChange({ layout: layout as HoldingsLayout })}
        >
          {HOLDINGS_LAYOUTS.map((layout) => (
            <DropdownMenuRadioItem key={layout} value={layout} onSelect={keepOpen}>
              {t(`holdings.layout.${layout}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-2xs text-muted-foreground">{t("holdings.sortLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={view.sort}
          onValueChange={(sort) => onChange({ sort: sort as HoldingsSort })}
        >
          {(["balance", "name", "activity"] as const).map((sort) => (
            <DropdownMenuRadioItem key={sort} value={sort} onSelect={keepOpen}>
              {t(`holdings.sort.${sort}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        {/* Rarer choices sit one level down; each trigger names its current setting. */}
        <DropdownMenuSub>
          <SettingsSubTrigger
            label={t("holdings.railsLabel")}
            value={
              view.rails.length === HOLDINGS_RAILS.length
                ? t("holdings.limitAll")
                : view.rails.map((rail) => t(`holdings.rail.${rail}`)).join(", ")
            }
          />
          <DropdownMenuSubContent className="w-52">
            {HOLDINGS_RAILS.map((rail) => {
              const checked = view.rails.includes(rail);
              return (
                <DropdownMenuCheckboxItem
                  key={rail}
                  checked={checked}
                  // The last rail stays on: an empty filter would list nothing.
                  disabled={checked && view.rails.length === 1}
                  onCheckedChange={(next) =>
                    onChange({
                      rails: next
                        ? HOLDINGS_RAILS.filter((entry) => entry === rail || view.rails.includes(entry))
                        : view.rails.filter((entry) => entry !== rail),
                    })
                  }
                  onSelect={keepOpen}
                >
                  {t(`holdings.rail.${rail}`)}
                </DropdownMenuCheckboxItem>
              );
            })}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <SettingsSubTrigger label={t("holdings.emptyLabel")} value={t(`holdings.emptyMode.${view.empty}`)} />
          <DropdownMenuSubContent className="w-52">
            <DropdownMenuRadioGroup
              value={view.empty}
              onValueChange={(empty) => onChange({ empty: empty as HoldingsEmpty })}
            >
              {(["fold", "show", "hide"] as const).map((mode) => (
                <DropdownMenuRadioItem key={mode} value={mode} onSelect={keepOpen}>
                  {t(`holdings.emptyMode.${mode}`)}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <SettingsSubTrigger
            label={t("holdings.limitLabel")}
            value={view.limit === 0 ? t("holdings.limitAll") : String(view.limit)}
          />
          <DropdownMenuSubContent className="w-32">
            <DropdownMenuRadioGroup
              value={String(view.limit)}
              onValueChange={(limit) => onChange({ limit: Number(limit) as HoldingsView["limit"] })}
            >
              {HOLDINGS_LIMITS.map((limit) => (
                <DropdownMenuRadioItem key={limit} value={String(limit)} onSelect={keepOpen}>
                  {limit === 0 ? t("holdings.limitAll") : String(limit)}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
      </DropdownMenuContent>
    </DropdownMenu>
  );
};

const SettingsSubTrigger = ({ label, value }: { label: string; value: string }) => (
  <DropdownMenuSubTrigger className="gap-3">
    <span className="flex-1 whitespace-nowrap">{label}</span>
    <span className="max-w-28 truncate text-xs text-muted-foreground">{value}</span>
  </DropdownMenuSubTrigger>
);
