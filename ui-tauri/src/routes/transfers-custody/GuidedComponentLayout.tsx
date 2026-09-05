/**
 * Presentational scaffolding for the guided custody-component builder.
 *
 * Pure layout: section rhythm, per-role leg identity, and the human-readable
 * movement summary. Nothing here touches the form model or the plan/apply
 * flow; amounts arrive as exact msat bigints and leave as exact strings.
 */
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Check } from "lucide-react";

import { cn } from "@/lib/utils";
import { formatCustodyMsat } from "../custodyGapsModel";
import type { GuidedLegRole } from "./guidedComponentModel";

/**
 * Role accents give each leg a stable identity while the user scans a stack of
 * otherwise identical rows: outbound sources are warm, owned sinks cool, fees
 * neutral, and the limbo roles muted-but-visible.
 */
const ROLE_ACCENT: Record<GuidedLegRole, string> = {
  source: "bg-orange-400/80 dark:bg-orange-300/70",
  destination: "bg-emerald-500/80 dark:bg-emerald-300/70",
  retained: "bg-sky-500/80 dark:bg-sky-300/70",
  fee: "bg-muted-foreground/50",
  external: "bg-violet-500/70 dark:bg-violet-300/60",
  suspense: "bg-amber-500/80 dark:bg-amber-300/70",
};

export function roleAccentClassName(role: string): string {
  return ROLE_ACCENT[role as GuidedLegRole] ?? "bg-muted-foreground/40";
}

/** Small colored dot used next to a role name. */
export function RoleDot({ role, className }: { role: string; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("inline-block size-2 shrink-0 rounded-full", roleAccentClassName(role), className)}
    />
  );
}

/** Titled section with a one-line hint and an optional trailing action. */
export function GuidedFormSection({
  title,
  hint,
  action,
  children,
  className,
}: {
  title: string;
  hint?: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("space-y-3", className)}>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="min-w-0 space-y-0.5">
          <h3 className="text-sm font-medium leading-5">{title}</h3>
          {hint ? <p className="text-xs leading-5 text-muted-foreground">{hint}</p> : null}
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      {children}
    </section>
  );
}

type MovementRole = "sources" | "destinations" | "fees" | "external" | "suspense";
export type MovementTotals = Record<MovementRole, ReadonlyMap<string, bigint>>;

function sumAmounts(amounts: ReadonlyMap<string, bigint>): bigint {
  return [...amounts.values()].reduce((total, amount) => total + amount, 0n);
}

/**
 * Reads like a ledger line — "sources = destinations + fees (+ external, suspense)"
 * — so the balance check is understandable without knowing the role taxonomy.
 * Only the not-balanced state is emphasised; a balanced movement stays quiet.
 */
export function MovementSummary({
  totals,
  balanced,
}: {
  totals: MovementTotals;
  balanced: boolean;
}) {
  const { t } = useTranslation("review");
  const sources = sumAmounts(totals.sources);
  const sinks = sumAmounts(totals.destinations) + sumAmounts(totals.fees) + sumAmounts(totals.external) + sumAmounts(totals.suspense);
  // Nothing typed yet: stay silent rather than flag an empty form as unbalanced.
  if (sources === 0n && sinks === 0n) return null;
  const difference = sinks - sources;
  const assets = new Set(Object.values(totals).flatMap((amounts) => [...amounts.keys()]));
  const defaultAsset = totals.sources.keys().next().value ?? "BTC";
  const groups: Array<{ key: string; label: string; value: ReadonlyMap<string, bigint>; always: boolean }> = [
    { key: "sources", label: t("swap.components.form.balance.sources"), value: totals.sources, always: true },
    { key: "destinations", label: t("swap.components.form.balance.destinations"), value: totals.destinations, always: true },
    { key: "fees", label: t("swap.components.form.balance.fees"), value: totals.fees, always: true },
    { key: "external", label: t("swap.components.form.balance.external"), value: totals.external, always: false },
    { key: "suspense", label: t("swap.components.form.balance.suspense"), value: totals.suspense, always: false },
  ];
  const visibleGroups = groups.filter((group) => group.always || sumAmounts(group.value) !== 0n);

  return (
    <div
      className={cn(
        "rounded-lg border px-4 py-3 text-sm",
        balanced ? "bg-muted/30" : "border-amber-500/40 bg-amber-500/10",
      )}
      role="status"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
        <p className="text-2xs font-medium tracking-[0.14em] text-muted-foreground uppercase">
          {t("swap.components.form.balance.title")}
        </p>
        {balanced ? (
          <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
            <Check className="size-3.5" aria-hidden="true" />
            {t("swap.components.form.balance.balanced")}
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-amber-900 dark:text-amber-100">
            <AlertTriangle className="size-3.5" aria-hidden="true" />
            {t("swap.components.form.balance.unbalanced")}
          </span>
        )}
      </div>
      <dl className="mt-2 flex flex-wrap items-baseline gap-x-2 gap-y-1 font-mono text-sm tabular-nums">
        {visibleGroups.map((group, index) => (
          <div key={group.key} className="flex items-baseline gap-2">
            {index > 0 ? (
              <span aria-hidden="true" className="text-muted-foreground">
                {index === 1 ? "=" : "+"}
              </span>
            ) : null}
            <dt className="font-sans text-xs text-muted-foreground">{group.label}</dt>
            <dd>{group.value.size
              ? [...group.value].map(([asset, amount]) => formatCustodyMsat(amount.toString(), asset)).join(" + ")
              : formatCustodyMsat("0", defaultAsset)}</dd>
          </div>
        ))}
      </dl>
      {!balanced && sources > 0n ? (
        <p className="mt-2 text-xs text-amber-900 dark:text-amber-100">
          {t("swap.components.form.balance.difference", {
            // BTC and LBTC share msat exposure; a mixed-asset residual is not
            // evidence that any particular asset supplied the difference.
            amount: assets.size > 1 ? `${difference} msat` : formatCustodyMsat(difference.toString(), defaultAsset),
          })}
        </p>
      ) : null}
    </div>
  );
}

/** Compact status banner used for the applied result and action errors. */
export function GuidedNotice({
  tone,
  children,
}: {
  tone: "success" | "error";
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "whitespace-pre-wrap rounded-md border px-3 py-2 text-sm",
        tone === "success"
          ? "border-emerald-200 bg-emerald-50 text-emerald-950 dark:border-emerald-400/30 dark:bg-emerald-950/30 dark:text-emerald-100"
          : "border-destructive/40 bg-destructive/10 text-destructive",
      )}
    >
      {children}
    </div>
  );
}
