/**
 * Shared translation + rendering helpers for custody-component validation.
 *
 * Extracted so both the components list/resolver and the guided component form
 * can render the same issue text without a circular import between route and
 * form modules. Local (client-side) issue codes come from
 * {@link previewCustodyComponentBatch}; backend issue/error codes come from the
 * daemon's `ui.transfers.components.*` responses.
 */
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";

import { DaemonRequestError } from "@/daemon/client";
import { cn } from "@/lib/utils";
import type {
  CustodyPreviewIssue,
  CustodyPreviewIssueCode,
} from "@/lib/custodyComponentBulk";

export interface CustodyValidationIssue {
  code?: string;
  message?: string;
  [key: string]: unknown;
}

const CUSTODY_LOCAL_ISSUE_KEYS = {
  jsonInvalid: "swap.components.localIssue.jsonInvalid",
  documentRequired: "swap.components.localIssue.documentRequired",
  componentObjectRequired: "swap.components.localIssue.componentObjectRequired",
  componentTypeUnsupported: "swap.components.localIssue.componentTypeUnsupported",
  conservationModeUnsupported:
    "swap.components.localIssue.conservationModeUnsupported",
  legsRequired: "swap.components.localIssue.legsRequired",
  legObjectRequired: "swap.components.localIssue.legObjectRequired",
  roleUnsupported: "swap.components.localIssue.roleUnsupported",
  legIdDuplicate: "swap.components.localIssue.legIdDuplicate",
  amountInvalid: "swap.components.localIssue.amountInvalid",
  transactionlessWalletRequired:
    "swap.components.localIssue.transactionlessWalletRequired",
  transactionlessTimeRequired:
    "swap.components.localIssue.transactionlessTimeRequired",
  valuationPairRequired: "swap.components.localIssue.valuationPairRequired",
  valuationAmountInvalid: "swap.components.localIssue.valuationAmountInvalid",
  valuationTokenInvalid: "swap.components.localIssue.valuationTokenInvalid",
  conversionPolicyInvalid: "swap.components.localIssue.conversionPolicyInvalid",
  conversionReviewedInvalid:
    "swap.components.localIssue.conversionReviewedInvalid",
  conversionValuationRequired:
    "swap.components.localIssue.conversionValuationRequired",
  sourceRequired: "swap.components.localIssue.sourceRequired",
  ownedDestinationRequired: "swap.components.localIssue.ownedDestinationRequired",
  anchorRequired: "swap.components.localIssue.anchorRequired",
  suspenseReviewRequired: "swap.components.localIssue.suspenseReviewRequired",
  suspenseQuantityModeRequired:
    "swap.components.localIssue.suspenseQuantityModeRequired",
  suspenseLocationInvalid: "swap.components.localIssue.suspenseLocationInvalid",
  suspenseTimeRequired: "swap.components.localIssue.suspenseTimeRequired",
  suspenseAllocationRequired:
    "swap.components.localIssue.suspenseAllocationRequired",
  suspenseObservedSourceRequired:
    "swap.components.localIssue.suspenseObservedSourceRequired",
  suspenseAssetMismatch: "swap.components.localIssue.suspenseAssetMismatch",
  suspenseTimeMismatch: "swap.components.localIssue.suspenseTimeMismatch",
  quantityUnbalanced: "swap.components.localIssue.quantityUnbalanced",
  conversionReviewRequired: "swap.components.localIssue.conversionReviewRequired",
  conversionValuationUnbalanced:
    "swap.components.localIssue.conversionValuationUnbalanced",
  conversionTopologyUnsupported:
    "swap.components.localIssue.conversionTopologyUnsupported",
  allocationsInvalid: "swap.components.localIssue.allocationsInvalid",
  allocationsRequired: "swap.components.localIssue.allocationsRequired",
  allocationObjectRequired: "swap.components.localIssue.allocationObjectRequired",
  allocationSourceInvalid: "swap.components.localIssue.allocationSourceInvalid",
  allocationSinkInvalid: "swap.components.localIssue.allocationSinkInvalid",
  allocationAmountInvalid: "swap.components.localIssue.allocationAmountInvalid",
  allocationEdgeDuplicate: "swap.components.localIssue.allocationEdgeDuplicate",
  allocationQuantityMismatch:
    "swap.components.localIssue.allocationQuantityMismatch",
  allocationSourceCoverage: "swap.components.localIssue.allocationSourceCoverage",
  allocationSinkCoverage: "swap.components.localIssue.allocationSinkCoverage",
} as const satisfies Record<CustodyPreviewIssueCode, string>;

