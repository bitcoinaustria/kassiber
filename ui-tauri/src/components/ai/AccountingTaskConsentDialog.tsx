import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AiToolConsentDecision, AiToolConsentRequest } from "@/daemon/stream";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useUiStore } from "@/store/ui";
import { TaskPreviewCard } from "./accounting/TaskPreviewCard";
import { isTaskPreview } from "./accounting/taskModel";

/** Validate the desktop-only field; model arguments cannot supply financial effects. */
function accountingTaskConsentPreview(request: AiToolConsentRequest) {
  const value = request.accountingTaskPreview;
  if (value?.status !== "ready" || !isTaskPreview(value.preview)) return null;
  const preview = value.preview;
  if (value.step !== preview.step || preview.id !== request.argumentsPreview.task_id
      || !/^[a-f0-9]{64}$/.test(preview.expected_digest)
      || !value.book || !/^[A-Z]{3}$/.test(value.book.currency)
      || !Number.isInteger(value.book.minor_unit_exponent)
      || value.book.minor_unit_exponent < 0 || value.book.minor_unit_exponent > 8) return null;
  return { preview, book: value.book };
}

export function AccountingTaskConsentDialog({ request, onDecision }: {
  request: AiToolConsentRequest;
  onDecision: (decision: AiToolConsentDecision) => Promise<void> | void;
}) {
  const { t } = useTranslation("assistant");
  const descriptionId = useId();
  const checkboxId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const hidden = useUiStore((state) => state.hideSensitive);
  const [pending, setPending] = useState(false);
  const [reviewed, setReviewed] = useState<string | null>(null);
  const verified = accountingTaskConsentPreview(request);
  const digest = verified?.preview.expected_digest;
  useEffect(() => { setReviewed(null); }, [request.callId, digest, hidden]);
  const canApprove = Boolean(verified?.preview.ready && !verified.preview.blockers?.length
    && !hidden && reviewed === digest);
  const decide = async (decision: "deny" | "allow_once") => {
    if (pending || (decision !== "deny" && (!canApprove || useUiStore.getState().hideSensitive))) return;
    setPending(true);
    try { await onDecision(decision); } finally { setPending(false); }
  };
  return <Dialog open>
    <DialogContent role="alertdialog" aria-describedby={descriptionId} showCloseButton={false}
      className="grid max-h-[85dvh] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-3xl"
      onOpenAutoFocus={(event) => { event.preventDefault(); cancelRef.current?.focus(); }}
      onEscapeKeyDown={(event) => { event.preventDefault(); void decide("deny"); }}
      onInteractOutside={(event) => { event.preventDefault(); }}>
      <DialogHeader className="border-b px-5 py-4 text-left">
        <DialogTitle>{t("consent.accountingTask.title")}</DialogTitle>
        <DialogDescription id={descriptionId}>{t("consent.accountingTask.description")}</DialogDescription>
      </DialogHeader>
      <div className="min-h-0 overflow-y-auto p-5">
        {hidden ? <p role="alert">{t("consent.accountingTask.hidden")}</p>
          : verified ? <TaskPreviewCard value={verified.preview} book={verified.book} />
          : <p role="alert">{t("consent.accountingTask.unavailable")}</p>}
      </div>
      <div className="space-y-4 border-t bg-background px-5 py-4">
        <label htmlFor={checkboxId} className="flex items-start gap-2 text-sm">
          <input id={checkboxId} type="checkbox" className="mt-1" checked={!hidden && reviewed === digest && Boolean(digest)}
            disabled={pending || hidden || !verified?.preview.ready || Boolean(verified.preview.blockers?.length)}
            onChange={(event) => setReviewed(event.target.checked && digest ? digest : null)} />
          {t("consent.accountingTask.acknowledge")}
        </label>
        <div className="flex flex-wrap justify-end gap-2">
          <Button ref={cancelRef} variant="outline" className="min-h-11" disabled={pending}
            onClick={() => void decide("deny")}>{t("consent.apply.cancel")}</Button>
          <Button className="min-h-11" disabled={pending || !canApprove}
            onClick={() => void decide("allow_once")}>{verified ? t(`consent.accountingTask.${verified.preview.step}`) : t("consent.allowOnce")}</Button>
        </div>
      </div>
    </DialogContent>
  </Dialog>;
}
