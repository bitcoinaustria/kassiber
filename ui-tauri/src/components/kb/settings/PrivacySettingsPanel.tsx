import * as React from "react";
import {
  Network,
  Pencil,
  ShieldCheck,
  ShieldOff,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { SettingsSwitchRow } from "./SettingsControls";
import {
  backendProtocolLabel,
  backendTrust,
  endpointHostLabel,
  type Backend,
  type Net,
} from "./SettingsModel";

export type ExposureFilter = "first" | "shielded" | "remote";

export const EXPOSURE_FILTERS: Array<{
  id: ExposureFilter;
  labelKey: string;
  hintKey: string;
  icon: LucideIcon;
  iconClass: string;
  barClass: string;
}> = [
  {
    id: "first",
    labelKey: "privacy.exposure.firstParty",
    hintKey: "privacy.exposure.firstPartyHint",
    icon: ShieldCheck,
    iconClass: "text-emerald-600 dark:text-emerald-400",
    barClass: "bg-emerald-500",
  },
  {
    id: "shielded",
    labelKey: "privacy.exposure.shielded",
    hintKey: "privacy.exposure.shieldedHint",
    icon: Network,
    iconClass: "text-sky-600 dark:text-sky-400",
    barClass: "bg-sky-500",
  },
  {
    id: "remote",
    labelKey: "privacy.exposure.thirdParty",
    hintKey: "privacy.exposure.thirdPartyHint",
    icon: ShieldOff,
    iconClass: "text-amber-600 dark:text-amber-400",
    barClass: "bg-amber-500",
  },
];

export function backendExposureFilter(backend: Backend): ExposureFilter {
  const posture = backendTrust(backend).posture;
  if (posture === "on-device" || posture === "self-hosted") return "first";
  if (posture === "shielded") return "shielded";
  return "remote";
}

export interface ExposureGroupDef {
  id: string;
  titleKey: string;
  subtitleKey: string;
  nets: Net[];
  canEdit: boolean;
}

// Group outbound network surfaces by the kind of data each one actually sees,
// which is more meaningful than a flat backend list.
export const EXPOSURE_GROUPS: ExposureGroupDef[] = [
  {
    id: "addresses",
    titleKey: "privacy.group.addressesTitle",
    subtitleKey: "privacy.group.addressesSubtitle",
    nets: ["BTC", "LIQUID"],
    canEdit: true,
  },
  {
    id: "lightning",
    titleKey: "privacy.group.lightningTitle",
    subtitleKey: "privacy.group.lightningSubtitle",
    nets: ["LN"],
    canEdit: true,
  },
  {
    id: "market",
    titleKey: "privacy.group.marketTitle",
    subtitleKey: "privacy.group.marketSubtitle",
    nets: ["FX"],
    canEdit: true,
  },
];

export function ExposurePostureBar({
  counts,
}: {
  counts: Record<ExposureFilter, number>;
}) {
  const { t } = useTranslation("settings");
  const total = counts.first + counts.shielded + counts.remote;
  if (total === 0) {
    return (
      <div
        className="h-2 w-full overflow-hidden rounded-full bg-muted"
        aria-hidden="true"
      />
    );
  }
  return (
    <div
      className="flex h-2 w-full overflow-hidden rounded-full bg-muted"
      role="img"
      aria-label={t("privacy.postureBarAria", {
        first: counts.first,
        shielded: counts.shielded,
        remote: counts.remote,
      })}
    >
      {EXPOSURE_FILTERS.map((filter) =>
        counts[filter.id] > 0 ? (
          <div
            key={filter.id}
            className={filter.barClass}
            style={{ width: `${(counts[filter.id] / total) * 100}%` }}
          />
        ) : null,
      )}
    </div>
  );
}

export function ExposureFilterTile({
  filter,
  count,
  active,
  onClick,
}: {
  filter: (typeof EXPOSURE_FILTERS)[number];
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  const { t } = useTranslation("settings");
  const Icon = filter.icon;
  const dim = filter.id === "remote" && count === 0;
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "rounded-md border bg-background p-3 text-left transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active && "border-foreground/30 bg-muted ring-1 ring-foreground/10",
      )}
    >
      <div className="flex items-center gap-2">
        <Icon
          className={cn("size-4", dim ? "text-muted-foreground" : filter.iconClass)}
          aria-hidden="true"
        />
        <span className="font-mono text-lg tabular-nums">{count}</span>
      </div>
      <p className="mt-1 text-sm font-medium">{t(filter.labelKey)}</p>
      <p className="text-xs text-muted-foreground">{t(filter.hintKey)}</p>
    </button>
  );
}

