import { CheckCircle2, ShieldCheck, XCircle } from "lucide-react";
import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ReviewProposalView, ReviewReceiptCard } from "./ReviewWorkflowCard";
import { reviewApprovalAvailable, reviewArtifact, reviewReceipt } from "./reviewWorkflow";
import { Button } from "@/components/ui/button";

import type {
  AiToolConsentDecision,
  AiToolConsentRequest,
} from "@/daemon/stream";
import { aiToolAllowsSessionConsent } from "@/daemon/stream";
import {
  Confirmation,
  ConfirmationAction,
  ConfirmationActions,
  ConfirmationRequest,
  ConfirmationTitle,
} from "@/components/ai-elements";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface ToolConsentDialogProps {
  request: AiToolConsentRequest | null;
  onDecision: (decision: AiToolConsentDecision) => Promise<void> | void;
}

export function ToolConsentDialog({
  request,
  onDecision,
}: ToolConsentDialogProps) {
  const { t } = useTranslation("assistant");
  if (request?.name === "ui.review.apply") {
    return <ReviewConsentDialog key={request.callId} request={request} onDecision={onDecision} />;
  }
  const hasPreview = request
    ? Object.keys(request.argumentsPreview).length > 0
    : false;

  return (
    <Dialog open={Boolean(request)}>
      <DialogContent showCloseButton={false} className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-base">
            <ShieldCheck className="h-4 w-4 text-primary" aria-hidden="true" />
            {t("consent.title")}
          </DialogTitle>
          <DialogDescription>
            <span className="font-medium text-foreground">
              {request?.summary ?? t("consent.toolAction")}
            </span>
          </DialogDescription>
        </DialogHeader>
        <Confirmation>
          <ConfirmationTitle>{request?.name ?? "tool"}</ConfirmationTitle>
          <ConfirmationRequest>
            <span className="block">
              {t("consent.description")}
            </span>
            {hasPreview ? (
              <details open className="mt-2">
                <summary className="cursor-pointer select-none text-2xs font-medium uppercase text-muted-foreground">
                  {t("consent.arguments")}
                </summary>
                <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-words rounded bg-background/75 px-2 py-1 font-mono text-2xs text-muted-foreground">
                  {JSON.stringify(request?.argumentsPreview ?? {}, null, 2)}
                </pre>
              </details>
            ) : null}
          </ConfirmationRequest>
          <ConfirmationActions>
            <ConfirmationAction
              type="button"
              variant="outline"
              onClick={() => void onDecision("deny")}
            >
              <XCircle aria-hidden="true" />
              {t("consent.deny")}
            </ConfirmationAction>
            {request && aiToolAllowsSessionConsent(request.name) ? (
              <ConfirmationAction
                type="button"
                variant="secondary"
                onClick={() => void onDecision("allow_session")}
              >
                <ShieldCheck aria-hidden="true" />
                {t("consent.allowSession")}
              </ConfirmationAction>
            ) : null}
            <ConfirmationAction
              type="button"
              onClick={() => void onDecision("allow_once")}
            >
              <CheckCircle2 aria-hidden="true" />
              {t("consent.allowOnce")}
            </ConfirmationAction>
          </ConfirmationActions>
        </Confirmation>
      </DialogContent>
    </Dialog>
  );
}

export function ReviewConsentBody({ request }: { request: AiToolConsentRequest }) {
  const { t } = useTranslation("assistant");
  const preview = request.reviewPreview;
  const artifact = preview?.status === "ready" ? reviewArtifact(preview.artifact) : null;
  const receipt = preview?.status === "applied" ? reviewReceipt(preview.receipt) : null;
  if (artifact) return <ReviewProposalView artifact={artifact} />;
  if (receipt) return <><p className="text-sm">{t("consent.apply.alreadyApplied")}</p><ReviewReceiptCard receipt={receipt} /></>;
  return <p role="alert" className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">{t("consent.apply.unavailable")}{preview?.status === "unavailable" ? <span className="mt-2 block font-mono text-xs">{preview.code}</span> : null}</p>;
}

function ReviewConsentDialog({ request, onDecision }: { request: AiToolConsentRequest; onDecision: ToolConsentDialogProps["onDecision"] }) {
  const { t } = useTranslation("assistant");
  const descriptionId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [pending, setPending] = useState(false);
  const canApprove = reviewApprovalAvailable(request.reviewPreview);
  const decide = async (decision: AiToolConsentDecision) => {
    if (pending || (decision !== "deny" && !canApprove)) return;
    setPending(true);
    try { await onDecision(decision); } finally { setPending(false); }
  };
  return <Dialog open>
    <DialogContent role="alertdialog" aria-describedby={descriptionId} showCloseButton={false}
      className="grid grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-2xl"
      onOpenAutoFocus={(event) => { event.preventDefault(); cancelRef.current?.focus(); }}
      onEscapeKeyDown={(event) => { event.preventDefault(); void decide("deny"); }}>
      <DialogHeader className="border-b px-5 py-4 text-left">
        <DialogTitle>{t("consent.apply.title")}</DialogTitle>
        <DialogDescription id={descriptionId}>{t("consent.apply.description")}</DialogDescription>
      </DialogHeader>
      <div className="min-h-0 overflow-y-auto p-5"><ReviewConsentBody request={request} /></div>
      <div className="flex flex-wrap justify-end gap-2 border-t bg-background px-5 py-4">
        <Button ref={cancelRef} variant="outline" className="min-h-11" disabled={pending} onClick={() => void decide("deny")}>{t("consent.apply.cancel")}</Button>
        <Button className="min-h-11" disabled={pending || !canApprove} onClick={() => void decide("allow_once")}>{t(request.reviewPreview?.status === "applied" ? "consent.apply.returnReceipt" : "consent.apply.confirm")}</Button>
      </div>
    </DialogContent>
  </Dialog>;
}
