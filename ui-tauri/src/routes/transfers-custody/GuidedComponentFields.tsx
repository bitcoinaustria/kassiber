/** Presentational fields and the daemon-resolved review for the guided builder. */
import type { ReactNode } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { ArrowRight, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { formatCustodyMsat } from "../custodyGapsModel";
import { custodyRoleLabel, custodyBackendIssueText, type CustodyValidationIssue } from "./custodyComponentIssues";
import { RoleDot, roleAccentClassName } from "./GuidedComponentLayout";
import { type CustodyComponentInput, type GuidedAllocationForm, type GuidedLegForm, type GuidedLegRole, type GuidedLocationKind } from "./guidedComponentModel";

const GUIDED_FORM_ROLES: readonly GuidedLegRole[] = [
  "source",
  "destination",
  "fee",
  "retained",
  "external",
  "suspense",
];

const LOCATION_KINDS: readonly GuidedLocationKind[] = [
  "transaction",
  "wallet",
  "untracked",
];

interface PlannedComponent extends CustodyComponentInput {
  id?: string;
  effective_state?: string;
  validation?: { issues?: CustodyValidationIssue[] };
}

export interface ComponentPlanResult {
  workspace_id?: string;
  profile_id?: string;
  components?: PlannedComponent[];
  component?: PlannedComponent;
  input_version: number;
  summary?: { count: number; active: number; draft: number };
}

const fieldClassName = "space-y-1.5 min-w-0";
const monoInputClassName = "font-mono tabular-nums";

export function GuidedLegRow({
  leg,
  ordinal,
  canRemove,
  conversionMode,
  onChange,
  onRemove,
}: {
  leg: GuidedLegForm;
  /** Zero-based position in the component; shown as "Leg n" and reused by allocations. */
  ordinal?: number;
  canRemove: boolean;
  conversionMode: boolean;
  onChange: (patch: Partial<GuidedLegForm>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation("review");
  const isSuspense = leg.role === "suspense";
  const isManual = !isSuspense && leg.locationMode === "manual";
  const isOrigin = !isSuspense && leg.locationMode === "origin";
  const originNeedsTime =
    isOrigin && !leg.origin?.transactionId;
  const showOccurredAt =
    isSuspense ||
    originNeedsTime ||
    (leg.locationMode === "manual" &&
      (leg.locationKind === "untracked" ||
        leg.locationKind === "wallet"));

  const reference = isManual ? (
    leg.locationKind === "transaction" ? (
      <div className={cn(fieldClassName, "lg:col-span-7")}>
        <Label htmlFor={`${leg.key}-transactionRef`}>{t("swap.components.form.leg.transactionRef")}</Label>
        <Input id={`${leg.key}-transactionRef`}
          className={monoInputClassName}
          value={leg.transactionRef}
          spellCheck={false}
          placeholder={t("swap.components.form.leg.transactionRefPlaceholder")}
          onChange={(event) => onChange({ transactionRef: event.target.value })}
        />
      </div>
    ) : leg.locationKind === "wallet" ? (
      <div className={cn(fieldClassName, "lg:col-span-7")}>
        <Label htmlFor={`${leg.key}-walletRef`}>{t("swap.components.form.leg.walletRef")}</Label>
        <Input id={`${leg.key}-walletRef`}
          value={leg.walletRef}
          spellCheck={false}
          placeholder={t("swap.components.form.leg.walletRefPlaceholder")}
          onChange={(event) => onChange({ walletRef: event.target.value })}
        />
      </div>
    ) : (
      <div className={cn(fieldClassName, "lg:col-span-7")}>
        <Label htmlFor={`${leg.key}-untrackedWallet`}>{t("swap.components.form.leg.untrackedWallet")}</Label>
        <Input id={`${leg.key}-untrackedWallet`}
          value={leg.untrackedWallet}
          spellCheck={false}
          placeholder={t("swap.components.form.leg.untrackedWalletPlaceholder")}
          onChange={(event) =>
            onChange({ untrackedWallet: event.target.value })
          }
        />
      </div>
    )
  ) : null;

  const occurredAt = showOccurredAt ? (
    <div className={cn(fieldClassName, "lg:col-span-5")}>
      <Label htmlFor={`${leg.key}-occurredAt`}>{t("swap.components.form.leg.occurredAt")}</Label>
      <Input id={`${leg.key}-occurredAt`}
        className={monoInputClassName}
        type="datetime-local"
        step={1}
        value={leg.occurredAt}
        onChange={(event) => onChange({ occurredAt: event.target.value })}
      />
    </div>
  ) : null;

  return (
    <div className="relative rounded-lg border bg-card/40 p-4 pl-5">
      <span
        aria-hidden="true"
        className={cn("absolute inset-y-3 left-0 w-1 rounded-r", roleAccentClassName(leg.role))}
      />
      <div className="flex items-center justify-between gap-3">
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <RoleDot role={leg.role} />
          <span className="font-mono">
            {ordinal === undefined
              ? custodyRoleLabel(t, leg.role)
              : t("swap.components.form.leg.ordinal", { index: ordinal + 1 })}
          </span>
          {ordinal !== undefined ? (
            <>
              <span aria-hidden="true">·</span>
              <span>{custodyRoleLabel(t, leg.role)}</span>
            </>
          ) : null}
        </p>
        {canRemove ? (
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            className="-mr-2 -mt-1 text-muted-foreground hover:text-foreground"
            aria-label={t("swap.components.form.removeLeg")}
            title={t("swap.components.form.removeLeg")}
            onClick={onRemove}
          >
            <Trash2 />
          </Button>
        ) : null}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-12">
        <div className={cn(fieldClassName, "lg:col-span-3")}>
          <Label htmlFor={`${leg.key}-role`}>{t("swap.components.form.leg.role")}</Label>
          <Select
            value={leg.role}
            onValueChange={(value) => onChange({ role: value as GuidedLegRole })}
          >
            <SelectTrigger id={`${leg.key}-role`} className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {GUIDED_FORM_ROLES.map((role) => (
                <SelectItem key={role} value={role}>
                  <RoleDot role={role} />
                  {custodyRoleLabel(t, role)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="grid grid-cols-[minmax(0,1fr)_5.5rem] gap-2 lg:col-span-4">
          <div className={fieldClassName}>
            <Label htmlFor={`${leg.key}-amount`}>{t("swap.components.form.leg.amount")}</Label>
            <Input id={`${leg.key}-amount`}
              className={monoInputClassName}
              inputMode="decimal"
              value={leg.amountBtc}
              spellCheck={false}
              placeholder="0.00000000"
              onChange={(event) => onChange({ amountBtc: event.target.value })}
            />
          </div>
          <div className={fieldClassName}>
            <Label htmlFor={`${leg.key}-asset`}>{t("swap.components.form.leg.asset")}</Label>
            <Input id={`${leg.key}-asset`}
              className={cn(monoInputClassName, "uppercase")}
              value={leg.asset}
              spellCheck={false}
              autoCapitalize="characters"
              onChange={(event) => onChange({ asset: event.target.value })}
            />
          </div>
        </div>

        {isManual ? (
          <div className={cn(fieldClassName, "lg:col-span-5")}>
            <Label htmlFor={`${leg.key}-location`}>{t("swap.components.form.leg.location")}</Label>
            <Select
              value={leg.locationKind}
              onValueChange={(value) =>
                onChange({ locationKind: value as GuidedLocationKind })
              }
            >
              <SelectTrigger id={`${leg.key}-location`} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LOCATION_KINDS.map((kind) => (
                  <SelectItem key={kind} value={kind}>
                    {t(`swap.components.form.leg.locationKind.${kind}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}

        {isOrigin ? (
          <div className="flex min-w-0 flex-col justify-end gap-1.5 lg:col-span-5">
            <span className="text-sm font-medium leading-none">
              {t("swap.components.form.leg.existingLocation")}
            </span>
            <div className="flex h-9 min-w-0 items-center justify-between gap-2 rounded-md border bg-muted/40 pl-3 pr-1 text-xs">
              <span className="min-w-0 truncate text-muted-foreground" title={leg.origin?.transactionId ?? leg.origin?.walletId ?? undefined}>
                {leg.origin?.transactionId
                  ? t("swap.components.form.leg.originTransaction", {
                      id: shortId(leg.origin.transactionId),
                    })
                  : leg.origin?.walletId ? t("swap.components.form.leg.originWallet", {
                      id: shortId(leg.origin.walletId),
                    }) : t("swap.components.form.leg.originLocation", { rail: leg.origin?.rail ?? "" })}
              </span>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-7 shrink-0 px-2 text-xs"
                onClick={() => onChange({ locationMode: "manual", origin: null })}
              >
                {t("swap.components.form.leg.changeLocation")}
              </Button>
            </div>
          </div>
        ) : null}

        {reference}
        {occurredAt}

        {conversionMode ? (
          <>
            <div className={cn(fieldClassName, "lg:col-span-4")}>
              <Label htmlFor={`${leg.key}-valuationUnit`}>{t("swap.components.form.leg.valuationUnit")}</Label>
              <Input id={`${leg.key}-valuationUnit`}
                className={monoInputClassName}
                value={leg.valuationUnit}
                spellCheck={false}
                placeholder={t("swap.components.form.leg.valuationUnitPlaceholder")}
                onChange={(event) => onChange({ valuationUnit: event.target.value })}
              />
            </div>
            <div className={cn(fieldClassName, "lg:col-span-4")}>
              <Label htmlFor={`${leg.key}-valuationAmount`}>{t("swap.components.form.leg.valuationAmount")}</Label>
              <Input id={`${leg.key}-valuationAmount`}
                className={monoInputClassName}
                inputMode="numeric"
                value={leg.valuationAmount}
                spellCheck={false}
                placeholder={t("swap.components.form.leg.valuationAmountPlaceholder")}
                onChange={(event) => onChange({ valuationAmount: event.target.value })}
              />
            </div>
          </>
        ) : null}
      </div>

      {isSuspense ? (
        <p className="mt-3 text-xs leading-5 text-muted-foreground">
          {t("swap.components.form.leg.suspenseHint")}
        </p>
      ) : null}
    </div>
  );
}

function shortId(value: string): string {
  if (value.length <= 18) return value;
  return `${value.slice(0, 10)}…${value.slice(-6)}`;
}

/** "2 · Destination · 0.99 BTC" — the amount is the user's own exact string. */
function legDisplayLabel(
  t: TFunction<"review">,
  leg: GuidedLegForm,
  index: number,
): string {
  const amount = leg.amountBtc.trim();
  const base = `${index + 1} · ${custodyRoleLabel(t, leg.role)}`;
  return amount ? `${base} · ${amount} ${leg.asset.trim() || "BTC"}` : base;
}

export function AllocationRow({
  allocation,
  conversionMode,
  legs,
  sourceLegs,
  sinkLegs,
  onChange,
  onRemove,
}: {
  allocation: GuidedAllocationForm;
  conversionMode: boolean;
  legs: GuidedLegForm[];
  sourceLegs: GuidedLegForm[];
  sinkLegs: GuidedLegForm[];
  onChange: (patch: Partial<GuidedAllocationForm>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation("review");
  return (
    <div
      className={cn(
        "grid gap-3 rounded-lg border p-3 sm:grid-cols-2 lg:items-end",
        conversionMode
          ? "lg:grid-cols-[minmax(0,1.3fr)_auto_minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1fr)_auto]"
          : "lg:grid-cols-[minmax(0,1.3fr)_auto_minmax(0,1.3fr)_minmax(0,1fr)_auto]",
      )}
    >
      <div className={fieldClassName}>
        <Label htmlFor={`${allocation.key}-source`}>{t("swap.components.form.allocations.source")}</Label>
        <Select
          value={allocation.sourceKey}
          onValueChange={(value) => onChange({ sourceKey: value })}
        >
          <SelectTrigger id={`${allocation.key}-source`} className="w-full">
            <SelectValue placeholder={t("swap.components.form.allocations.selectLeg")} />
          </SelectTrigger>
          <SelectContent>
            {sourceLegs.map((leg) => (
              <SelectItem key={leg.key} value={leg.key}>
                <RoleDot role={leg.role} />
                {legDisplayLabel(t, leg, legs.indexOf(leg))}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <ArrowRight
        aria-hidden="true"
        className="hidden size-4 self-end pb-0 text-muted-foreground lg:mb-2.5 lg:block"
      />
      <div className={fieldClassName}>
        <Label htmlFor={`${allocation.key}-sink`}>{t("swap.components.form.allocations.sink")}</Label>
        <Select
          value={allocation.sinkKey}
          onValueChange={(value) => onChange({ sinkKey: value })}
        >
          <SelectTrigger id={`${allocation.key}-sink`} className="w-full">
            <SelectValue placeholder={t("swap.components.form.allocations.selectLeg")} />
          </SelectTrigger>
          <SelectContent>
            {sinkLegs.map((leg) => (
              <SelectItem key={leg.key} value={leg.key}>
                <RoleDot role={leg.role} />
                {legDisplayLabel(t, leg, legs.indexOf(leg))}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className={fieldClassName}>
        <Label htmlFor={`${allocation.key}-sourceAmount`}>{t(conversionMode ? "swap.components.form.allocations.sourceAmount" : "swap.components.form.allocations.amount")}</Label>
        <Input id={`${allocation.key}-sourceAmount`}
          className={monoInputClassName}
          inputMode="decimal"
          value={allocation.amountBtc}
          spellCheck={false}
          placeholder="0.00000000"
          onChange={(event) => onChange({ amountBtc: event.target.value, ...(!conversionMode ? { sinkAmountBtc: event.target.value } : {}) })}
        />
      </div>
      {conversionMode ? <div className={fieldClassName}>
        <Label htmlFor={`${allocation.key}-sinkAmount`}>{t("swap.components.form.allocations.sinkAmount")}</Label>
        <Input id={`${allocation.key}-sinkAmount`} className={monoInputClassName} inputMode="decimal" value={allocation.sinkAmountBtc} spellCheck={false}
          placeholder="0.00000000" onChange={(event) => onChange({ sinkAmountBtc: event.target.value })} />
      </div> : null}
      <Button
        type="button"
        size="icon"
        variant="ghost"
        className="justify-self-end text-muted-foreground hover:text-foreground sm:col-span-2 lg:col-span-1"
        onClick={onRemove}
        aria-label={t("swap.components.form.allocations.remove")}
        title={t("swap.components.form.allocations.remove")}
      >
        <Trash2 />
      </Button>
    </div>
  );
}

/** One "label: value" line inside the review panel; values keep their exact text. */
function ReviewFact({ label, children, mono = false }: { label: string; children: ReactNode; mono?: boolean }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={cn("min-w-0 break-all", mono && "font-mono")}>{children}</dd>
    </>
  );
}

/**
 * The daemon-resolved plan, shown before the explicit confirmation. Every
 * accounting fact the daemon will persist stays visible — exact amounts, full
 * ids, the untouched RFC 3339 time, rail/network scope, conservation unit,
 * valuations, notes, and open validation issues — but each is labelled rather
 * than dumped as a dotted token string.
 */
export function GuidedServerPreview({ result, activates }: { result: ComponentPlanResult; activates: boolean }) {
  const { t } = useTranslation("review");
  const components = result.components ?? (result.component ? [result.component] : []);
  const summary = result.summary ?? {
    count: components.length,
    active: activates ? components.length : 0,
    draft: activates ? 0 : components.length,
  };
  return <div className="space-y-4 text-sm">
    <div className="space-y-1">
      <h3 className="font-medium">{t("swap.components.form.review.title")}</h3>
      <p className="text-muted-foreground">
        {t(activates ? "swap.components.form.review.activates" : "swap.components.form.review.drafts", summary)}
      </p>
      <p className="text-xs text-muted-foreground">{t("swap.components.form.inputVersion", { version: result.input_version })}</p>
    </div>
    {components.map((component, index) => <div key={component.id ?? index} className="space-y-3">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="font-medium">
          {t(`swap.components.type.${component.component_type}`, { defaultValue: component.component_type })}
        </span>
        <span aria-hidden="true" className="text-muted-foreground">·</span>
        <span className="text-muted-foreground">{t(`swap.components.mode.${component.conservation_mode}`)}</span>
        {component.evidence_kind || component.evidence_grade ? (
          <Badge variant="outline" className="font-mono text-2xs font-normal">
            {t("swap.components.audit.evidence", {
              kind: component.evidence_kind ?? "—",
              grade: component.evidence_grade ?? "—",
            })}
          </Badge>
        ) : null}
      </div>
      {component.conversion_policy ? (
        <p className="text-xs text-muted-foreground">
          {t("swap.components.audit.policy", { policy: component.conversion_policy })}
        </p>
      ) : null}
      {component.notes ? <p className="whitespace-pre-wrap text-muted-foreground">{component.notes}</p> : null}

      <ol className="divide-y overflow-hidden rounded-md border bg-background/60">
        {component.legs.map((leg, ordinal) => {
          const scope = [leg.chain, leg.network].filter(Boolean).join(" · ");
          const conserved = [leg.exposure, leg.conservation_unit].filter(Boolean).join(" / ");
          return <li key={leg.id} className="space-y-2 p-3">
            <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
              <p className="flex items-center gap-2">
                <RoleDot role={leg.role} />
                <span className="font-mono text-xs text-muted-foreground">
                  {t("swap.components.form.leg.ordinal", { index: ordinal + 1 })}
                </span>
                <span className="font-medium">{custodyRoleLabel(t, leg.role)}</span>
              </p>
              <p className="font-mono tabular-nums">{formatCustodyMsat(leg.amount_msat, leg.asset)}</p>
            </div>
            <dl className="grid gap-x-4 gap-y-1 text-xs sm:grid-cols-[auto_minmax(0,1fr)]">
              {leg.transaction_id ? <ReviewFact label={t("swap.components.form.review.transaction")} mono>{leg.transaction_id}</ReviewFact> : null}
              {leg.wallet_id ? <ReviewFact label={t("swap.components.form.review.wallet")} mono>{leg.wallet_id}</ReviewFact> : null}
              {leg.occurred_at ? <ReviewFact label={t("swap.components.form.review.occurredAt")} mono>
                <time dateTime={leg.occurred_at}>{leg.occurred_at}</time>
              </ReviewFact> : null}
              {leg.rail ? <ReviewFact label={t("swap.components.form.review.rail")}>{leg.rail}</ReviewFact> : null}
              {scope ? <ReviewFact label={t("swap.components.form.review.network")}>{scope}</ReviewFact> : null}
              {conserved ? <ReviewFact label={t("swap.components.form.review.conserved")}>{conserved}</ReviewFact> : null}
              {leg.valuation_amount != null ? <ReviewFact label={t("swap.components.form.review.valuation")} mono>
                {String(leg.valuation_amount)} {leg.valuation_unit}
              </ReviewFact> : null}
              {leg.notes ? <ReviewFact label={t("swap.components.form.review.notes")}>
                <span className="whitespace-pre-wrap">{leg.notes}</span>
              </ReviewFact> : null}
            </dl>
          </li>;
        })}
      </ol>

      {component.allocations.length > 0 ? <div className="space-y-1">
        <h4 className="text-2xs font-medium tracking-[0.14em] text-muted-foreground uppercase">
          {t("swap.components.form.review.allocations")}
        </h4>
        <ul className="space-y-1 font-mono text-xs tabular-nums">
          {component.allocations.map((allocation, ordinal) => {
            const source = component.legs.findIndex((leg) => leg.id === allocation.source_leg_id);
            const sink = component.legs.findIndex((leg) => leg.id === allocation.sink_leg_id);
            return <li key={ordinal}>
              {t("swap.components.form.review.allocationEdge", {
                source: source + 1,
                sink: sink + 1,
                sourceAmount: formatCustodyMsat(allocation.source_amount_msat, component.legs[source]?.asset ?? "BTC"),
                sinkAmount: formatCustodyMsat(allocation.sink_amount_msat, component.legs[sink]?.asset ?? "BTC"),
              })}
            </li>;
          })}
        </ul>
      </div> : null}

      {component.validation?.issues?.length ? <div className="rounded-md border border-amber-300/60 bg-amber-50 p-3 text-xs text-amber-950 dark:border-amber-400/30 dark:bg-amber-950/30 dark:text-amber-100">
        <div className="font-medium">{t("swap.components.form.review.issues")}</div>
        <ul className="mt-1 list-disc space-y-1 pl-5">
          {component.validation.issues.map((issue, ordinal) => <li key={ordinal}>{custodyBackendIssueText(t, issue)}</li>)}
        </ul>
      </div> : null}
    </div>)}
  </div>;
}
