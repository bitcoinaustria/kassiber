import * as React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { AssistantSessionContext } from "./assistantSession";
import type { EvidenceRequest } from "./evidenceRequest";

/** Missing evidence is a local handoff, never an accounting approval. */
export function EvidenceRequestCard({ request }: { request: EvidenceRequest }) {
  const { t } = useTranslation("assistant");
  const assistant = React.useContext(AssistantSessionContext);
  const state = assistant?.evidenceRequests?.[request.request_id] ?? { status: "idle" };
  const waiting = ["received", "partial", "unavailable"].includes(state.status);
  const blocked = !assistant?.openEvidenceRequest || assistant.isStreaming || ["opening", "continuing", "stale"].includes(state.status);
  const otherActive = Object.entries(assistant?.evidenceRequests ?? {}).some(([id, value]) =>
    id !== request.request_id && ["opening", "received", "partial", "unavailable"].includes(value.status));
  return <section aria-label={t("evidence.label")} className="my-3 border-l-2 border-border pl-4 text-sm">
    <p className="font-medium">{t(`evidence.missing.${request.action}`)}</p>
    <p className="mt-1 text-muted-foreground">{request.explanation || t(`evidence.why.${request.action}`)}</p>
    <div className="mt-3 flex flex-wrap items-center gap-3">
      <Button size="sm" variant="outline" disabled={blocked || otherActive || (waiting && assistant?.isStreaming)}
        onClick={() => waiting ? assistant?.continueEvidenceRequest?.(request)
          : void assistant?.openEvidenceRequest?.(request)}>
        {waiting ? t("evidence.resume") : t(`evidence.action.${request.action}`)}
      </Button>
      {!waiting && !["continuing", "stale"].includes(state.status) ?
        <button type="button" className="text-xs text-muted-foreground underline-offset-4 hover:underline"
          disabled={blocked || otherActive}
          onClick={() => assistant?.continueEvidenceRequest?.(request, true)}>{t("evidence.unavailable")}</button> : null}
    </div>
    <p role="status" aria-live="polite" className="mt-2 text-xs text-muted-foreground">
      {state.error || t(`evidence.status.${state.status}`)}
    </p>
  </section>;
}
