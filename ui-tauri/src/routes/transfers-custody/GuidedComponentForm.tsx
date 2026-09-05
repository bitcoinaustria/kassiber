/**
 * Guided custody-component builder.
 *
 * A structured, no-JSON editor for authoring custody components (manual bridges
 * and swaps). It builds the same spec object the daemon expects, validates it
 * live with {@link previewCustodyComponentBatch}, and submits it through the
 * `ui.transfers.components.{plan,apply}` dry-run → commit flow.
 *
 * Layout: the body is a scrollable column of sections (interpretation, legs,
 * allocations, evidence, validation, daemon review); the actions live in a
 * footer that a dialog host can pin below the scroll region. The "embedded"
 * variant therefore renders two siblings — body and footer — for a
 * `grid-rows-[auto_minmax(0,1fr)_auto]` dialog.
 */
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Loader2, Plus, RotateCcw, Save } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
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
import { bookIdentityKey, useUiStore } from "@/store/ui";
import { GuidedLegRow, AllocationRow, GuidedServerPreview, type ComponentPlanResult } from "./GuidedComponentFields";
import {
  GuidedFormSection,
  GuidedNotice,
  MovementSummary,
  type MovementTotals,
} from "./GuidedComponentLayout";
import {
  CustodyErrorList,
  custodyMutationError,
} from "./custodyComponentIssues";
import {
  canConfirmGuidedPlan,
  bindGuidedPlanScope,
  type GuidedPlanCommitment,
  createGuidedAllocation,
  createGuidedLeg,
  createInitialGuidedForm,
  formToComponentSpec,
  formToDocument,
  hasGuidedFormInput,
  isSinkRole,
  type GuidedAllocationForm,
  type GuidedComponentFormState,
  type GuidedLegForm,
  type GuidedLegRole,
} from "./guidedComponentModel";

interface ReviewedPlan extends GuidedPlanCommitment {
  args: Record<string, unknown>;
  result: ComponentPlanResult;
  activate: boolean;
}

function currentScope(): string {
  const state = useUiStore.getState();
  return JSON.stringify([bookIdentityKey(state.identity), state.daemonSession, state.dataMode]);
}

