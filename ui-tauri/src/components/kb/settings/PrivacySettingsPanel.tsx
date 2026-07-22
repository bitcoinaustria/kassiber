import * as React from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Info,
  Loader2,
  Network,
  Pencil,
  ShieldAlert,
  ShieldCheck,
  ShieldOff,
  WalletCards,
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

type PrivacySeverity =
  | "positive"
  | "info"
  | "low"
  | "medium"
  | "high"
  | "critical";
type PrivacyRiskLevel = "none" | "low" | "medium" | "high" | "critical";
type PrivacyEvidenceLevel =
  | "ground_truth"
  | "reviewed"
  | "imported"
  | "heuristic"
  | "unavailable";

interface PrivacyFinding {
  code: string;
  severity: PrivacySeverity;
  scope: "wallet" | "transaction";
  count: number;
  impact: number;
  evidence_level: PrivacyEvidenceLevel;
  remediation: string;
  attribution: "user_wallet" | "counterparty" | "local_data";
  occurrences?: number;
}

interface PrivacyScoreSummary {
  state: string;
  wallet_count: number;
  transaction_count: number;
  risk_weight: number;
  risk_count: number;
  unknown_count: number;
  risk_level: PrivacyRiskLevel;
  finding_counts: Record<PrivacySeverity, number>;
  top_findings: PrivacyFinding[];
}

interface PrivacyCoverage {
  wallet_count: number;
  wallets_with_inventory: number;
  inventory_outputs: number;
  active_utxos: number;
  transaction_total: number;
  transaction_full: number;
  transaction_partial: number;
  transaction_not_analysable: number;
  transaction_scored: number;
}

interface PrivacyWalletScore {
  id: string;
  label: string;
  kind: string;
  state: string;
  transaction_count: number;
  scored_transaction_count: number;
  inventory_output_count: number;
  active_utxo_count: number;
  address: {
    known_address_count: number;
    reused_address_count: number;
    active_utxo_count: number;
    dust_utxo_count: number;
    script_type_counts: Record<string, number>;
  };
  risk_weight: number;
  risk_count: number;
  unknown_count: number;
  risk_level: PrivacyRiskLevel;
  finding_counts: Record<PrivacySeverity, number>;
  top_findings: PrivacyFinding[];
}

interface PrivacyTransactionScore {
  id: string;
  external_id: string;
  wallet_id: string;
  wallet_label: string;
  occurred_at: string;
  direction: string;
  asset: string;
  state: string;
  support: {
    level: string;
    reason: string | null;
    input_count: number;
    output_count: number;
    known_input_values: number;
    known_output_values: number;
  };
  risk_weight: number;
  risk_count: number;
  unknown_count: number;
  risk_level: PrivacyRiskLevel;
  finding_counts: Record<PrivacySeverity, number>;
  top_findings: PrivacyFinding[];
}

interface PrivacyHygieneData {
  profile: { id: string; label: string } | null;
  summary: PrivacyScoreSummary;
  coverage: PrivacyCoverage;
  wallets: PrivacyWalletScore[];
  transactions: PrivacyTransactionScore[];
  findings: PrivacyFinding[];
  meta: {
    local_only: boolean;
    egress: string;
    scope: string;
  };
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

function severityClass(severity: PrivacySeverity): string {
  switch (severity) {
    case "positive":
      return "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300";
    case "critical":
    case "high":
      return "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-300";
    case "medium":
      return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "low":
      return "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300";
    case "info":
    default:
      return "border-muted-foreground/20 text-muted-foreground";
  }
}

function riskClass(level: PrivacyRiskLevel): string {
  switch (level) {
    case "critical":
    case "high":
      return "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-300";
    case "medium":
      return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "low":
      return "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300";
    case "none":
    default:
      return "border-muted-foreground/20 text-muted-foreground";
  }
}

function SeverityIcon({ severity }: { severity: PrivacySeverity }) {
  if (severity === "positive") {
    return <CheckCircle2 className="size-3.5" aria-hidden="true" />;
  }
  if (severity === "critical" || severity === "high") {
    return <ShieldAlert className="size-3.5" aria-hidden="true" />;
  }
  if (severity === "medium") {
    return <AlertTriangle className="size-3.5" aria-hidden="true" />;
  }
  return <Info className="size-3.5" aria-hidden="true" />;
}

function shortTransactionRef(value: string): string {
  if (/^[0-9a-f]{64}$/i.test(value)) {
    return `${value.slice(0, 10)}...${value.slice(-6)}`;
  }
  return value || "--";
}

function RiskBadge({
  level,
  weight,
}: {
  level: PrivacyRiskLevel;
  weight: number;
}) {
  const { t } = useTranslation("settings");
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-semibold",
        riskClass(level),
      )}
    >
      <span>{t(`privacy.riskLevel.${level}` as never)}</span>
      {weight > 0 ? <span className="font-mono tabular-nums">{weight}</span> : null}
    </span>
  );
}