const CUSTODY_BACKEND_ISSUE_KEYS = {
  active_lineage_conflict: "swap.components.backendIssue.active_lineage_conflict",
  active_transaction_membership_conflict:
    "swap.components.backendIssue.active_transaction_membership_conflict",
  allocation_leg_invalid: "swap.components.backendIssue.allocation_leg_invalid",
  allocation_network_mismatch:
    "swap.components.backendIssue.allocation_network_mismatch",
  allocation_network_scope_invalid:
    "swap.components.backendIssue.allocation_network_scope_invalid",
  allocation_quantity_mismatch:
    "swap.components.backendIssue.allocation_quantity_mismatch",
  allocation_required: "swap.components.backendIssue.allocation_required",
  allocation_sink_coverage_mismatch:
    "swap.components.backendIssue.allocation_sink_coverage_mismatch",
  allocation_source_coverage_mismatch:
    "swap.components.backendIssue.allocation_source_coverage_mismatch",
  anchor_asset_mismatch: "swap.components.backendIssue.anchor_asset_mismatch",
  anchor_chain_mismatch: "swap.components.backendIssue.anchor_chain_mismatch",
  anchor_coverage_mismatch: "swap.components.backendIssue.anchor_coverage_mismatch",
  anchor_network_mismatch: "swap.components.backendIssue.anchor_network_mismatch",
  anchor_occurred_at_mismatch:
    "swap.components.backendIssue.anchor_occurred_at_mismatch",
  anchor_rail_mismatch: "swap.components.backendIssue.anchor_rail_mismatch",
  anchor_transaction_identity_mismatch:
    "swap.components.backendIssue.anchor_transaction_identity_mismatch",
  anchor_transaction_missing:
    "swap.components.backendIssue.anchor_transaction_missing",
  anchor_transaction_excluded:
    "swap.components.backendIssue.anchor_transaction_excluded",
  anchor_transaction_retracted:
    "swap.components.backendIssue.anchor_transaction_retracted",
  anchor_wallet_mismatch: "swap.components.backendIssue.anchor_wallet_mismatch",
  component_allocation_count_mismatch:
    "swap.components.backendIssue.component_allocation_count_mismatch",
  component_content_commitment_missing:
    "swap.components.backendIssue.component_content_commitment_missing",
  component_leg_count_mismatch:
    "swap.components.backendIssue.component_leg_count_mismatch",
  conversion_not_reviewed: "swap.components.backendIssue.conversion_not_reviewed",
  conversion_fee_quantity_mismatch:
    "swap.components.backendIssue.conversion_fee_quantity_mismatch",
  conversion_fee_valuation_mismatch:
    "swap.components.backendIssue.conversion_fee_valuation_mismatch",
  conversion_policy_missing:
    "swap.components.backendIssue.conversion_policy_missing",
  conversion_topology_unsupported:
    "swap.components.backendIssue.conversion_topology_unsupported",
  conversion_valuation_missing:
    "swap.components.backendIssue.conversion_valuation_missing",
  custody_component_value_only_loss_unsupported:
    "swap.components.backendIssue.custody_component_value_only_loss_unsupported",
  custody_component_fee_orphaned:
    "swap.components.backendIssue.custody_component_fee_orphaned",
  custody_location_continuity_mismatch:
    "swap.components.backendIssue.custody_location_continuity_mismatch",
  destination_anchor_direction_mismatch:
    "swap.components.backendIssue.destination_anchor_direction_mismatch",
  fee_source_asset_mismatch:
    "swap.components.backendIssue.fee_source_asset_mismatch",
  fee_source_scope_mismatch:
    "swap.components.backendIssue.fee_source_scope_mismatch",
  fee_source_wallet_mismatch:
    "swap.components.backendIssue.fee_source_wallet_mismatch",
  leg_occurred_at_invalid: "swap.components.backendIssue.leg_occurred_at_invalid",
  leg_occurred_at_missing: "swap.components.backendIssue.leg_occurred_at_missing",
  loss_anchor_direction_mismatch:
    "swap.components.backendIssue.loss_anchor_direction_mismatch",
  missing_owned_destination:
    "swap.components.backendIssue.missing_owned_destination",
  missing_source: "swap.components.backendIssue.missing_source",
  no_legs: "swap.components.backendIssue.no_legs",
  owned_leg_wallet_missing:
    "swap.components.backendIssue.owned_leg_wallet_missing",
  revision_link_invalid: "swap.components.backendIssue.revision_link_invalid",
  revision_link_missing: "swap.components.backendIssue.revision_link_missing",
  source_anchor_direction_mismatch:
    "swap.components.backendIssue.source_anchor_direction_mismatch",
  transaction_anchor_missing:
    "swap.components.backendIssue.transaction_anchor_missing",
  transactionless_leg_wallet_missing:
    "swap.components.backendIssue.transactionless_leg_wallet_missing",
  unbalanced_conversion_valuation:
    "swap.components.backendIssue.unbalanced_conversion_valuation",
  unbalanced_quantity: "swap.components.backendIssue.unbalanced_quantity",
  unresolved_value: "swap.components.backendIssue.unresolved_value",
} as const;

