import { CheckCircle2, ShieldCheck, XCircle } from "lucide-react";

import type {
  AiToolConsentDecision,
  AiToolConsentRequest,
} from "@/daemon/stream";
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
  const hasPreview = request
    ? Object.keys(request.argumentsPreview).length > 0
    : false;

  return (
    <Dialog open={Boolean(request)}>
      <DialogContent showCloseButton={false} className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-base">
            <ShieldCheck className="h-4 w-4 text-primary" aria-hidden="true" />
            Allow tool action?
          </DialogTitle>
          <DialogDescription>
            <span className="font-medium text-foreground">
              {request?.summary ?? "Tool action"}
            </span>
          </DialogDescription>
        </DialogHeader>
        <Confirmation>
          <ConfirmationTitle>{request?.name ?? "tool"}</ConfirmationTitle>
          <ConfirmationRequest>
            <span className="block">
              This action needs explicit approval before Kassiber runs it.
            </span>
            {hasPreview ? (
              <details className="mt-2">
                <summary className="cursor-pointer select-none text-[10px] font-medium uppercase text-muted-foreground">
                  Arguments
                </summary>
                <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-words rounded bg-background/75 px-2 py-1 font-mono text-[10px] text-muted-foreground">
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
              Deny
            </ConfirmationAction>
            <ConfirmationAction
              type="button"
              variant="secondary"
              onClick={() => void onDecision("allow_session")}
            >
              <ShieldCheck aria-hidden="true" />
              Allow this session
            </ConfirmationAction>
            <ConfirmationAction
              type="button"
              onClick={() => void onDecision("allow_once")}
            >
              <CheckCircle2 aria-hidden="true" />
              Allow once
            </ConfirmationAction>
          </ConfirmationActions>
        </Confirmation>
      </DialogContent>
    </Dialog>
  );
}