function FindingPill({ finding }: { finding: PrivacyFinding }) {
  const { t } = useTranslation("settings");
  return (
    <div className="flex min-w-0 items-start gap-2 rounded-md border bg-background px-3 py-2">
      <span
        className={cn(
          "mt-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded-md border",
          severityClass(finding.severity),
        )}
      >
        <SeverityIcon severity={finding.severity} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <p className="text-sm font-medium">
            {t(`privacy.findings.${finding.code}.title` as never)}
          </p>
          <span className="rounded-md border px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">
            {t(`privacy.evidence.${finding.evidence_level}` as never)}
          </span>
        </div>
        <p className="text-xs text-muted-foreground">
          {t(`privacy.findings.${finding.code}.detail` as never, {
            count: finding.count,
            occurrences: finding.occurrences ?? finding.count,
          })}
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          {t(`privacy.findings.${finding.code}.remediation` as never, {
            defaultValue: finding.remediation,
          })}
        </p>
      </div>
    </div>
  );
}

function FindingList({
  findings,
  emptyLabel,
  limit = 4,
}: {
  findings: PrivacyFinding[];
  emptyLabel: string;
  limit?: number;
}) {
  if (findings.length === 0) {
    return (
      <p className="rounded-md border border-dashed bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
        {emptyLabel}
      </p>
    );
  }
  return (
    <div className="grid gap-2">
      {findings.slice(0, limit).map((finding) => (
        <FindingPill
          key={`${finding.code}-${finding.scope}`}
          finding={finding}
        />
      ))}
    </div>
  );
}

function CoverageTile({
  label,
  value,
}: {
  label: string;
  value: number;
}) {
  return (
    <div className="rounded-md border bg-background px-3 py-2">
      <div className="font-mono text-lg font-semibold tabular-nums">{value}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  );
}

function WalletScoreRow({ wallet }: { wallet: PrivacyWalletScore }) {
  const { t } = useTranslation("settings");
  const firstFinding = wallet.top_findings[0];
  return (
    <div className="rounded-md border bg-background p-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{wallet.label}</p>
          <p className="text-xs text-muted-foreground">
            {t("privacy.scoreWalletMeta", {
              txs: wallet.transaction_count,
              utxos: wallet.active_utxo_count,
            })}
          </p>
        </div>
        <RiskBadge level={wallet.risk_level} weight={wallet.risk_weight} />
      </div>
      <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
        <span>
          {t("privacy.scoreReuse", {
            count: wallet.address.reused_address_count,
          })}
        </span>
        <span>
          {t("privacy.scoreDust", {
            count: wallet.address.dust_utxo_count,
          })}
        </span>
        {wallet.unknown_count > 0 ? (
          <span>{t("privacy.unknownCount", { count: wallet.unknown_count })}</span>
        ) : null}
      </div>
      {firstFinding ? (
        <div className="mt-2 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <span
            className={cn(
              "inline-flex size-5 shrink-0 items-center justify-center rounded-md border",
              severityClass(firstFinding.severity),
            )}
          >
            <SeverityIcon severity={firstFinding.severity} />
          </span>
          <span className="min-w-0 truncate">
            {t(`privacy.findings.${firstFinding.code}.title` as never)}
          </span>
        </div>
      ) : null}
    </div>
  );
}

function TransactionTellRow({ tx }: { tx: PrivacyTransactionScore }) {
  const { t } = useTranslation("settings");
  const firstFinding = tx.top_findings[0];
  return (
    <div className="rounded-md border bg-background p-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate font-mono text-xs">
            {shortTransactionRef(tx.external_id || tx.id)}
          </p>
          <p className="text-xs text-muted-foreground">
            {tx.wallet_label} · {t(`privacy.scoreState.${tx.state}` as never)}
          </p>
        </div>
        <RiskBadge level={tx.risk_level} weight={tx.risk_weight} />
      </div>
      {firstFinding ? (
        <div className="mt-2 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <span
            className={cn(
              "inline-flex size-5 shrink-0 items-center justify-center rounded-md border",
              severityClass(firstFinding.severity),
            )}
          >
            <SeverityIcon severity={firstFinding.severity} />
          </span>
          <span className="min-w-0 truncate">
            {t(`privacy.findings.${firstFinding.code}.title` as never)}
          </span>
        </div>
      ) : null}
    </div>
  );
}

