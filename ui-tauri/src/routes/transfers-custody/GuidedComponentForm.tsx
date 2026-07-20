/**
 * Guided custody-component builder.
 *
 * A structured, no-JSON editor for authoring custody components (manual bridges
 * and swaps). It builds the same spec object the daemon expects, validates it
 * live with {@link previewCustodyComponentBatch}, and submits it through the
 * `ui.transfers.components.{plan,apply}` dry-run → commit flow.
 */
import { useMemo, useState } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { Check, Loader2, Plus, RotateCcw, Save, Trash2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useDaemonMutation } from "@/daemon/client";
import {
  buildCustodyBulkRequest,
  decimalBtcToMsat,
  previewCustodyComponentBatch,
} from "@/lib/custodyComponentBulk";
import { cn } from "@/lib/utils";
import { formatCustodyMsat } from "../custodyGapsModel";
import {
  CustodyErrorList,
  custodyMutationError,
  custodyRoleLabel,
} from "./custodyComponentIssues";
import {
  createGuidedAllocation,
  createGuidedLeg,
  createInitialGuidedForm,
  formToDocument,
  isOwnedRole,
  isSinkRole,
  type GuidedAllocationForm,
  type GuidedComponentFormState,
  type GuidedLegForm,
  type GuidedLegRole,
  type GuidedLocationKind,
} from "./guidedComponentModel";

// Suspense legs need reviewed grade + explicit allocations; they are authored
// through the advanced path, so the guided role picker offers the everyday set.
const GUIDED_FORM_ROLES: readonly GuidedLegRole[] = [
  "source",
  "destination",
  "fee",
  "retained",
  "external",
];

const LOCATION_KINDS: readonly GuidedLocationKind[] = [
  "transaction",
  "wallet",
  "untracked",
];

interface BulkResolveResult {
  input_version: number;
  summary: { count: number; active: number; draft: number };
}

function sumMsat(legs: GuidedLegForm[], predicate: (role: GuidedLegRole) => boolean) {
  let total = 0n;
  for (const leg of legs) {
    if (!predicate(leg.role)) continue;
    const msat = decimalBtcToMsat(leg.amountBtc.trim());
    if (msat !== null) total += msat;
  }
  return total;
}