export function ExposureEndpointRow({
  backend,
  canEdit,
  onEdit,
}: {
  backend: Backend;
  canEdit: boolean;
  onEdit: () => void;
}) {
  const { t } = useTranslation("settings");
  const trust = backendTrust(backend);
  const TrustIcon = trust.icon;
  return (
    <div className="rounded-md border bg-background p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{backend.name}</p>
          <p className="truncate text-xs text-muted-foreground">
            {backendProtocolLabel(backend)} · {endpointHostLabel(backend.url)}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-1.5">
          <span
            className={cn(
              "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium",
              trust.className,
            )}
          >
            <TrustIcon className="size-3" aria-hidden="true" />
            {trust.label}
          </span>
          {canEdit ? (
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              aria-label={t("privacy.editEndpoint", { name: backend.name })}
              onClick={onEdit}
            >
              <Pencil className="size-3.5" aria-hidden="true" />
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function PrivacySettingsPanel({
  hideSensitive,
  setHideSensitive,
  clearClipboard,
  setClearClipboard,
  backends,
  aiFeaturesEnabled,
  onEditBackend,
  onManageAi,
  onManageMarketData,
}: {
  hideSensitive: boolean;
  setHideSensitive: (value: boolean) => void;
  clearClipboard: boolean;
  setClearClipboard: (value: boolean) => void;
  backends: Backend[];
  aiFeaturesEnabled: boolean;
  onEditBackend: (backend: Backend) => void;
  onManageAi: () => void;
  onManageMarketData: () => void;
}) {
  const { t } = useTranslation("settings");
  const [filter, setFilter] = React.useState<ExposureFilter | null>(null);

  // Only enabled backends actually send traffic off the machine.
  const enabled = React.useMemo(
    () => backends.filter((backend) => backend.on),
    [backends],
  );
  const counts = React.useMemo(() => {
    const next: Record<ExposureFilter, number> = {
      first: 0,
      shielded: 0,
      remote: 0,
    };
    enabled.forEach((backend) => {
      next[backendExposureFilter(backend)] += 1;
    });
    return next;
  }, [enabled]);

  const activeFilterEntry = EXPOSURE_FILTERS.find(
    (entry) => entry.id === filter,
  );
  const activeFilterLabel = activeFilterEntry
    ? t(activeFilterEntry.labelKey)
    : undefined;

  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <h3 className="text-sm font-semibold">
          {t("privacy.onScreenHeading")}
        </h3>
        <SettingsSwitchRow
          label={t("privacy.blurLabel")}
          description={
            hideSensitive ? t("privacy.blurOn") : t("privacy.blurOff")
          }
          checked={hideSensitive}
          onCheckedChange={setHideSensitive}
        />
        <SettingsSwitchRow
          label={t("privacy.clipboardLabel")}
          description={
            clearClipboard
              ? t("privacy.clipboardOn")
              : t("privacy.clipboardOff")
          }
          checked={clearClipboard}
          onCheckedChange={setClearClipboard}
        />
      </section>

      <section className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">
            {t("privacy.leavesHeading")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("privacy.leavesDescription")}
          </p>
        </div>

        <ExposurePostureBar counts={counts} />
        <p
          className={cn(
            "text-xs",
            counts.remote > 0
              ? "text-amber-600 dark:text-amber-400"
              : "text-muted-foreground",
          )}
        >
          {counts.remote > 0
            ? t("privacy.thirdPartyWarning", { count: counts.remote })
            : aiFeaturesEnabled
              ? t("privacy.noThirdPartyWithAi")
              : t("privacy.noThirdParty")}
        </p>

        <div className="grid gap-2 sm:grid-cols-3">
          {EXPOSURE_FILTERS.map((entry) => (
            <ExposureFilterTile
              key={entry.id}
              filter={entry}
              count={counts[entry.id]}
              active={filter === entry.id}
              onClick={() =>
                setFilter((current) => (current === entry.id ? null : entry.id))
              }
            />
          ))}
        </div>

        {filter ? (
          <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
            <span>{t("privacy.showingFilter", { label: activeFilterLabel })}</span>
            <button
              type="button"
              className="underline-offset-4 hover:underline"
              onClick={() => setFilter(null)}
            >
              {t("privacy.clearFilter")}
            </button>
          </div>
        ) : null}

        {EXPOSURE_GROUPS.map((group) => {
          const all = enabled.filter((backend) =>
            group.nets.includes(backend.net),
          );
          if (all.length === 0) return null;
          const rows = filter
            ? all.filter((backend) => backendExposureFilter(backend) === filter)
            : all;
          return (
            <div key={group.id} className="space-y-2">
              <div>
                <p className="text-sm font-medium">{t(group.titleKey)}</p>
                <p className="text-xs text-muted-foreground">
                  {t(group.subtitleKey)}
                </p>
              </div>
              {rows.length > 0 ? (
                <div className="grid gap-2">
                  {rows.map((backend) => (
                    <ExposureEndpointRow
                      key={backend.id}
                      backend={backend}
                      canEdit={group.canEdit}
                      onEdit={() =>
                        group.id === "market"
                          ? onManageMarketData()
                          : onEditBackend(backend)
                      }
                    />
                  ))}
                </div>
              ) : (
                <p className="rounded-md border border-dashed bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
                  {t("privacy.noneMatchFilter", { label: activeFilterLabel })}
                </p>
              )}
            </div>
          );
        })}

        <div className="space-y-2">
          <div>
            <p className="text-sm font-medium">
              {t("privacy.assistantHeading")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("privacy.assistantDescription")}
            </p>
          </div>
          <div className="flex flex-col gap-2 rounded-md border bg-background p-3 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-sm text-muted-foreground">
              {aiFeaturesEnabled
                ? t("privacy.assistantEnabled")
                : t("privacy.assistantDisabled")}
            </p>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="shrink-0"
              onClick={onManageAi}
            >
              {t("privacy.reviewAi")}
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}