function PrivacyHygieneSection() {
  const { t } = useTranslation("settings");
  const hygieneQuery = useDaemon<PrivacyHygieneData>(
    "ui.privacy_hygiene.snapshot",
    undefined,
    { refetchOnMount: "always" },
  );
  const hygiene =
    hygieneQuery.data?.kind === "ui.privacy_hygiene.snapshot"
      ? hygieneQuery.data.data
      : null;
  const summary = hygiene?.summary ?? null;
  const coverage = hygiene?.coverage ?? null;
  const topWallets = React.useMemo(
    () =>
      [...(hygiene?.wallets ?? [])]
        .sort(
          (a, b) =>
            b.risk_weight - a.risk_weight ||
            b.unknown_count - a.unknown_count ||
            a.label.localeCompare(b.label),
        )
        .slice(0, 4),
    [hygiene?.wallets],
  );
  const topTransactions = (hygiene?.transactions ?? []).slice(0, 4);

  return (
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-semibold">
          {t("privacy.scoreHeading")}
        </h3>
        <p className="text-sm text-muted-foreground">
          {t("privacy.scoreDescription")}
        </p>
      </div>

      {hygieneQuery.isLoading ? (
        <div className="flex items-center gap-2 rounded-md border bg-background px-3 py-4 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          {t("privacy.scoreLoading")}
        </div>
      ) : hygieneQuery.isError ? (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-3 text-sm text-red-700 dark:text-red-300">
          {hygieneQuery.error instanceof Error
            ? hygieneQuery.error.message
            : t("privacy.scoreUnavailable")}
        </div>
      ) : summary ? (
        <>
          <div className="grid gap-3 lg:grid-cols-[minmax(0,0.95fr)_minmax(0,1.05fr)]">
            <div className="rounded-md border bg-background p-4">
              <div className="flex flex-wrap items-center gap-2">
                <RiskBadge
                  level={summary.risk_level}
                  weight={summary.risk_weight}
                />
                <span className="inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium text-muted-foreground">
                  <Info className="size-3.5" aria-hidden="true" />
                  {t(`privacy.scoreState.${summary.state}` as never)}
                </span>
                <span className="inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium text-muted-foreground">
                  <ShieldCheck className="size-3.5" aria-hidden="true" />
                  {t("privacy.scoreLocalOnly")}
                </span>
              </div>
              <div className="mt-3 grid gap-2 sm:grid-cols-3">
                <CoverageTile
                  label={t("privacy.summaryRisks")}
                  value={summary.risk_count}
                />
                <CoverageTile
                  label={t("privacy.summaryUnknowns")}
                  value={summary.unknown_count}
                />
                <CoverageTile
                  label={t("privacy.summaryWallets")}
                  value={summary.wallet_count}
                />
              </div>
              <p className="mt-3 text-sm text-muted-foreground">
                {t("privacy.scoreSummary", {
                  wallets: summary.wallet_count,
                  transactions: summary.transaction_count,
                })}
              </p>
            </div>
            <div className="space-y-2">
              <p className="text-sm font-medium">{t("privacy.scoreTopFindings")}</p>
              <FindingList
                findings={summary.top_findings}
                emptyLabel={t("privacy.scoreNoFindings")}
                limit={4}
              />
            </div>
          </div>

          {coverage ? (
            <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
              <CoverageTile
                label={t("privacy.coverage.full")}
                value={coverage.transaction_full}
              />
              <CoverageTile
                label={t("privacy.coverage.partial")}
                value={coverage.transaction_partial}
              />
              <CoverageTile
                label={t("privacy.coverage.notAnalysable")}
                value={coverage.transaction_not_analysable}
              />
              <CoverageTile
                label={t("privacy.coverage.activeUtxos")}
                value={coverage.active_utxos}
              />
            </div>
          ) : null}

          <div className="grid gap-3 lg:grid-cols-2">
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <WalletCards className="size-4 text-muted-foreground" aria-hidden="true" />
                <p className="text-sm font-medium">{t("privacy.scoreWallets")}</p>
              </div>
              {topWallets.length > 0 ? (
                <div className="grid gap-2">
                  {topWallets.map((wallet) => (
                    <WalletScoreRow key={wallet.id} wallet={wallet} />
                  ))}
                </div>
              ) : (
                <p className="rounded-md border border-dashed bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
                  {t("privacy.scoreNoWallets")}
                </p>
              )}
            </div>
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <AlertTriangle className="size-4 text-muted-foreground" aria-hidden="true" />
                <p className="text-sm font-medium">{t("privacy.scoreTransactions")}</p>
              </div>
              {topTransactions.length > 0 ? (
                <div className="grid gap-2">
                  {topTransactions.map((tx) => (
                    <TransactionTellRow key={tx.id} tx={tx} />
                  ))}
                </div>
              ) : (
                <p className="rounded-md border border-dashed bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
                  {t("privacy.scoreNoTransactions")}
                </p>
              )}
            </div>
          </div>
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
      <PrivacyHygieneSection />

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
        <h3 className="text-sm font-semibold">
          {t("privacy.updatesHeading")}
        </h3>
        <SettingsSwitchRow
          label={t("privacy.updatesLabel")}
          description={
            automaticUpdateChecks
              ? t("privacy.updatesOn")
              : t("privacy.updatesOff")
          }
          checked={automaticUpdateChecks}
          onCheckedChange={setAutomaticUpdateChecks}
        />
        <p className="text-xs text-muted-foreground">
          {t("privacy.updatesTrust")}
        </p>
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
