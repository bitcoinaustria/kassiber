import * as React from "react";
import {
  Network,
  Pencil,
  ShieldCheck,
  ShieldOff,
  type LucideIcon,
} from "lucide-react";

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
  label: string;
  hint: string;
  icon: LucideIcon;
  iconClass: string;
  barClass: string;
}> = [
  {
    id: "first",
    label: "First-party",
    hint: "On your machine or infrastructure you operate",
    icon: ShieldCheck,
    iconClass: "text-emerald-600 dark:text-emerald-400",
    barClass: "bg-emerald-500",
  },
  {
    id: "shielded",
    label: "Tor / proxy",
    hint: "IP hidden, queries still seen",
    icon: Network,
    iconClass: "text-sky-600 dark:text-sky-400",
    barClass: "bg-sky-500",
  },
  {
    id: "remote",
    label: "Third-party",
    hint: "Can observe your queries",
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
  title: string;
  subtitle: string;
  nets: Net[];
  canEdit: boolean;
}

// Group outbound network surfaces by the kind of data each one actually sees,
// which is more meaningful than a flat backend list.
export const EXPOSURE_GROUPS: ExposureGroupDef[] = [
  {
    id: "addresses",
    title: "Addresses & balances",
    subtitle: "Indexers that resolve the addresses and UTXOs you look up.",
    nets: ["BTC", "LIQUID"],
    canEdit: true,
  },
  {
    id: "lightning",
    title: "Lightning node",
    subtitle: "Reads channel and payment history from your own node.",
    nets: ["LN"],
    canEdit: true,
  },
  {
    id: "market",
    title: "Market prices",
    subtitle:
      "Offline history stays local. Live providers see the pair and coarse transaction-time blocks.",
    nets: ["FX"],
    canEdit: true,
  },
];

export function ExposurePostureBar({
  counts,
}: {
  counts: Record<ExposureFilter, number>;
}) {
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
      aria-label={`${counts.first} first-party, ${counts.shielded} Tor or proxy, ${counts.remote} third-party network surfaces`}
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
      <p className="mt-1 text-sm font-medium">{filter.label}</p>
      <p className="text-xs text-muted-foreground">{filter.hint}</p>
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
              aria-label={`Edit ${backend.name}`}
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

  const activeFilterLabel = EXPOSURE_FILTERS.find(
    (entry) => entry.id === filter,
  )?.label;

  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <h3 className="text-sm font-semibold">On screen</h3>
        <SettingsSwitchRow
          label="Blur sensitive values"
          description={
            hideSensitive
              ? "Balances, addresses, and amounts are blurred until you reveal them."
              : "Balances, addresses, and amounts are shown in full."
          }
          checked={hideSensitive}
          onCheckedChange={setHideSensitive}
        />
        <SettingsSwitchRow
          label="Clear clipboard after copy"
          description={
            clearClipboard
              ? "Copied addresses and keys are cleared from the system clipboard after 30 seconds."
              : "Copied values stay in the system clipboard until overwritten."
          }
          checked={clearClipboard}
          onCheckedChange={setClearClipboard}
        />
      </section>

      <section className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">What leaves this machine</h3>
          <p className="text-sm text-muted-foreground">
            Kassiber is local-first. Network endpoints are grouped by what each
            one can see; assistant prompt exposure is shown separately below.
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
            ? `${counts.remote} third-party endpoint${
                counts.remote === 1 ? "" : "s"
              } can see your queries. Mark your own nodes as yours, or route over Tor.`
            : aiFeaturesEnabled
              ? "No third-party network endpoints can see your queries. Review assistant providers below for prompt exposure."
              : "No third-party endpoints can see your queries."}
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
            <span>Showing {activeFilterLabel} surfaces only.</span>
            <button
              type="button"
              className="underline-offset-4 hover:underline"
              onClick={() => setFilter(null)}
            >
              Clear filter
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
                <p className="text-sm font-medium">{group.title}</p>
                <p className="text-xs text-muted-foreground">
                  {group.subtitle}
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
                  None match the {activeFilterLabel} filter.
                </p>
              )}
            </div>
          );
        })}

        <div className="space-y-2">
          <div>
            <p className="text-sm font-medium">Assistant prompts</p>
            <p className="text-xs text-muted-foreground">
              Prompt content — which can include book data — leaves only if you
              enable a remote or CLI provider.
            </p>
          </div>
          <div className="flex flex-col gap-2 rounded-md border bg-background p-3 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-sm text-muted-foreground">
              {aiFeaturesEnabled
                ? "Enabled. Local providers keep prompts on this machine; remote and CLI providers can see prompt content."
                : "Disabled. Provider settings stay saved without sending prompts."}
            </p>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="shrink-0"
              onClick={onManageAi}
            >
              Review AI providers
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}
