/**
 * A tool call waiting for the user's approval, as a row in the chat work log
 * beside the call it belongs to: what will run, the plan or arguments the
 * daemon recomputed, what makes it risky, then "Waiting for approval ·
 * Decline · Approve · More". It replaces the old approval dialog, so the turn
 * stays readable while it waits.
 *
 * The layout follows T3 Code's pending-approval row and its approval actions
 * (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/TimelinePendingApprovalRow.tsx` and
 * `ComposerPendingApprovalActions.tsx`. Reimplemented with Kassiber's own
 * primitives; the approval rules (once-only tools, plan and review previews
 * that must be current) are Kassiber's.
 */

import * as React from "react";
import {
  ChevronDown,
  FileUp,
  Globe,
  ListChecks,
  RotateCcw,
  Shield,
  SquarePen,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type {
  AiToolConsentDecision,
  AiToolConsentRequest,
} from "@/daemon/stream";
import { cn } from "@/lib/utils";

import { AssistantSessionContext } from "./assistantSession";
import { PlanConsentBody } from "./PlanConsentBody";
import {
  PLAN_BOUND_CONSENT_TOOLS,
  planApprovalAvailable,
  planConsentPreview,
} from "./planConsent";
import { ReviewProposalView, ReviewReceiptCard } from "./ReviewWorkflowCard";
import {
  reviewApprovalAvailable,
  reviewArtifact,
  reviewReceipt,
} from "./reviewWorkflow";

/** Marks a pending-approval row, so focus handling can find its siblings. */
const PENDING_APPROVAL_ATTRIBUTE = "data-pending-approval";
/** Marks an assistant surface (panel, page) whose composer takes focus back. */
const ASSISTANT_SURFACE_ATTRIBUTE = "data-assistant-surface";

type ApprovalKind = "review" | "import" | "rollback" | "tool";

function approvalKind(request: AiToolConsentRequest): ApprovalKind {
  if (request.name === "ui.review.apply") return "review";
  if (PLAN_BOUND_CONSENT_TOOLS.has(request.name)) {
    return request.name === "ui.imports.rollback" ? "rollback" : "import";
  }
  return "tool";
}

const KIND_ICON: Record<ApprovalKind, LucideIcon> = {
  review: ListChecks,
  import: FileUp,
  rollback: RotateCcw,
  tool: SquarePen,
};

/**
 * The approve label: plan-bound and review approvals say exactly what runs
 * once; any other call is a plain "Approve".
 */
const APPROVE_LABEL_KEYS = {
  review: "consent.apply.confirm",
  reviewApplied: "consent.apply.returnReceipt",
  import: "consent.plan.confirmImport",
  rollback: "consent.plan.confirmRollback",
  tool: "consent.approve",
} as const;

/** The proposal (with its per-year tax delta) the review approval covers. */
export function ReviewConsentBody({ request }: { request: AiToolConsentRequest }) {
  const { t } = useTranslation("assistant");
  const preview = request.reviewPreview;
  const artifact = preview?.status === "ready" ? reviewArtifact(preview.artifact) : null;
  const receipt = preview?.status === "applied" ? reviewReceipt(preview.receipt) : null;
  if (artifact) return <ReviewProposalView artifact={artifact} />;
  if (receipt) {
    return (
      <>
        <p className="text-sm">{t("consent.apply.alreadyApplied")}</p>
        <ReviewReceiptCard receipt={receipt} />
      </>
    );
  }
  return (
    <p role="alert" className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
      {t("consent.apply.unavailable")}
      {preview?.status === "unavailable" ? (
        <span className="mt-2 block font-mono text-xs">{preview.code}</span>
      ) : null}
    </p>
  );
}

/** Focus the user is using elsewhere, which a new approval must not take. */
function focusIsTaken(row: HTMLElement): boolean {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || active === document.body) return false;
  if (row.contains(active)) return false;
  // Typing: text in the composer, or in any field on the page. An empty
  // composer (the question was just sent) is not typing.
  if (active.isContentEditable || active instanceof HTMLSelectElement) return true;
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
    return active.value.length > 0;
  }
  // Another approval already has it.
  if (active.closest(`[${PENDING_APPROVAL_ATTRIBUTE}]`)) return true;
  // A menu, or a dialog other than the sheet this row sits in.
  const layer = active.closest(
    '[role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"]',
  );
  return Boolean(layer && !layer.contains(row));
}

/**
 * Where focus goes once this row is answered: the next waiting approval, or
 * the composer of the surface the row is in.
 */