export interface GuidedComponentFormProps {
  /** Preload state to edit/revise an existing component; omit to author new. */
  initialForm?: GuidedComponentFormState;
  /** When set, submitting revises this component instead of creating a batch. */
  edit?: { componentId: string; state: "draft" | "active" };
  /** Called after a successful revise (e.g. to close the editor). */
  onDone?: () => void;
  /** Reports the form's submit-in-flight state (so a host dialog can block
   *  dismissal mid-submit — this form uses its own mutation observers). */
  onBusyChange?: (busy: boolean) => void;
  /** "card" wraps in a titled Card (create); "embedded" renders a scroll body
   *  plus a footer row for a `grid-rows-[auto_minmax(0,1fr)_auto]` dialog. */
  variant?: "card" | "embedded";
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

function movementTotals(legs: GuidedLegForm[]): MovementTotals {
  const totals = {
    sources: new Map<string, bigint>(), destinations: new Map<string, bigint>(),
    fees: new Map<string, bigint>(), external: new Map<string, bigint>(), suspense: new Map<string, bigint>(),
  };
  const roles = { source: "sources", destination: "destinations", retained: "destinations", fee: "fees", external: "external", suspense: "suspense" } as const;
  for (const leg of legs) {
    const amount = decimalBtcToMsat(leg.amountBtc.trim());
    if (amount === null) continue;
    const asset = leg.asset.trim().toUpperCase() || "BTC";
    const group = totals[roles[leg.role]];
    group.set(asset, (group.get(asset) ?? 0n) + amount);
  }
  return totals;
}

type InFlight = "draft" | "activate" | "confirm" | null;

export function GuidedComponentForm({
  initialForm,
  edit,
  onDone,
  onBusyChange,
  variant = "card",
}: GuidedComponentFormProps = {}) {
  const { t } = useTranslation("review");
  const formId = useId();
  const [form, setForm] = useState<GuidedComponentFormState>(
    () => initialForm ?? createInitialGuidedForm(),
  );
  const [result, setResult] = useState<
    ComponentPlanResult["summary"] | null
  >(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [reviewedPlan, setReviewedPlan] = useState<ReviewedPlan | null>(null);
  const [inFlight, setInFlight] = useState<InFlight>(null);
  const reviewRef = useRef<HTMLDivElement | null>(null);
  const scope = useUiStore((state) => JSON.stringify([
    bookIdentityKey(state.identity), state.daemonSession, state.dataMode,
  ]));
  const document = formToDocument(form);
  const latestDocument = useRef(document);
  latestDocument.current = document;
  const canConfirm = canConfirmGuidedPlan(reviewedPlan, document, scope);

  useEffect(() => { setReviewedPlan(null); }, [document, scope]);

  const planMutation = useDaemonMutation<ComponentPlanResult>(
    "ui.transfers.components.plan",
    { invalidateQueries: false },
  );
  const applyMutation = useDaemonMutation<ComponentPlanResult>(
    "ui.transfers.components.apply",
  );
  const pending = planMutation.isPending || applyMutation.isPending;

  useEffect(() => {
    onBusyChange?.(pending);
    return () => onBusyChange?.(false);
  }, [pending, onBusyChange]);

  // Bring the daemon's review into view once it arrives so the confirmation
  // is never made against a panel that is scrolled out of sight.
  useEffect(() => {
    if (reviewedPlan && canConfirm) {
      reviewRef.current?.scrollIntoView?.({ block: "nearest", behavior: "smooth" });
    }
  }, [reviewedPlan, canConfirm]);

  const preview = useMemo(() => previewCustodyComponentBatch(formToDocument(form)), [
    form,
  ]);
  const totals = useMemo(() => movementTotals(form.legs), [form.legs]);
  const sinkMsat = useMemo(() => sumMsat(form.legs, isSinkRole), [form.legs]);
  const sourceMsat = sumMsat(form.legs, (role) => role === "source");
  const balanced = sourceMsat > 0n && sourceMsat === sinkMsat;
  // This summary adds only the shared Bitcoin/msat exposure. Other assets or
  // explicit conserved units remain visible in the legs and daemon review.
  const hasBitcoinSummary = form.legs.every((leg) =>
    ["BTC", "LBTC"].includes(leg.asset.trim().toUpperCase() || "BTC") &&
    (!leg.origin?.exposure || leg.origin.exposure === "bitcoin") &&
    (!leg.origin?.conservationUnit || leg.origin.conservationUnit === "msat"),
  );
  const canActivate =
    preview.structuralErrors.length === 0 && preview.activationErrors.length === 0;
  const canSaveDraft = preview.structuralErrors.length === 0;
  const showValidation = Boolean(initialForm) || hasGuidedFormInput(form);

  const patchForm = (patch: Partial<GuidedComponentFormState>) => {
    if (pending) return;
    setForm((prev) => ({ ...prev, ...patch }));
    setResult(null);
    setActionError(null);
  };

  const patchLeg = (key: string, patch: Partial<GuidedLegForm>) => {
    if (pending) return;
    setForm((prev) => ({
      ...prev,
      legs: prev.legs.map((leg) => (leg.key === key ? { ...leg, ...patch } : leg)),
    }));
    setResult(null);
    setActionError(null);
  };

  const addLeg = () => patchForm({ legs: [...form.legs, createGuidedLeg("destination")] });
  const removeLeg = (key: string) =>
    // Drop any allocation edge that referenced the removed leg so no dangling
    // edge (and its typed amount) is silently discarded on submit.
    patchForm({
      legs: form.legs.filter((leg) => leg.key !== key),
      allocations: form.allocations.filter(
        (allocation) => allocation.sourceKey !== key && allocation.sinkKey !== key,
      ),
    });
  const reset = () => {
    setForm(createInitialGuidedForm());
    setResult(null);
    setActionError(null);
  };

  const addAllocation = () =>
    patchForm({ allocations: [...form.allocations, createGuidedAllocation()] });
  const patchAllocation = (key: string, patch: Partial<GuidedAllocationForm>) => {
    if (pending) return;
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
    const spec = formToComponentSpec(form);
    const planScope = currentScope();
    const planDocument = document;
    // The daemon's active scope is authoritative; persisted UI identity labels
    // can lag behind a book opened by the daemon or the browser preview.
    const planArgs = edit
      ? { action: "revise", component_id: edit.componentId, spec, activate }
      : buildCustodyBulkRequest(nextPreview, { activate });
    setReviewedPlan(null);
    setInFlight(activate ? "activate" : "draft");
    try {
      const plan = await planMutation.mutateAsync(planArgs);
      if (!plan.data || !Number.isSafeInteger(plan.data.input_version) ||
          !(plan.data.component || plan.data.components?.length)) {
        setActionError(t("swap.components.backendError.unexpected"));
        return;
      }
      const scopedArgs = bindGuidedPlanScope(planArgs, plan.data);
      if (!scopedArgs) {
        setActionError(t("swap.components.backendError.unexpected"));
        return;
      }
      if (currentScope() !== planScope || latestDocument.current !== planDocument) {
        setActionError(t("swap.components.form.previewChanged"));
        return;
      }
      setReviewedPlan({
        document: planDocument, scope: planScope, inputVersion: plan.data.input_version,
        args: scopedArgs, result: plan.data, activate,
      });
    } catch (error) {
      setActionError(custodyMutationError(t, error));
    } finally {
      setInFlight(null);
    }
  };

  const confirm = async () => {
    if (pending || !reviewedPlan ||
        !canConfirmGuidedPlan(reviewedPlan, latestDocument.current, currentScope())) {
      setActionError(t("swap.components.form.previewChanged"));
      return;
    }
    setActionError(null);
    setInFlight("confirm");
    try {
      const applied = await applyMutation.mutateAsync({
        ...reviewedPlan.args, expected_input_version: reviewedPlan.inputVersion,
      });
      setReviewedPlan(null);
      if (applied.data && currentScope() === reviewedPlan.scope) {
        if (edit) onDone?.();
        else {
          setForm(createInitialGuidedForm());
          setResult(applied.data.summary ?? null);
        }
      }
    } catch (error) {
      setReviewedPlan(null);
      setActionError(custodyMutationError(t, error));
    } finally {
      setInFlight(null);
    }
  };

  const conversionMode = form.conservationMode === "conversion";
  const hasReview = Boolean(reviewedPlan && canConfirm);

  const body = (
    <fieldset disabled={pending} className="min-w-0 space-y-7">
      <GuidedFormSection
        title={t("swap.components.form.sections.interpretation")}
        hint={t("swap.components.form.sections.interpretationHint")}
      >
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor={`${formId}-component-type`}>
              {t("swap.components.form.componentType")}
            </Label>
            <Select
              value={form.componentType}
              onValueChange={(value) =>
                patchForm({ componentType: value as "manual_bridge" | "swap" })
              }
            >
              <SelectTrigger id={`${formId}-component-type`} className="w-full">
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
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor={`${formId}-conservation-mode`}>
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
              <SelectTrigger id={`${formId}-conservation-mode`} className="w-full">
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
        </div>
        {conversionMode ? (
          <div className="grid gap-4 rounded-lg border border-cyan-500/30 bg-cyan-500/5 p-4 sm:grid-cols-2 sm:items-end">
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor={`${formId}-conversion-policy`}>
                {t("swap.components.form.conversionPolicy")}
              </Label>
              <Input
                id={`${formId}-conversion-policy`}
                className="font-mono"
                value={form.conversionPolicy}
                spellCheck={false}
                placeholder={t("swap.components.form.conversionPolicyPlaceholder")}
                onChange={(event) =>
                  patchForm({ conversionPolicy: event.target.value })
                }
              />
            </div>
            <label className="flex min-h-9 items-center gap-2 text-sm">
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
      </GuidedFormSection>

      <GuidedFormSection
        title={t("swap.components.form.legsTitle")}
        hint={t("swap.components.form.legsHint")}
        action={
          <Button type="button" size="sm" variant="outline" onClick={addLeg}>
            <Plus />
            {t("swap.components.form.addLeg")}
          </Button>
        }
      >
        <div className="space-y-3">
          {form.legs.map((leg, index) => (
            <GuidedLegRow
              key={leg.key}
              leg={leg}
              ordinal={index}
              canRemove={form.legs.length > 2}
              conversionMode={conversionMode}
              onChange={(patch) => patchLeg(leg.key, patch)}
              onRemove={() => removeLeg(leg.key)}
            />
          ))}
        </div>
        {form.conservationMode === "quantity" && hasBitcoinSummary ? (
          <MovementSummary totals={totals} balanced={balanced} />
        ) : null}
      </GuidedFormSection>

      {showAllocations ? (
        <GuidedFormSection
          title={t("swap.components.form.allocations.title")}
          hint={t("swap.components.form.allocations.hint")}
          action={
            <Button type="button" size="sm" variant="outline" onClick={addAllocation}>
              <Plus />
              {t("swap.components.form.allocations.add")}
            </Button>
          }
        >
          {form.allocations.length === 0 ? (
            <p className="rounded-lg border border-dashed px-4 py-3 text-xs leading-5 text-muted-foreground">
              {t("swap.components.form.allocations.empty")}
            </p>
          ) : (
            <div className="space-y-3">
              {form.allocations.map((allocation) => (
                <AllocationRow
                  key={allocation.key}
                  allocation={allocation}
                  conversionMode={conversionMode}
                  legs={form.legs}
                  sourceLegs={sourceLegs}
                  sinkLegs={sinkLegs}
                  onChange={(patch) => patchAllocation(allocation.key, patch)}
                  onRemove={() => removeAllocation(allocation.key)}
                />
              ))}
            </div>
          )}
        </GuidedFormSection>
      ) : null}

      <GuidedFormSection
        title={t("swap.components.form.sections.evidence")}
        hint={t("swap.components.form.sections.evidenceHint")}
      >
        <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,18rem)]">
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor={`${formId}-notes`}>{t("swap.components.form.notes")}</Label>
            <Textarea
              id={`${formId}-notes`}
              className="min-h-20"
              value={form.notes}
              maxLength={2000}
              onChange={(event) => patchForm({ notes: event.target.value })}
              placeholder={t("swap.components.form.notesPlaceholder")}
            />
          </div>
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor={`${formId}-evidence-kind`}>
              {t("swap.components.form.evidenceKind")}
            </Label>
            <Input
              id={`${formId}-evidence-kind`}
              className="font-mono"
              value={form.evidenceKind}
              spellCheck={false}
              onChange={(event) => patchForm({ evidenceKind: event.target.value })}
              placeholder="manual_migration_review"
            />
            <p className="text-xs leading-5 text-muted-foreground">
              {t("swap.components.form.evidenceKindHint")}
            </p>
          </div>
        </div>
      </GuidedFormSection>

      {showValidation && preview.structuralErrors.length > 0 ? (
        <CustodyErrorList
          title={t("swap.components.structuralErrors")}
          issues={preview.structuralErrors}
          destructive
        />
      ) : null}
      {showValidation && preview.activationErrors.length > 0 ? (
        <CustodyErrorList
          title={t("swap.components.activationErrors")}
          issues={preview.activationErrors}
        />
      ) : null}

      {reviewedPlan && canConfirm ? (
        <div ref={reviewRef} className="rounded-lg border border-primary/30 bg-muted/20 p-4">
          <GuidedServerPreview result={reviewedPlan.result} activates={reviewedPlan.activate} />
        </div>
      ) : null}
    </fieldset>
  );

  const footer = (
    <div className="space-y-3">
      {result ? (
        <GuidedNotice tone="success">{t("swap.components.savedSummary", result)}</GuidedNotice>
      ) : null}
      {actionError ? <GuidedNotice tone="error">{actionError}</GuidedNotice> : null}
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:items-center sm:justify-between">
        {edit ? (
          <Button
            type="button"
            variant="ghost"
            disabled={pending}
            onClick={() => onDone?.()}
          >
            {t("swap.components.revisionDialog.cancel")}
          </Button>
        ) : (
          <Button
            type="button"
            variant="ghost"
            disabled={pending}
            onClick={reset}
          >
            <RotateCcw />
            {t("swap.components.form.reset")}
          </Button>
        )}
        <div className="flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:justify-end">
          <Button
            type="button"
            variant={hasReview ? "outline" : "secondary"}
            disabled={pending || !canSaveDraft}
            onClick={() => void submit(false)}
          >
            {inFlight === "draft" ? <Loader2 className="animate-spin" /> : <Save />}
            {t("swap.components.form.previewDraft")}
          </Button>
          <Button
            type="button"
            variant={hasReview ? "outline" : "default"}
            disabled={pending || !canActivate}
            onClick={() => void submit(true)}
          >
            {inFlight === "activate" ? <Loader2 className="animate-spin" /> : <Check />}
            {t("swap.components.form.previewActivation")}
          </Button>
          {reviewedPlan && canConfirm ? (
            <Button type="button" disabled={pending || !canConfirm} onClick={() => void confirm()}>
              {inFlight === "confirm" ? <Loader2 className="animate-spin" /> : <Check />}
              {reviewedPlan.activate
                ? t("swap.components.form.confirmActivation")
                : t("swap.components.form.confirmDraft")}
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );

  if (variant === "embedded") {
    return (
      <>
        <div className="min-h-0 overflow-y-auto px-6 py-5">{body}</div>
        <div className="border-t bg-background/80 px-6 py-4">{footer}</div>
      </>
    );
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("swap.components.form.title")}</CardTitle>
        <CardDescription>{t("swap.components.form.description")}</CardDescription>
      </CardHeader>
      <CardContent>{body}</CardContent>
      <CardFooter className="block border-t">{footer}</CardFooter>
    </Card>
  );
}