export function GuidedComponentForm() {
  const { t } = useTranslation("review");
  const [form, setForm] = useState<GuidedComponentFormState>(
    createInitialGuidedForm,
  );
  const [result, setResult] = useState<BulkResolveResult["summary"] | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const planMutation = useDaemonMutation<BulkResolveResult>(
    "ui.transfers.components.plan",
    { invalidateQueries: false },
  );
  const applyMutation = useDaemonMutation<BulkResolveResult>(
    "ui.transfers.components.apply",
  );
  const pending = planMutation.isPending || applyMutation.isPending;

  const preview = useMemo(() => previewCustodyComponentBatch(formToDocument(form)), [
    form,
  ]);
  const sourceMsat = useMemo(
    () => sumMsat(form.legs, (role) => role === "source"),
    [form.legs],
  );
  const sinkMsat = useMemo(() => sumMsat(form.legs, isSinkRole), [form.legs]);
  const balanced = sourceMsat > 0n && sourceMsat === sinkMsat;
  const canActivate =
    preview.structuralErrors.length === 0 && preview.activationErrors.length === 0;
  const canSaveDraft = preview.structuralErrors.length === 0;

  const patchForm = (patch: Partial<GuidedComponentFormState>) => {
    setForm((prev) => ({ ...prev, ...patch }));
    setResult(null);
    setActionError(null);
  };

  const patchLeg = (key: string, patch: Partial<GuidedLegForm>) => {
    setForm((prev) => ({
      ...prev,
      legs: prev.legs.map((leg) => (leg.key === key ? { ...leg, ...patch } : leg)),
    }));
    setResult(null);
    setActionError(null);
  };

  const addLeg = () => patchForm({ legs: [...form.legs, createGuidedLeg("destination")] });
  const removeLeg = (key: string) =>
    patchForm({ legs: form.legs.filter((leg) => leg.key !== key) });
  const reset = () => {
    setForm(createInitialGuidedForm());
    setResult(null);
    setActionError(null);
  };

  const addAllocation = () =>
    patchForm({ allocations: [...form.allocations, createGuidedAllocation()] });
  const patchAllocation = (key: string, patch: Partial<GuidedAllocationForm>) => {
    setForm((prev) => ({
      ...prev,
      allocations: prev.allocations.map((allocation) =>
        allocation.key === key ? { ...allocation, ...patch } : allocation,
      ),
    }));
    setResult(null);
    setActionError(null);
  };
  const removeAllocation = (key: string) =>
    patchForm({
      allocations: form.allocations.filter((allocation) => allocation.key !== key),
    });

  const sourceLegs = form.legs.filter((leg) => leg.role === "source");
  const sinkLegs = form.legs.filter((leg) => isSinkRole(leg.role));
  const hasSuspenseLeg = form.legs.some((leg) => leg.role === "suspense");
  const showAllocations =
    form.allocations.length > 0 ||
    sourceLegs.length > 1 ||
    hasSuspenseLeg ||
    form.conservationMode === "conversion";

  const submit = async (activate: boolean) => {
    setResult(null);
    setActionError(null);
    const nextPreview = previewCustodyComponentBatch(formToDocument(form));
    if (nextPreview.structuralErrors.length > 0) return;
    if (activate && nextPreview.activationErrors.length > 0) return;
    try {
      const plan = await planMutation.mutateAsync(
        buildCustodyBulkRequest(nextPreview, { activate }),
      );
      if (plan.data?.input_version === undefined) {
        setActionError(t("swap.components.backendError.unexpected"));
        return;
      }
      const applied = await applyMutation.mutateAsync(
        buildCustodyBulkRequest(nextPreview, {
          activate,
          expectedInputVersion: plan.data.input_version,
        }),
      );
      if (applied.data) {
        setResult(applied.data.summary);
        reset();
      }
    } catch (error) {
      setActionError(custodyMutationError(t, error));
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("swap.components.form.title")}</CardTitle>
        <CardDescription>{t("swap.components.form.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <div className="space-y-1.5">
            <Label htmlFor="guided-component-type">
              {t("swap.components.form.componentType")}
            </Label>
            <Select
              value={form.componentType}
              onValueChange={(value) =>
                patchForm({ componentType: value as "manual_bridge" | "swap" })
              }
            >
              <SelectTrigger id="guided-component-type">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="manual_bridge">
                  {t("swap.components.type.manual_bridge")}
                </SelectItem>
                <SelectItem value="swap">
                  {t("swap.components.type.swap")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="guided-conservation-mode">
              {t("swap.components.form.conservationMode")}
            </Label>
            <Select
              value={form.conservationMode}
              onValueChange={(value) =>
                patchForm({
                  conservationMode: value as "quantity" | "conversion",
                })
              }
            >
              <SelectTrigger id="guided-conservation-mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="quantity">
                  {t("swap.components.mode.quantity")}
                </SelectItem>
                <SelectItem value="conversion">
                  {t("swap.components.mode.conversion")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="guided-evidence-kind">
              {t("swap.components.form.evidenceKind")}
            </Label>
            <Input
              id="guided-evidence-kind"
              value={form.evidenceKind}
              spellCheck={false}
              onChange={(event) => patchForm({ evidenceKind: event.target.value })}
              placeholder="manual_migration_review"
            />
          </div>
        </div>
        {form.conservationMode === "conversion" ? (
          <div className="grid gap-4 rounded-md border border-cyan-500/30 bg-cyan-500/5 p-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="guided-conversion-policy">
                {t("swap.components.form.conversionPolicy")}
              </Label>
              <Input
                id="guided-conversion-policy"
                value={form.conversionPolicy}
                spellCheck={false}
                placeholder={t("swap.components.form.conversionPolicyPlaceholder")}
                onChange={(event) =>
                  patchForm({ conversionPolicy: event.target.value })
                }
              />
            </div>
            <label className="flex items-end gap-2 pb-2 text-sm sm:items-center sm:pb-0">
              <Checkbox
                checked={form.conversionReviewed}
                onCheckedChange={(checked) =>
                  patchForm({ conversionReviewed: checked === true })
                }
              />
              <span>{t("swap.components.form.conversionReviewed")}</span>
            </label>
          </div>
        ) : null}
        <div className="space-y-1.5">
          <Label htmlFor="guided-notes">{t("swap.components.form.notes")}</Label>
          <Textarea
            id="guided-notes"
            value={form.notes}
            maxLength={2000}
            onChange={(event) => patchForm({ notes: event.target.value })}
            placeholder={t("swap.components.form.notesPlaceholder")}
          />
        </div>

        <div className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3 className="text-sm font-medium">
                {t("swap.components.form.legsTitle")}
              </h3>
              <p className="text-xs text-muted-foreground">
                {t("swap.components.form.legsHint")}
              </p>
            </div>
            <Button type="button" size="sm" variant="outline" onClick={addLeg}>
              <Plus />
              {t("swap.components.form.addLeg")}
            </Button>
          </div>

          {form.legs.map((leg) => (
            <GuidedLegRow
              key={leg.key}
              leg={leg}
              canRemove={form.legs.length > 2}
              conversionMode={form.conservationMode === "conversion"}
              onChange={(patch) => patchLeg(leg.key, patch)}
              onRemove={() => removeLeg(leg.key)}
            />
          ))}
        </div>

        {showAllocations ? (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div>
                <h3 className="text-sm font-medium">
                  {t("swap.components.form.allocations.title")}
                </h3>
                <p className="text-xs text-muted-foreground">
                  {t("swap.components.form.allocations.hint")}
                </p>
              </div>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={addAllocation}
              >
                <Plus />
                {t("swap.components.form.allocations.add")}
              </Button>
            </div>
            {form.allocations.length === 0 ? (
              <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
                {t("swap.components.form.allocations.empty")}
              </p>
            ) : (
              form.allocations.map((allocation) => (
                <AllocationRow
                  key={allocation.key}
                  allocation={allocation}
                  legs={form.legs}
                  sourceLegs={sourceLegs}
                  sinkLegs={sinkLegs}
                  onChange={(patch) => patchAllocation(allocation.key, patch)}
                  onRemove={() => removeAllocation(allocation.key)}
                />
              ))
            )}
          </div>
        ) : null}

        <div
          className={cn(
            "flex flex-wrap items-center justify-between gap-3 rounded-md border p-3 text-sm",
            balanced
              ? "border-emerald-500/30 bg-emerald-500/5"
              : "border-amber-500/40 bg-amber-500/10",
          )}
        >
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <span>
              <span className="text-muted-foreground">
                {t("swap.components.form.balance.sources")}:{" "}
              </span>
              {formatCustodyMsat(sourceMsat.toString(), "BTC")}
            </span>
            <span>
              <span className="text-muted-foreground">
                {t("swap.components.form.balance.sinks")}:{" "}
              </span>
              {formatCustodyMsat(sinkMsat.toString(), "BTC")}
            </span>
          </div>
          <Badge variant={balanced ? "default" : "secondary"}>
            {balanced
              ? t("swap.components.form.balance.balanced")
              : t("swap.components.form.balance.unbalanced")}
          </Badge>
        </div>

        {preview.structuralErrors.length > 0 ? (
          <CustodyErrorList
            title={t("swap.components.structuralErrors")}
            issues={preview.structuralErrors}
            destructive
          />
        ) : null}
        {preview.activationErrors.length > 0 ? (
          <CustodyErrorList
            title={t("swap.components.activationErrors")}
            issues={preview.activationErrors}
          />
        ) : null}

        {result ? (
          <div className="rounded-md border border-emerald-200 bg-emerald-50 p-3 text-sm text-emerald-950 dark:border-emerald-400/30 dark:bg-emerald-950/30 dark:text-emerald-100">
            {t("swap.components.savedSummary", result)}
          </div>
        ) : null}
        {actionError ? (
          <div className="whitespace-pre-wrap rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
            {actionError}
          </div>
        ) : null}

        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="secondary"
            disabled={pending || !canSaveDraft}
            onClick={() => void submit(false)}
          >
            {pending ? <Loader2 className="animate-spin" /> : <Save />}
            {t("swap.components.form.saveDraft")}
          </Button>
          <Button
            type="button"
            disabled={pending || !canActivate}
            onClick={() => void submit(true)}
          >
            {pending ? <Loader2 className="animate-spin" /> : <Check />}
            {t("swap.components.form.activate")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={pending}
            onClick={reset}
          >
            <RotateCcw />
            {t("swap.components.form.reset")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function GuidedLegRow({
  leg,
  canRemove,
  conversionMode,
  onChange,
  onRemove,
}: {
  leg: GuidedLegForm;
  canRemove: boolean;
  conversionMode: boolean;
  onChange: (patch: Partial<GuidedLegForm>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation("review");
  const showOccurredAt =
    leg.locationKind === "untracked" ||
    (leg.locationKind === "wallet" && isOwnedRole(leg.role));

  return (
    <div className="space-y-3 rounded-lg border p-3">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <div className="space-y-1.5">
          <Label>{t("swap.components.form.leg.role")}</Label>
          <Select
            value={leg.role}
            onValueChange={(value) => onChange({ role: value as GuidedLegRole })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {GUIDED_FORM_ROLES.map((role) => (
                <SelectItem key={role} value={role}>
                  {custodyRoleLabel(t, role)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label>{t("swap.components.form.leg.amount")}</Label>
          <Input
            inputMode="decimal"
            value={leg.amountBtc}
            spellCheck={false}
            placeholder="0.00000000"
            onChange={(event) => onChange({ amountBtc: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label>{t("swap.components.form.leg.location")}</Label>
          <Select
            value={leg.locationKind}
            onValueChange={(value) =>
              onChange({ locationKind: value as GuidedLocationKind })
            }
          >
            <SelectTrigger>
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
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        {leg.locationKind === "transaction" ? (
          <div className="space-y-1.5 sm:col-span-2">
            <Label>{t("swap.components.form.leg.transactionRef")}</Label>
            <Input
              value={leg.transactionRef}
              spellCheck={false}
              placeholder={t("swap.components.form.leg.transactionRefPlaceholder")}
              onChange={(event) => onChange({ transactionRef: event.target.value })}
            />
          </div>
        ) : null}
        {leg.locationKind === "wallet" ? (
          <div className="space-y-1.5">
            <Label>{t("swap.components.form.leg.walletRef")}</Label>
            <Input
              value={leg.walletRef}
              spellCheck={false}
              placeholder={t("swap.components.form.leg.walletRefPlaceholder")}
              onChange={(event) => onChange({ walletRef: event.target.value })}
            />
          </div>
        ) : null}
        {leg.locationKind === "untracked" ? (
          <div className="space-y-1.5">
            <Label>{t("swap.components.form.leg.untrackedWallet")}</Label>
            <Input
              value={leg.untrackedWallet}
              spellCheck={false}
              placeholder={t("swap.components.form.leg.untrackedWalletPlaceholder")}
              onChange={(event) =>
                onChange({ untrackedWallet: event.target.value })
              }
            />
          </div>
        ) : null}
        {showOccurredAt ? (
          <div className="space-y-1.5">
            <Label>{t("swap.components.form.leg.occurredAt")}</Label>
            <Input
              type="datetime-local"
              value={leg.occurredAt}
              onChange={(event) => onChange({ occurredAt: event.target.value })}
            />
          </div>
        ) : null}
      </div>

      {conversionMode ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label>{t("swap.components.form.leg.valuationUnit")}</Label>
            <Input
              value={leg.valuationUnit}
              spellCheck={false}
              placeholder={t("swap.components.form.leg.valuationUnitPlaceholder")}
              onChange={(event) => onChange({ valuationUnit: event.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <Label>{t("swap.components.form.leg.valuationAmount")}</Label>
            <Input
              inputMode="numeric"
              value={leg.valuationAmount}
              spellCheck={false}
              placeholder={t("swap.components.form.leg.valuationAmountPlaceholder")}
              onChange={(event) => onChange({ valuationAmount: event.target.value })}
            />
          </div>
        </div>
      ) : null}

      {canRemove ? (
        <div className="flex justify-end">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="text-muted-foreground"
            onClick={onRemove}
          >
            <Trash2 />
            {t("swap.components.form.removeLeg")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function legDisplayLabel(
  t: TFunction<"review">,
  leg: GuidedLegForm,
  index: number,
): string {
  return `${index + 1} · ${custodyRoleLabel(t, leg.role)}`;
}

function AllocationRow({
  allocation,
  legs,
  sourceLegs,
  sinkLegs,
  onChange,
  onRemove,
}: {
  allocation: GuidedAllocationForm;
  legs: GuidedLegForm[];
  sourceLegs: GuidedLegForm[];
  sinkLegs: GuidedLegForm[];
  onChange: (patch: Partial<GuidedAllocationForm>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation("review");
  return (
    <div className="grid gap-3 rounded-lg border p-3 sm:grid-cols-[1fr_1fr_1fr_auto] sm:items-end">
      <div className="space-y-1.5">
        <Label>{t("swap.components.form.allocations.source")}</Label>
        <Select
          value={allocation.sourceKey}
          onValueChange={(value) => onChange({ sourceKey: value })}
        >
          <SelectTrigger>
            <SelectValue placeholder={t("swap.components.form.allocations.selectLeg")} />
          </SelectTrigger>
          <SelectContent>
            {sourceLegs.map((leg) => (
              <SelectItem key={leg.key} value={leg.key}>
                {legDisplayLabel(t, leg, legs.indexOf(leg))}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-1.5">
        <Label>{t("swap.components.form.allocations.sink")}</Label>
        <Select
          value={allocation.sinkKey}
          onValueChange={(value) => onChange({ sinkKey: value })}
        >
          <SelectTrigger>
            <SelectValue placeholder={t("swap.components.form.allocations.selectLeg")} />
          </SelectTrigger>
          <SelectContent>
            {sinkLegs.map((leg) => (
              <SelectItem key={leg.key} value={leg.key}>
                {legDisplayLabel(t, leg, legs.indexOf(leg))}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-1.5">
        <Label>{t("swap.components.form.allocations.amount")}</Label>
        <Input
          inputMode="decimal"
          value={allocation.amountBtc}
          spellCheck={false}
          placeholder="0.00000000"
          onChange={(event) => onChange({ amountBtc: event.target.value })}
        />
      </div>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        className="text-muted-foreground"
        onClick={onRemove}
        aria-label={t("swap.components.form.allocations.remove")}
      >
        <Trash2 />
      </Button>
    </div>
  );
}