function nextFocusTarget(row: HTMLElement | null): HTMLElement | null {
  if (!row) return null;
  const other = Array.from(
    document.querySelectorAll<HTMLElement>(`[${PENDING_APPROVAL_ATTRIBUTE}]`),
  ).find((element) => element !== row);
  if (other) return other;
  return (
    row
      .closest(`[${ASSISTANT_SURFACE_ATTRIBUTE}]`)
      ?.querySelector<HTMLElement>("textarea:not([disabled])") ?? null
  );
}

const INLINE_BUTTON_CLASS =
  "kb-hit-floor inline-flex shrink-0 cursor-pointer items-center gap-0.5 rounded-sm font-medium underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50";

const INLINE_TONE = {
  default: "text-foreground",
  muted: "text-muted-foreground hover:text-foreground",
  destructive: "text-destructive/85 hover:text-destructive",
} as const;

export function PendingApprovalRow({ request }: { request: AiToolConsentRequest }) {
  const { t } = useTranslation("assistant");
  const assistant = React.useContext(AssistantSessionContext);
  const rowRef = React.useRef<HTMLDivElement>(null);
  const labelId = React.useId();
  const reasonId = React.useId();
  const risksId = React.useId();
  const [responding, setResponding] = React.useState(false);
  const handedOffRef = React.useRef(false);
  const mountedRef = React.useRef(true);
  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const kind = approvalKind(request);
  const planPreview =
    kind === "import" || kind === "rollback"
      ? planConsentPreview(request.name, request.argumentsPreview)
      : null;
  // Plan-bound and review approvals run exactly the plan the daemon showed;
  // without a current one there is nothing to approve.
  const canApprove =
    kind === "review"
      ? reviewApprovalAvailable(request.reviewPreview)
      : kind === "tool"
        ? true
        : planApprovalAvailable(planPreview);
  const canAllowSession = canApprove && request.canAllowSession;
  const hasArguments = Object.keys(request.argumentsPreview).length > 0;
  const Icon = request.egresses && kind === "tool" ? Globe : KIND_ICON[kind];
  const approveLabel = t(
    kind === "review" && request.reviewPreview?.status === "applied"
      ? APPROVE_LABEL_KEYS.reviewApplied
      : APPROVE_LABEL_KEYS[kind],
  );
  const reason =
    kind === "review"
      ? t("consent.apply.description")
      : kind === "tool"
        ? null
        : t("consent.plan.description");
  const risks = [
    request.egresses ? t("consent.risk.network") : null,
    request.reversible ? null : t("consent.risk.permanent"),
  ].filter((risk): risk is string => Boolean(risk));

  // Focus moves here when the row appears, unless the user is typing or busy
  // elsewhere; the thread already keeps the newest turn in view.
  React.useEffect(() => {
    const row = rowRef.current;
    if (!row || focusIsTaken(row)) return;
    row.focus({ preventScroll: true });
  }, []);

  // The row leaves as soon as it is answered: when the user answered from it
  // (or its More menu), focus moves on first so it does not fall to the page.
  const handOffFocus = (fromMenu: boolean) => {
    const row = rowRef.current;
    if (!row || (!fromMenu && !row.contains(document.activeElement))) return;
    handedOffRef.current = fromMenu;
    nextFocusTarget(row)?.focus({ preventScroll: true });
  };

  const decide = async (decision: AiToolConsentDecision, fromMenu = false) => {
    if (responding || !assistant) return;
    if (decision !== "deny" && !canApprove) return;
    if (decision === "allow_session" && !canAllowSession) return;
    setResponding(true);
    handOffFocus(fromMenu);
    try {
      await assistant.sendConsent(request.callId, decision);
    } finally {
      if (mountedRef.current) setResponding(false);
    }
  };

  const stopAnswer = () => {
    if (!assistant) return;
    handOffFocus(true);
    assistant.abort();
  };

  const disabled = responding || !assistant;
  const body =
    kind === "review" ? (
      <ReviewConsentBody request={request} />
    ) : kind === "import" || kind === "rollback" ? (
      <PlanConsentBody preview={planPreview} />
    ) : null;

  return (
    <div
      ref={rowRef}
      role="group"
      aria-labelledby={labelId}
      aria-describedby={
        [reason ? reasonId : null, risks.length ? risksId : null]
          .filter(Boolean)
          .join(" ") || undefined
      }
      tabIndex={-1}
      data-pending-approval={request.callId}
      className="mb-2 w-full min-w-0 rounded-md px-0.5 py-0.5 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/70"
    >
      <div className="flex min-h-6 min-w-0 items-start gap-1.5 leading-relaxed">
        <span className="flex size-6 shrink-0 items-center justify-center">
          <Icon className="size-4 text-muted-foreground" aria-hidden="true" />
        </span>
        <div className="min-w-0 flex-1 pt-0.5">
          <p id={labelId} className="break-words font-medium text-foreground">
            <span className="sr-only">{t("consent.rowLabel")}: </span>
            {request.summary}
          </p>
          <code className="block truncate font-mono text-2xs text-muted-foreground">
            {request.name}
          </code>
        </div>
      </div>
      <div className="ms-7.5 flex min-w-0 flex-col gap-1.5 pb-1 text-xs">
        {reason ? (
          <span id={reasonId} className="select-text text-muted-foreground">
            {reason}
          </span>
        ) : null}
        {body ? (
          <Collapsible defaultOpen className="min-w-0">
            <CollapsibleTrigger className="group inline-flex items-center gap-1 rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
              <ChevronDown
                className="size-3 transition-transform group-data-[state=closed]:-rotate-90"
                aria-hidden="true"
              />
              {t(kind === "review" ? "consent.proposalDetails" : "consent.planDetails")}
            </CollapsibleTrigger>
            <CollapsibleContent>
              <div
                className="mt-1.5 max-h-[min(26rem,55vh)] select-text overflow-y-auto overscroll-contain rounded-md border border-border/60 bg-background/60 p-3 text-sm"
                tabIndex={0}
                aria-label={t(kind === "review" ? "consent.proposalDetails" : "consent.planDetails")}
              >
                {body}
              </div>
            </CollapsibleContent>
          </Collapsible>
        ) : hasArguments ? (
          <code
            aria-label={t("consent.arguments")}
            tabIndex={0}
            className="block max-h-40 min-w-0 select-text overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/55 px-2 py-1 font-mono text-2xs leading-5 text-foreground [scrollbar-width:thin] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/70"
          >
            {JSON.stringify(request.argumentsPreview, null, 2)}
          </code>
        ) : null}
        {risks.length > 0 ? (
          <span
            id={risksId}
            className="inline-flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-amber-700 dark:text-amber-400"
          >
            <TriangleAlert className="size-3 shrink-0" aria-hidden="true" />
            {risks.map((risk, index) => (
              <React.Fragment key={risk}>
                {index > 0 ? (
                  <span aria-hidden="true" className="text-muted-foreground">
                    ·
                  </span>
                ) : null}
                <span>{risk}</span>
              </React.Fragment>
            ))}
          </span>
        ) : null}
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="inline-flex items-center gap-1 text-amber-700 dark:text-amber-400">
            <Shield className="size-3 shrink-0" aria-hidden="true" />
            {t("consent.waiting")}
          </span>
          <span aria-hidden="true" className="text-muted-foreground">
            ·
          </span>
          <button
            type="button"
            className={cn(INLINE_BUTTON_CLASS, INLINE_TONE.muted)}
            disabled={disabled}
            onClick={() => void decide("deny")}
          >
            {t("consent.decline")}
          </button>
          <button
            type="button"
            className={cn(
              INLINE_BUTTON_CLASS,
              kind === "rollback" ? INLINE_TONE.destructive : INLINE_TONE.default,
            )}
            disabled={disabled || !canApprove}
            onClick={() => void decide("allow_once")}
          >
            {approveLabel}
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger
              className={cn(INLINE_BUTTON_CLASS, INLINE_TONE.muted)}
              disabled={disabled}
              aria-label={t("consent.moreOptions")}
            >
              {t("consent.more")}
              <ChevronDown className="size-3" aria-hidden="true" />
            </DropdownMenuTrigger>
            <DropdownMenuContent
              side="top"
              align="start"
              className="min-w-44"
              onCloseAutoFocus={(event) => {
                // The choice moved focus on already (the row is leaving).
                if (!handedOffRef.current) return;
                handedOffRef.current = false;
                event.preventDefault();
              }}
            >
              {canAllowSession ? (
                <DropdownMenuItem onSelect={() => void decide("allow_session", true)}>
                  {t("consent.allowChat")}
                </DropdownMenuItem>
              ) : null}
              <DropdownMenuItem variant="destructive" onSelect={stopAnswer}>
                {t("consent.stopAnswer")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </div>
  );
}
