/**
 * A dry run of a pairing change in the review core (`ui.review.plan`), and
 * applying exactly that run once the owner confirms (`ui.review.apply`).
 * The core plans on a copy of the book and recalculates its journals, so the
 * preview names what really changes: which side is held, whether reports
 * are blocked and how each tax year moves. Applying rebuilds the journals in
 * the same step.
 *
 * Plans go out as previews (`preview: true`): the daemon runs one on a worker
 * and keeps answering other requests, and a newer preview cancels the one
 * still running (it answers `review_plan_cancelled`, which nobody waits for
 * any more).
 */
import * as React from "react";

import { reviewArtifact, reviewTaxDelta, type ReviewArtifact } from "@/components/ai/reviewWorkflow";
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import type { ProfilesSnapshot } from "@/mocks/profiles";
import { useUiStore } from "@/store/ui";

export type ReviewOperation = Record<string, unknown>;

export type PreviewState =
  | { status: "idle" }
  | { status: "checking"; startedAt: number }
  | { status: "ready"; artifact: ReviewArtifact; key: string }
  | { status: "failed"; message: string; /** The daemon's error code, when it refused. */ code: string | null };

interface ApplyResult {
  operations?: Array<{ type?: string; result?: Record<string, unknown> }>;
}

const messageOf = (cause: unknown, fallback: string) => (cause instanceof Error && cause.message ? cause.message : fallback);

/** The daemon's error code behind a failed request, or null for anything else. */
export function daemonErrorCode(cause: unknown): string | null {
  return cause instanceof DaemonRequestError ? (cause.envelope.error?.code ?? null) : null;
}

/** The daemon's answer to a preview it stopped because a newer one started. */
const PREVIEW_CANCELLED = "review_plan_cancelled";

/** Wait this long after the last change before planning, so a quick second choice starts one run. */
const SETTLE_MS = 500;

/**
 * Finished plans by book, input version and operations: cancelling and
 * reopening the same change, or going back to a choice already previewed,
 * reuses the run instead of recalculating the book again. A changed book
 * has a new input version, so it never reuses a stale plan.
 */
const finished = new Map<string, ReviewArtifact>();
const FINISHED_LIMIT = 20;

/**
 * Plans still running, by the same key. The core recalculates the whole
 * book for one, so closing a dialog and opening the same change again joins
 * the run that is under way instead of starting a second behind it.
 */
const running = new Map<string, { promise: Promise<ReviewArtifact | null>; startedAt: number }>();

/** Forget every plan (tests start each case from a clean book). */
export function forgetFinishedPreviews() {
  finished.clear();
  running.clear();
}

function remember(cacheKey: string, artifact: ReviewArtifact) {
  finished.delete(cacheKey);
  finished.set(cacheKey, artifact);
  while (finished.size > FINISHED_LIMIT) finished.delete(finished.keys().next().value as string);
}

/**
 * Plans `operations` whenever they change (null plans nothing) and again on
 * `retry`. `apply` sends the planned artifact with one idempotency key, so a
 * retry after a lost answer applies it once.
 */