const CUSTODY_BACKEND_ERROR_KEYS = {
  custody_component_anchor_time_mismatch:
    "swap.components.backendError.custody_component_anchor_time_mismatch",
  custody_component_draft_exists:
    "swap.components.backendError.custody_component_draft_exists",
  custody_component_incomplete:
    "swap.components.backendError.custody_component_incomplete",
  custody_component_lineage_exists:
    "swap.components.backendError.custody_component_lineage_exists",
  custody_component_membership_conflict:
    "swap.components.backendError.custody_component_membership_conflict",
  custody_component_not_superseded:
    "swap.components.backendError.custody_component_not_superseded",
  custody_component_scope_mismatch:
    "swap.components.backendError.custody_component_scope_mismatch",
  custody_component_state_conflict:
    "swap.components.backendError.custody_component_state_conflict",
  custody_component_superseded:
    "swap.components.backendError.custody_component_superseded",
  custody_component_validation:
    "swap.components.backendError.custody_component_validation",
  conflict: "swap.components.backendError.conflict",
  not_found: "swap.components.backendError.not_found",
  validation: "swap.components.backendError.validation",
} as const;

export function custodyPreviewIssueText(
  t: TFunction<"review">,
  issue: CustodyPreviewIssue,
) {
  return t(CUSTODY_LOCAL_ISSUE_KEYS[issue.code], issue.values ?? {});
}

export function custodyBackendIssueText(
  t: TFunction<"review">,
  issue: CustodyValidationIssue,
) {
  const code = issue.code ?? "";
  const key =
    CUSTODY_BACKEND_ISSUE_KEYS[code as keyof typeof CUSTODY_BACKEND_ISSUE_KEYS];
  return key
    ? t(key)
    : t("swap.components.backendIssue.unknown", {
        code: code || t("swap.components.unknownIssue"),
      });
}

function isUnknownRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function validationIssuesFromDetails(details: unknown): CustodyValidationIssue[] {
  if (!isUnknownRecord(details)) return [];
  const validation = details.validation;
  if (!isUnknownRecord(validation) || !Array.isArray(validation.issues)) return [];
  return validation.issues.filter(isUnknownRecord) as CustodyValidationIssue[];
}

export function custodyMutationError(t: TFunction<"review">, error: unknown) {
  if (error instanceof DaemonRequestError) {
    const code = error.envelope.error?.code ?? "";
    const key =
      CUSTODY_BACKEND_ERROR_KEYS[code as keyof typeof CUSTODY_BACKEND_ERROR_KEYS];
    const base = key
      ? t(key)
      : t("swap.components.backendError.unknown", {
          code: code || t("swap.components.unknownIssue"),
        });
    const issues = validationIssuesFromDetails(error.envelope.error?.details);
    return [base, ...issues.map((issue) => custodyBackendIssueText(t, issue))].join(
      "\n",
    );
  }
  return t("swap.components.backendError.unexpected");
}

export function custodyRoleLabel(t: TFunction<"review">, role: string) {
  const labels: Record<string, string> = {
    source: t("swap.components.role.source"),
    destination: t("swap.components.role.destination"),
    fee: t("swap.components.role.fee"),
    external: t("swap.components.role.external"),
    retained: t("swap.components.role.retained"),
    suspense: t("swap.components.role.suspense"),
    unresolved: t("swap.components.role.unresolved"),
  };
  return labels[role] ?? t("swap.components.role.unknown", { role });
}

export function CustodyErrorList({
  title,
  issues,
  destructive = false,
}: {
  title: string;
  issues: CustodyPreviewIssue[];
  destructive?: boolean;
}) {
  const { t } = useTranslation("review");
  return (
    <div
      className={cn(
        "rounded-md border p-3 text-sm",
        destructive
          ? "border-destructive/40 bg-destructive/10 text-destructive"
          : "border-amber-300/60 bg-amber-50 text-amber-950 dark:border-amber-400/30 dark:bg-amber-950/30 dark:text-amber-100",
      )}
    >
      <div className="font-medium">{title}</div>
      <ul className="mt-1 list-disc space-y-1 pl-5">
        {issues.map((issue, index) => (
          <li key={`${index}:${issue.code}`}>
            {custodyPreviewIssueText(t, issue)}
          </li>
        ))}
      </ul>
    </div>
  );
}
