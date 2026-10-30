import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useOfflineMode } from "@/lib/offlineMode";
import { cn } from "@/lib/utils";
import { SettingsGroup, SettingsRow, SettingsSelect } from "./SettingsControls";
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
  /** How the endpoint's posture reads: OK, neutral, or a warning. */
  toneClass: string;
}> = [
  {
    id: "first",
    labelKey: "privacy.exposure.firstParty",
    hintKey: "privacy.exposure.firstPartyHint",
    toneClass: "text-emerald-600 dark:text-emerald-400",
  },
  {
    id: "shielded",
    labelKey: "privacy.exposure.shielded",
    hintKey: "privacy.exposure.shieldedHint",
    toneClass: "text-muted-foreground",
  },
  {
    id: "remote",
    labelKey: "privacy.exposure.thirdParty",
    hintKey: "privacy.exposure.thirdPartyHint",
    toneClass: "text-amber-600 dark:text-amber-400",
  },
];

/** Stands in for "no filter" in the Show select. */
const ALL_ENDPOINTS = "all";

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

export function ExposureEndpointRow({
  backend,
  canEdit,
  onEdit,
}: {
  backend: Backend;
  canEdit: boolean;
  onEdit: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const trust = backendTrust(backend);
  const tone = EXPOSURE_FILTERS.find(
    (entry) => entry.id === backendExposureFilter(backend),
  )?.toneClass;
  return (
    <SettingsRow
      label={backend.name}
      description={`${backendProtocolLabel(backend)} · ${endpointHostLabel(backend.url)}`}
    >
      <span className={cn("text-sm", tone)} title={trust.note}>
        {trust.label}
      </span>
      {canEdit ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          aria-label={t("privacy.editEndpoint", { name: backend.name })}
          onClick={onEdit}
        >
          {t("common:actions.edit")}
        </Button>
      ) : null}
    </SettingsRow>
  );
}

export function PrivacySettingsPanel({
  hideSensitive,
  setHideSensitive,
  clearClipboard,
  setClearClipboard,
  automaticUpdateChecks,
  setAutomaticUpdateChecks,
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
  automaticUpdateChecks: boolean;
  setAutomaticUpdateChecks: (value: boolean) => void;
  backends: Backend[];
  aiFeaturesEnabled: boolean;
  onEditBackend: (backend: Backend) => void;
  onManageAi: () => void;
  onManageMarketData: () => void;
}) {
  const { t } = useTranslation("settings");
  const [filter, setFilter] = React.useState<ExposureFilter | null>(null);
  const offlineMode = useOfflineMode();

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
    ? // dynamic key
      t(activeFilterEntry.labelKey as never)
    : undefined;

  return (
    <div className="space-y-6">
      <SettingsGroup
        title={t("privacy.offlineHeading")}
        footer={
          offlineMode.environmentBlocked && !offlineMode.offline
            ? t("privacy.offlineEnvironment")
            : null
        }
      >
        <SettingsRow
          label={t("privacy.offlineLabel")}
          description={
            offlineMode.offline ? t("privacy.offlineOn") : t("privacy.offlineOff")
          }
          htmlFor="settings-offline"
        >
          <Switch
            id="settings-offline"
            checked={offlineMode.offline}
            onCheckedChange={offlineMode.setOffline}
            disabled={!offlineMode.known || offlineMode.pending}
          />
        </SettingsRow>
        {offlineMode.failed ? (
          <p className="px-4 py-3 text-sm text-destructive" role="alert">
            {t("privacy.offlineFailed")}
          </p>
        ) : null}
      </SettingsGroup>

      <SettingsGroup title={t("privacy.onScreenHeading")}>
        <SettingsRow
          label={t("privacy.blurLabel")}
          description={hideSensitive ? t("privacy.blurOn") : t("privacy.blurOff")}
          htmlFor="settings-blur"
        >
          <Switch
            id="settings-blur"
            checked={hideSensitive}
            onCheckedChange={setHideSensitive}
          />
        </SettingsRow>
        <SettingsRow
          label={t("privacy.clipboardLabel")}
          description={
            clearClipboard ? t("privacy.clipboardOn") : t("privacy.clipboardOff")
          }
          htmlFor="settings-clipboard"
        >
          <Switch
            id="settings-clipboard"
            checked={clearClipboard}
            onCheckedChange={setClearClipboard}
          />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup
        title={t("privacy.updatesHeading")}
        footer={t("privacy.updatesTrust")}
      >
        <SettingsRow
          label={t("privacy.updatesLabel")}
          description={
            automaticUpdateChecks ? t("privacy.updatesOn") : t("privacy.updatesOff")
          }
          htmlFor="settings-update-checks"
        >
          <Switch
            id="settings-update-checks"
            checked={automaticUpdateChecks}
            onCheckedChange={setAutomaticUpdateChecks}
          />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup
        title={t("privacy.leavesHeading")}
        footer={t("privacy.leavesDescription")}
      >
        <p
          className={cn(
            "px-4 py-3 text-sm",
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
        <SettingsRow
          label={t("privacy.assistantHeading")}
          description={
            aiFeaturesEnabled
              ? t("privacy.assistantEnabled")
              : t("privacy.assistantDisabled")
          }
        >
          <Button type="button" size="sm" variant="outline" onClick={onManageAi}>
            {t("privacy.reviewAi")}
          </Button>
        </SettingsRow>
        <SettingsRow
          label={t("privacy.filterLabel")}
          description={
            activeFilterEntry
              ? // dynamic key
                t(activeFilterEntry.hintKey as never)
              : t("privacy.filterAllHint")
          }
          htmlFor="settings-exposure-filter"
        >
          <SettingsSelect
            value={filter ?? ALL_ENDPOINTS}
            onValueChange={(value) =>
              setFilter(value === ALL_ENDPOINTS ? null : (value as ExposureFilter))
            }
            options={[
              {
                value: ALL_ENDPOINTS,
                label: t("privacy.filterAll", { count: enabled.length }),
              },
              ...EXPOSURE_FILTERS.map((entry) => ({
                value: entry.id,
                label: t("privacy.filterOption", {
                  // dynamic key
                  label: t(entry.labelKey as never),
                  count: counts[entry.id],
                }),
              })),
            ]}
            triggerProps={{
              id: "settings-exposure-filter",
              "data-exposure-filter": filter ?? ALL_ENDPOINTS,
            }}
          />
        </SettingsRow>
      </SettingsGroup>

      {EXPOSURE_GROUPS.map((group) => {
        const all = enabled.filter((backend) => group.nets.includes(backend.net));
        if (all.length === 0) return null;
        const rows = filter
          ? all.filter((backend) => backendExposureFilter(backend) === filter)
          : all;
        return (
          <SettingsGroup
            key={group.id}
            // dynamic key
            title={t(group.titleKey as never)}
            // dynamic key
            footer={t(group.subtitleKey as never)}
          >
            {rows.length > 0 ? (
              rows.map((backend) => (
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
              ))
            ) : (
              <p className="px-4 py-3 text-sm text-muted-foreground">
                {t("privacy.noneMatchFilter", { label: activeFilterLabel })}
              </p>
            )}
          </SettingsGroup>
        );
      })}
    </div>
  );
}
