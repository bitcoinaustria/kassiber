import * as React from "react";
import type { TFunction } from "i18next";
import {
  AlertTriangle,
  Database,
  Loader2,
  Network,
  Pencil,
  RefreshCw,
  ShieldCheck,
  ShieldOff,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { cn } from "@/lib/utils";
import { SettingsSwitchRow } from "./SettingsControls";
import {
  backendProtocolLabel,
  backendTrust,
  endpointHostLabel,
  type Backend,
  type Net,
} from "./SettingsModel";

type PrivacyFindingSeverity = "info" | "warning" | "alert";

interface PrivacyHygieneFinding {
  id: string;
  category: string;
  severity: PrivacyFindingSeverity;
  title?: string;
  detail?: string;
  evidence_level: string;
  evidence?: Record<string, unknown>;
}

interface PrivacyHygienePayload {
  summary: {
    finding_count?: number;
    highest_severity?: string;
    remote_backend_count?: number;
    off_device_ai_provider_count?: number;
    watch_only_material_wallet_count?: number;
  };
  facts?: {
    database?: { status?: string; evidence_level?: string };
  };
  findings?: PrivacyHygieneFinding[];
  limitations?: Array<{
    code?: string;
    message?: string;
    evidence_level?: string;
  }>;
}

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
      {/* dynamic key */}
      <p className="mt-1 text-sm font-medium">{t(filter.labelKey as never)}</p>
      {/* dynamic key */}
      <p className="text-xs text-muted-foreground">{t(filter.hintKey as never)}</p>
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

function findingText(
  t: TFunction<"settings">,
  finding: PrivacyHygieneFinding,
  field: "title" | "detail",
) {
  return t(`privacy.finding.${finding.id}.${field}` as never, {
    defaultValue: finding[field] ?? finding.id,
    ...(finding.evidence ?? {}),
  });
}

function limitationText(
  t: TFunction<"settings">,
  code: string | undefined,
  fallback: string | undefined,
) {
  return t(`privacy.limitation.${code || "unknown"}` as never, {
    defaultValue: fallback ?? code ?? "Unknown limitation",
  });
}

function evidenceLabel(
  t: TFunction<"settings">,
  evidenceLevel: string | undefined,
) {
  const key = evidenceLevel || "unknown";
  return t(`privacy.evidence.${key}` as never, {
    defaultValue: key,
  });
}

function severityLabel(
  t: TFunction<"settings">,
  severity: string | undefined,
) {
  const key = severity || "none";
  return t(`privacy.severity.${key}` as never, {
    defaultValue: key,
  });
}

function databaseStatusLabel(
  t: TFunction<"settings">,
  status: string | undefined,
) {
  const key = status || "unknown";
  return t(`privacy.database.${key}` as never, {
    defaultValue: key,
  });
}

function HygieneMetric({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-md border bg-background p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-1 truncate font-mono text-lg tabular-nums">{value}</p>
    </div>
  );
}

function PrivacyHygienePanel() {
  const { t } = useTranslation("settings");
  const hygieneQuery = useDaemon<PrivacyHygienePayload>(
    "ui.reports.privacy_hygiene",
    undefined,
    { refetchOnMount: "always" },
  );
  const payload = hygieneQuery.data?.data;
  const findings = payload?.findings ?? [];
  const limitations = payload?.limitations ?? [];
  const summary: PrivacyHygienePayload["summary"] = payload?.summary ?? {};
  const databaseStatus = payload?.facts?.database?.status;
  const highestSeverity = summary.highest_severity ?? "none";

  return (
    <section className="space-y-3">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold">
            {t("privacy.hygieneHeading")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("privacy.hygieneDescription")}
          </p>
        </div>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={t("privacy.hygieneRefresh")}
          onClick={() => void hygieneQuery.refetch()}
          disabled={hygieneQuery.isFetching}
        >
          <RefreshCw
            className={cn("size-3.5", hygieneQuery.isFetching && "animate-spin")}
            aria-hidden="true"
          />
        </Button>
      </div>

      {hygieneQuery.isLoading ? (
        <div className="flex items-center gap-2 rounded-md border bg-background p-3 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          {t("privacy.hygieneLoading")}
        </div>
      ) : hygieneQuery.error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
          {t("privacy.hygieneError", {
            error:
              hygieneQuery.error instanceof Error
                ? hygieneQuery.error.message
                : String(hygieneQuery.error),
          })}
        </div>
      ) : payload ? (
        <>
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            <HygieneMetric
              label={t("privacy.metric.findings")}
              value={`${summary.finding_count ?? findings.length} · ${severityLabel(
                t,
                highestSeverity,
              )}`}
            />
            <HygieneMetric
              label={t("privacy.metric.database")}
              value={databaseStatusLabel(t, databaseStatus)}
            />
            <HygieneMetric
              label={t("privacy.metric.remoteBackends")}
              value={String(summary.remote_backend_count ?? 0)}
            />
            <HygieneMetric
              label={t("privacy.metric.offDeviceAi")}
              value={String(summary.off_device_ai_provider_count ?? 0)}
            />
          </div>

          {findings.length > 0 ? (
            <div className="grid gap-2">
              {findings.map((finding) => (
                <div
                  key={finding.id}
                  className={cn(
                    "rounded-md border bg-background p-3",
                    finding.severity === "alert" &&
                      "border-destructive/30 bg-destructive/10",
                    finding.severity === "warning" &&
                      "border-amber-500/30 bg-amber-500/10",
                  )}
                >
                  <div className="flex items-start gap-2">
                    {finding.category === "storage" ? (
                      <Database
                        className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                    ) : finding.severity === "info" ? (
                      <ShieldCheck
                        className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                    ) : (
                      <AlertTriangle
                        className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
                        aria-hidden="true"
                      />
                    )}
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <p className="text-sm font-medium">
                          {findingText(t, finding, "title")}
                        </p>
                        <span className="rounded-md border px-1.5 py-0.5 text-[11px] text-muted-foreground">
                          {evidenceLabel(t, finding.evidence_level)}
                        </span>
                      </div>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {findingText(t, finding, "detail")}
                      </p>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p className="rounded-md border border-dashed bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
              {t("privacy.hygieneNoFindings")}
            </p>
          )}

          {limitations.length > 0 ? (
            <div className="space-y-1 text-xs text-muted-foreground">
              {limitations.map((limitation) => (
                <p key={limitation.code || limitation.message}>
                  {limitationText(t, limitation.code, limitation.message)}
                </p>
              ))}
            </div>
          ) : null}
        </>
      ) : null}
    </section>
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
    ? // dynamic key
      t(activeFilterEntry.labelKey as never)
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
                {/* dynamic key */}
                <p className="text-sm font-medium">{t(group.titleKey as never)}</p>
                <p className="text-xs text-muted-foreground">
                  {/* dynamic key */}
                  {t(group.subtitleKey as never)}
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

        <PrivacyHygienePanel />

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
