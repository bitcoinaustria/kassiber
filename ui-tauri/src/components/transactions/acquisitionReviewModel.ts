import { reviewArtifact, type ReviewArtifact } from "@/components/ai/reviewWorkflow";

export type AcquisitionScope = { workspace_id: string; profile_id: string; input_version: number };

/** Delayed reads cannot carry a declaration across a book/session change. */
export async function planAcquisitionReview({ readScope, createPlan, isCurrent, transactionId, kind, invalidMessage }: {
  readScope: () => Promise<AcquisitionScope | undefined>;
  createPlan: (args: Record<string, unknown>) => Promise<unknown>;
  isCurrent: () => boolean;
  transactionId: string;
  kind: string | null;
  invalidMessage: string;
}): Promise<ReviewArtifact | null> {
  const scope = await readScope();
  if (!isCurrent()) return null;
  if (!scope) throw new Error(invalidMessage);
  const result = await createPlan({ expected_scope: { workspace_id: scope.workspace_id, profile_id: scope.profile_id },
    expected_input_version: scope.input_version, operations: [{ type: "kind_override", transaction_id: transactionId,
      kind, reason: "Reviewed acquisition classification" }] });
  if (!isCurrent()) return null;
  const artifact = reviewArtifact(result, true);
  if (!artifact || artifact.workspace_id !== scope.workspace_id || artifact.profile_id !== scope.profile_id ||
      artifact.base_input_version !== scope.input_version) throw new Error(invalidMessage);
  return artifact;
}