export function useReviewPreview(operations: ReviewOperation[] | null, failedText: string) {
  const quiet = { invalidateQueries: false };
  const profiles = useDaemon<ProfilesSnapshot>("ui.profiles.snapshot");
  const journals = useDaemonMutation<{ status?: { journalInputVersion?: unknown } }>("ui.journals.snapshot", quiet);
  const cases = useDaemonMutation<{ input_version: number }>("ui.review.cases", quiet);
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", quiet);
  const applyMutation = useDaemonMutation<ApplyResult>("ui.review.apply");
  const [state, setState] = React.useState<PreviewState>({ status: "idle" });
  const run = React.useRef(0);
  const [attempt, setAttempt] = React.useState(0);
  const snapshot = profiles.data?.data;
  const workspaceId = snapshot?.activeWorkspaceId;
  const profileId = snapshot?.activeProfileId;
  const key = operations ? JSON.stringify(operations) : null;

  React.useEffect(() => {
    const token = ++run.current;
    if (!key) {
      setState({ status: "idle" });
      return;
    }
    setState({ status: "checking", startedAt: Date.now() });
    if (!workspaceId || !profileId) return;
    const expected_scope = { workspace_id: workspaceId, profile_id: profileId };
    const session = useUiStore.getState().daemonSession;
    // The plan binds to the book's input version. The journal snapshot reads
    // it straight from the book; the review cases (a whole recalculation)
    // only stand in when the snapshot has none to give.
    const inputVersion = async () => {
      try {
        const version = (await journals.mutateAsync({ expected_scope })).data?.status?.journalInputVersion;
        if (typeof version === "number" && Number.isSafeInteger(version)) return version;
      } catch {
        // Fall through to the review cases, which refuse in their own words.
      }
      return (await cases.mutateAsync({ limit: 1, expected_scope })).data?.input_version ?? 0;
    };
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const version = await inputVersion();
          const cacheKey = JSON.stringify([workspaceId, profileId, version, key]);
          let artifact = finished.get(cacheKey) ?? null;
          for (let attempts = 0; !artifact; attempts += 1) {
            let under = running.get(cacheKey);
            if (!under) {
              const promise = plan
                .mutateAsync({ operations: JSON.parse(key), expected_input_version: version, expected_scope, preview: true })
                .then((envelope) => reviewArtifact(envelope.data))
                .finally(() => running.delete(cacheKey));
              under = { promise, startedAt: Date.now() };
              running.set(cacheKey, under);
            } else if (token === run.current) {
              // Joined a run that began earlier: count its time, not this dialog's.
              setState({ status: "checking", startedAt: under.startedAt });
            }
            try {
              artifact = await under.promise;
            } catch (cause) {
              // Another check on screen started after this one and the daemon
              // stopped this one for it. Wait for the others to finish, then
              // run again, so two open checks never keep stopping each other.
              if (daemonErrorCode(cause) !== PREVIEW_CANCELLED || attempts >= 2 || token !== run.current) throw cause;
              await Promise.all([...running.values()].map((other) => other.promise.catch(() => null)));
              if (token !== run.current) return;
              continue;
            }
            if (!artifact) break;
            remember(cacheKey, artifact);
          }
          if (token !== run.current || useUiStore.getState().daemonSession !== session) return;
          if (!artifact) throw new Error(failedText);
          setState({ status: "ready", artifact, key: crypto.randomUUID() });
        } catch (cause) {
          if (token !== run.current) return;
          const code = daemonErrorCode(cause);
          // A check stopped for a newer one has no reason worth showing; Try again runs it.
          setState({ status: "failed", message: code === PREVIEW_CANCELLED ? failedText : messageOf(cause, failedText), code });
        }
      })();
    }, SETTLE_MS);
    return () => window.clearTimeout(timer);
    // The mutations are stable; a preview runs once per set of operations.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, workspaceId, profileId, attempt]);

  const retry = React.useCallback(() => setAttempt((count) => count + 1), []);

  const apply = React.useCallback(async (): Promise<ApplyResult | null> => {
    if (state.status !== "ready") return null;
    const { artifact, key: idempotency_key } = state;
    const envelope = await applyMutation.mutateAsync({
      artifact,
      idempotency_key,
      expected_scope: { workspace_id: artifact.workspace_id, profile_id: artifact.profile_id },
    });
    return envelope.data ?? null;
  }, [applyMutation, state]);

  return { state, apply, retry, applying: applyMutation.isPending };
}

/** What a previewed change does to the two sides of a move. */
export interface MoveOutcome {
  /** Held after the change (a decision is needed; reports wait). */
  sendHeld: boolean;
  receiptHeld: boolean;
  /** Held before the change. */
  wasHeld: boolean;
  reportsReadyBefore: boolean;
  reportsReadyAfter: boolean;
  /** Held transactions in the whole book, before and after. */
  heldBefore: number;
  heldAfter: number;
  /** Any tax year changes. */
  taxChanges: boolean;
}

function heldIds(effects: ReviewArtifact["before"]): Set<string> {
  const ids = new Set<string>();
  for (const row of effects.quarantines) {
    if (row && typeof row === "object" && "transaction_id" in row) {
      ids.add(String((row as { transaction_id: unknown }).transaction_id));
    }
  }
  return ids;
}

export function moveOutcome(artifact: ReviewArtifact, outId: string, inId: string): MoveOutcome {
  const before = heldIds(artifact.before);
  const after = heldIds(artifact.after);
  const delta = reviewTaxDelta(artifact);
  return {
    sendHeld: after.has(outId),
    receiptHeld: after.has(inId),
    wasHeld: before.has(outId) || before.has(inId),
    reportsReadyBefore: artifact.before.report_ready,
    reportsReadyAfter: artifact.after.report_ready,
    heldBefore: artifact.before.quarantine_count,
    heldAfter: artifact.after.quarantine_count,
    taxChanges: Boolean(delta?.years.length),
  };
}

/** The id an applied operation returned (`pair_id`, `dismissal_id`). */
export function appliedId(result: ApplyResult | null, field: "pair_id" | "dismissal_id"): string | null {
  const value = result?.operations?.[0]?.result?.[field];
  return typeof value === "string" && value ? value : null;
}
