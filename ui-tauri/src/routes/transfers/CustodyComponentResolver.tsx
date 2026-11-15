/**
 * Custody components: the developer-gated editor for histories a simple
 * link can't describe (one send into several wallets, a migration). Moved
 * out of the old matching tools unchanged.
 */
import { useMemo, useState } from "react";
import { Check, ChevronDown, ChevronRight, Loader2, Pencil, Plus, Trash2, Undo2, Unlink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { formatCustodyExactInteger, type CustodyExactInteger } from "@/lib/custodyComponentBulk";
import { pageDescriptionClassName } from "@/lib/screen-layout";
import { currentUiLocale } from "@/lib/localeFormat";
import { GuidedComponentForm } from "../transfers-custody/GuidedComponentForm";
import {
  componentToFormState,
  isGuidedEditableComponentType,
  type GuidedComponentFormState,
} from "../transfers-custody/guidedComponentModel";
import { custodyBackendIssueText, custodyMutationError, custodyRoleLabel } from "../transfers-custody/custodyComponentIssues";

const REVIEW_PAGE_SIZE = 25;

function compactRecordId(value: string) {
  if (!value) return "—";
  if (value.length <= 22) return value;
  return `${value.slice(0, 12)}…${value.slice(-6)}`;
}

interface CustodyComponentLeg {
  id: string;
  ordinal: number;
  role: string;
  rail: string;
  chain?: string | null;
  network?: string | null;
  asset: string;
  exposure?: string | null;
  conservation_unit?: string | null;
  amount_msat: CustodyExactInteger;
  valuation_unit?: string | null;
  valuation_amount?: CustodyExactInteger | null;
  transaction_id: string | null;
  anchor_transaction_id?: string | null;
  wallet_id: string | null;
  occurred_at: string | null;
  notes?: string | null;
}

interface CustodyComponentAllocation {
  id: string;
  ordinal: number;
  source_leg_id: string;
  sink_leg_id: string;
  source_amount_msat: CustodyExactInteger;
  sink_amount_msat: CustodyExactInteger;
}

interface CustodyValidationIssue {
  code?: string;
  message?: string;
  [key: string]: unknown;
}

interface CustodyComponent {
  id: string;
  lineage_id: string;
  revision: number;
  component_type: string;
  conservation_mode: "quantity" | "conversion";
  state: "draft" | "active" | "superseded";
  effective_state: "draft" | "active" | "superseded";
  evidence_kind: string | null;
  evidence_grade: string | null;
  conversion_policy: string | null;
  conversion_reviewed: boolean;
  authored_source: string;
  notes: string | null;
  created_at: string;
  activated_at: string | null;
  superseded_at: string | null;
  legs: CustodyComponentLeg[];
  allocations: CustodyComponentAllocation[];
  validation: {
    activatable?: boolean;
    issues?: CustodyValidationIssue[];
  };
}

type CustodyComponentListEnvelope = {
  components: CustodyComponent[];
  has_more?: boolean;
  limit?: number;
};

interface CustodyComponentMutationResult {
  input_version: number;
  component: CustodyComponent;
  dry_run?: boolean;
}

function custodyComponentsFromEnvelope(data: CustodyComponentListEnvelope | undefined) {
  return data?.components ?? [];
}

function formatCustodyInteger(value: CustodyExactInteger) {
  return formatCustodyExactInteger(value, currentUiLocale());
}

export function CustodyComponentResolver() {
  const { t } = useTranslation("review");
  const [actionError, setActionError] = useState<string | null>(null);
  const [pendingComponentId, setPendingComponentId] = useState<string | null>(null);
  const [editingComponent, setEditingComponent] = useState<CustodyComponent | null>(
    null,
  );
  // The embedded guided form submits through its own mutation observers, so
  // `mutationPending` below cannot see its in-flight state; track it explicitly
  // to keep the revise dialog from being dismissed mid-submit.
  const [editorBusy, setEditorBusy] = useState(false);
  const [editingForm, setEditingForm] = useState<GuidedComponentFormState | null>(null);
  const [creating, setCreating] = useState(false);
  const [creatorBusy, setCreatorBusy] = useState(false);

  const componentQuery = useDaemon<CustodyComponentListEnvelope>(
    "ui.transfers.components.list",
    { limit: 1000 },
  );
  const componentPlanMutation = useDaemonMutation<CustodyComponentMutationResult>(
    "ui.transfers.components.plan",
    { invalidateQueries: false },
  );
  const componentApplyMutation =
    useDaemonMutation<CustodyComponentMutationResult>(
      "ui.transfers.components.apply",
    );

  const components = useMemo(
    () => custodyComponentsFromEnvelope(componentQuery.data?.data),
    [componentQuery.data?.data],
  );
  const mutationPending =
    componentPlanMutation.isPending || componentApplyMutation.isPending;

  // The list can hold many components; render a capped window of compact rows
  // and expand a row's legs/allocations/audit only on demand.
  const [componentVisibleCount, setComponentVisibleCount] =
    useState(REVIEW_PAGE_SIZE);
  const [expandedComponents, setExpandedComponents] = useState<Set<string>>(
    new Set(),
  );
  const toggleComponentExpanded = (id: string) =>
    setExpandedComponents((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const visibleComponents = components.slice(0, componentVisibleCount);
  const hasMoreComponents = components.length > visibleComponents.length;

  const openRevisionEditor = (component: CustodyComponent) => {
    try {
      const form = componentToFormState(component);
      setEditingForm(form);
      setEditingComponent(component);
      setActionError(null);
    } catch (error) {
      setActionError(error instanceof Error && error.message === "guided_component_shape_unsupported"
        ? t("swap.components.form.unsupportedShape")
        : custodyMutationError(t, error));
    }
  };

  const closeRevisionEditor = () => {
    setEditingComponent(null);
    setEditingForm(null);
  };

  const mutateComponent = async (
    component: CustodyComponent,
    action: "activate" | "retire" | "supersede" | "undo",
  ) => {
    setPendingComponentId(component.id);
    setActionError(null);
    try {
      if (action === "activate" || action === "supersede" || action === "retire") {
        const planArgs = {
          action: action === "activate" ? "activate" : "supersede",
          component_id: component.id,
          reason:
            action === "retire"
              ? "desktop_custody_review_retire"
              : action === "supersede"
                ? "desktop_custody_review_supersede"
                : undefined,
        };
        const previewResponse = await componentPlanMutation.mutateAsync(planArgs);
        if (previewResponse.data?.input_version === undefined) {
          throw new Error(t("swap.components.backendError.unexpected"));
        }
        await componentApplyMutation.mutateAsync({
          ...planArgs,
          expected_input_version: previewResponse.data.input_version,
        });
      } else {
        const planArgs = {
          action: "undo",
          component_id: component.id,
          reason: "desktop_custody_review_undo",
        };
        const previewResponse = await componentPlanMutation.mutateAsync(planArgs);
        if (previewResponse.data?.input_version === undefined) {
          throw new Error(t("swap.components.backendError.unexpected"));
        }
        const response = await componentApplyMutation.mutateAsync({
          ...planArgs,
          expected_input_version: previewResponse.data.input_version,
        });
        if (
          response.data &&
          isGuidedEditableComponentType(response.data.component.component_type)
        ) {
          openRevisionEditor(response.data.component);
        }
      }
    } catch (error) {
      setActionError(custodyMutationError(t, error));
    } finally {
      setPendingComponentId(null);
    }
  };

  return (
    <div className="space-y-(--kb-page-gap)">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <p className={pageDescriptionClassName}>
          {t("swap.components.description")}
        </p>
        <Button type="button" size="sm" className="shrink-0" onClick={() => setCreating(true)}>
          <Plus className="size-4" aria-hidden="true" />
          {t("swap.components.new")}
        </Button>
      </div>

      {actionError ? (
        <div className="whitespace-pre-wrap rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          {actionError}
        </div>
      ) : null}

      <Card>
        <CardHeader>
          <CardTitle>{t("swap.components.listTitle")}</CardTitle>
          <CardDescription>{t("swap.components.listDescription")}</CardDescription>
        </CardHeader>
        <CardContent>
          {componentQuery.data?.data?.has_more ? (
            <div className="mb-3 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-400/30 dark:bg-amber-950/30 dark:text-amber-100">
              {t("swap.components.moreAvailable", {
                count: componentQuery.data.data.limit ?? 1000,
              })}
            </div>
          ) : null}
          {componentQuery.isLoading ? (
            <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
              <Loader2 className="animate-spin" />
              {t("swap.components.loading")}
            </div>
          ) : componentQuery.isError ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
              {t("swap.components.loadFailed", {
                error: custodyMutationError(t, componentQuery.error),
              })}
            </div>
          ) : components.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {t("swap.components.empty")}
            </p>
          ) : (
            <div className="space-y-3">
              {visibleComponents.map((component) => {
                const issues = component.validation?.issues ?? [];
                const pending = mutationPending && pendingComponentId === component.id;
                const isExpanded = expandedComponents.has(component.id);
                const legsById = new Map(
                  component.legs.map((leg) => [leg.id, leg] as const),
                );
                return (
                  <div key={component.id} className="rounded-lg border p-4">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="font-medium">
                            {t(`swap.components.type.${component.component_type}`, {
                              defaultValue: component.component_type,
                            })}
                          </span>
                          <Badge
                            variant={
                              component.effective_state === "active"
                                ? "default"
                                : component.state === "superseded"
                                  ? "outline"
                                  : "secondary"
                            }
                          >
                            {t(`swap.components.state.${component.effective_state}`)}
                          </Badge>
                          <span className="text-xs text-muted-foreground">
                            {t("swap.components.revision", { count: component.revision })}
                          </span>
                        </div>
                        <div className="mt-1 font-mono text-xs text-muted-foreground">
                          {compactRecordId(component.id)}
                        </div>
                        <button
                          type="button"
                          className="mt-1 flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
                          onClick={() => toggleComponentExpanded(component.id)}
                          aria-expanded={isExpanded}
                        >
                          {isExpanded ? (
                            <ChevronDown className="size-3.5" />
                          ) : (
                            <ChevronRight className="size-3.5" />
                          )}
                          {t("swap.components.legCount", {
                            count: component.legs.length,
                          })}
                        </button>
                        {isExpanded ? (
                        <div className="mt-2 flex flex-wrap gap-1.5 text-xs">
                          <Badge variant="outline">
                            {t("swap.components.audit.mode", {
                              mode: t(
                                `swap.components.mode.${component.conservation_mode}`,
                              ),
                            })}
                          </Badge>
                          <Badge variant="outline">
                            {t("swap.components.audit.authoredSource", {
                              source: t(
                                `swap.components.authoredSource.${component.authored_source}`,
                                { defaultValue: component.authored_source },
                              ),
                            })}
                          </Badge>
                          {component.evidence_kind || component.evidence_grade ? (
                            <Badge variant="outline">
                              {t("swap.components.audit.evidence", {
                                kind: component.evidence_kind ?? "—",
                                grade: component.evidence_grade ?? "—",
                              })}
                            </Badge>
                          ) : null}
                          {component.conservation_mode === "conversion" ? (
                            <>
                              <Badge variant="outline">
                                {t("swap.components.audit.policy", {
                                  policy:
                                    component.conversion_policy ??
                                    t("swap.components.audit.missing"),
                                })}
                              </Badge>
                              <Badge
                                variant={
                                  component.conversion_reviewed ? "default" : "secondary"
                                }
                              >
                                {t("swap.components.audit.reviewed", {
                                  value: component.conversion_reviewed
                                    ? t("swap.components.audit.yes")
                                    : t("swap.components.audit.no"),
                                })}
                              </Badge>
                            </>
                          ) : null}
                        </div>
                        ) : null}
                        {component.notes ? (
                          <p className="mt-2 text-sm text-muted-foreground">{component.notes}</p>
                        ) : null}
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {(component.state === "draft" ||
                          component.state === "active") &&
                        isGuidedEditableComponentType(component.component_type) ? (
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            disabled={mutationPending}
                            onClick={() => openRevisionEditor(component)}
                          >
                            <Pencil />
                            {component.state === "draft"
                              ? t("swap.components.editDraft")
                              : t("swap.components.revise")}
                          </Button>
                        ) : null}
                        {component.state === "draft" ? (
                          <>
                            <Button
                              type="button"
                              size="sm"
                              disabled={
                                mutationPending ||
                                component.validation?.activatable === false
                              }
                              onClick={() => void mutateComponent(component, "activate")}
                            >
                              {pending ? <Loader2 className="animate-spin" /> : <Check />}
                              {t("swap.components.activate")}
                            </Button>
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              disabled={mutationPending}
                              onClick={() => void mutateComponent(component, "retire")}
                            >
                              {pending ? (
                                <Loader2 className="animate-spin" />
                              ) : (
                                <Trash2 />
                              )}
                              {t("swap.components.retireDraft")}
                            </Button>
                          </>
                        ) : null}
                        {component.state === "active" ? (
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            disabled={mutationPending}
                            onClick={() => void mutateComponent(component, "supersede")}
                          >
                            {pending ? <Loader2 className="animate-spin" /> : <Unlink />}
                            {t("swap.components.supersede")}
                          </Button>
                        ) : null}
                        {component.state === "superseded" ? (
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            disabled={mutationPending}
                            onClick={() => void mutateComponent(component, "undo")}
                          >
                            {pending ? <Loader2 className="animate-spin" /> : <Undo2 />}
                            {t("swap.components.undo")}
                          </Button>
                        ) : null}
                      </div>
                    </div>

                    {isExpanded ? (
                    <>
                    <div className="mt-3 grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                      {component.legs.map((leg) => (
                        <div key={leg.id} className="rounded-md bg-muted/50 p-2 text-xs">
                          <div className="flex items-center justify-between gap-2">
                            <Badge variant="outline">
                              {custodyRoleLabel(t, leg.role)}
                            </Badge>
                            <span className="font-mono">
                              {t("swap.components.audit.amountMsat", {
                                amount: formatCustodyInteger(leg.amount_msat),
                              })}
                            </span>
                          </div>
                          <div className="mt-1 text-muted-foreground">
                            {leg.rail} · {leg.asset} · {leg.exposure ?? "—"} / {leg.conservation_unit ?? "—"}
                          </div>
                          {leg.valuation_unit && leg.valuation_amount !== null && leg.valuation_amount !== undefined ? (
                            <div className="mt-1 text-muted-foreground">
                              {t("swap.components.audit.valuation", {
                                amount: formatCustodyInteger(leg.valuation_amount),
                                unit: leg.valuation_unit,
                              })}
                            </div>
                          ) : null}
                          <div className="mt-1 truncate font-mono text-muted-foreground">
                            {compactRecordId(
                              leg.transaction_id ?? leg.wallet_id ?? "",
                            )}
                          </div>
                        </div>
                      ))}
                    </div>

                    {component.allocations.length > 0 ? (
                      <div className="mt-3 rounded-md border p-3 text-xs">
                        <div className="font-medium">
                          {t("swap.components.audit.allocations")}
                        </div>
                        <div className="mt-2 space-y-1.5">
                          {component.allocations.map((allocation) => {
                            const source = legsById.get(allocation.source_leg_id);
                            const sink = legsById.get(allocation.sink_leg_id);
                            return (
                              <div
                                key={allocation.id}
                                className="rounded-md bg-muted/50 px-2 py-1.5 font-mono"
                              >
                                {t("swap.components.audit.allocationEdge", {
                                  source: source
                                    ? `${source.ordinal + 1} (${custodyRoleLabel(t, source.role)})`
                                    : compactRecordId(allocation.source_leg_id),
                                  sourceAmount: formatCustodyInteger(
                                    allocation.source_amount_msat,
                                  ),
                                  sink: sink
                                    ? `${sink.ordinal + 1} (${custodyRoleLabel(t, sink.role)})`
                                    : compactRecordId(allocation.sink_leg_id),
                                  sinkAmount: formatCustodyInteger(
                                    allocation.sink_amount_msat,
                                  ),
                                })}
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    ) : null}

                    {issues.length > 0 ? (
                      <div className="mt-3 rounded-md border border-amber-300/60 bg-amber-50 p-3 text-xs text-amber-950 dark:border-amber-400/30 dark:bg-amber-950/30 dark:text-amber-100">
                        <div className="font-medium">{t("swap.components.validationIssues")}</div>
                        <ul className="mt-1 list-disc space-y-1 pl-4">
                          {issues.map((issue, index) => (
                            <li key={`${issue.code ?? "issue"}:${index}`}>
                              {custodyBackendIssueText(t, issue)}
                            </li>
                          ))}
                        </ul>
                      </div>
                    ) : null}
                    </>
                    ) : null}
                  </div>
                );
              })}
              {hasMoreComponents ? (
                <div className="flex justify-center pt-1">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      setComponentVisibleCount((count) => count + REVIEW_PAGE_SIZE)
                    }
                  >
                    {t("swap.loadMore")}
                  </Button>
                </div>
              ) : null}
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog
        open={editingComponent !== null}
        onOpenChange={(open) => {
          if (!open && !mutationPending && !editorBusy) closeRevisionEditor();
        }}
      >
        {/* The base dialog is pinned between the window inset and the bottom
            edge, so it always spans the viewport height; explicit rows keep
            the header and action footer fixed and give the form the rest as a
            scroll region instead of a blank gap. `sm:max-w-4xl` must carry the
            same breakpoint as the primitive's `sm:max-w-lg` to override it. */}
        <DialogContent className="grid w-[calc(100vw-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-4xl xl:max-w-5xl">
          <DialogHeader className="border-b px-(--kb-card-padding) py-4 pr-12 text-left">
            <DialogTitle>
              {editingComponent?.state === "active"
                ? t("swap.components.revisionDialog.reviseTitle")
                : t("swap.components.revisionDialog.editTitle")}
            </DialogTitle>
            <DialogDescription className="max-w-3xl">
              {t("swap.components.revisionDialog.description")}
            </DialogDescription>
          </DialogHeader>
          {editingComponent && editingForm ? (
            <GuidedComponentForm
              key={editingComponent.id}
              variant="embedded"
              edit={{
                componentId: editingComponent.id,
                state: editingComponent.state === "active" ? "active" : "draft",
              }}
              initialForm={editingForm}
              onDone={closeRevisionEditor}
              onBusyChange={setEditorBusy}
            />
          ) : null}
        </DialogContent>
      </Dialog>
      <Dialog open={creating} onOpenChange={(open) => (creatorBusy ? null : setCreating(open))}>
        <DialogContent className="grid w-[calc(100vw-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-4xl xl:max-w-5xl">
          <DialogHeader className="border-b px-(--kb-card-padding) py-4 pr-12 text-left">
            <DialogTitle>{t("swap.components.newTitle")}</DialogTitle>
            <DialogDescription className="max-w-3xl">{t("swap.components.newDescription")}</DialogDescription>
          </DialogHeader>
          {creating ? (
            <GuidedComponentForm
              variant="embedded"
              onDone={() => setCreating(false)}
              onBusyChange={setCreatorBusy}
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** A persisted pair's fee, shaped for {@link SwapFeeText} / {@link feePercent}. */
