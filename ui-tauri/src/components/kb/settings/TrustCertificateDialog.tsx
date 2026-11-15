import * as React from "react";
import { useTranslation } from "react-i18next";

import { CopyButton } from "@/components/kb/CopyButton";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { DaemonRequestError, useDaemonMutation } from "@/daemon/client";
import {
  certificateErrorKey,
  type CertificateErrorKey,
  type CertificateProblem,
  type PresentedCertificate,
  type TrustedCertificate,
} from "@/lib/certificateTrust";
import { formatDateTime } from "@/lib/localeFormat";

export type TrustCertificatePhase =
  | "intro"
  | "fetching"
  | "review"
  | "trusting"
  | "done"
  | "error";

export interface TrustCertificateViewProps {
  backendLabel: string;
  phase: TrustCertificatePhase;
  problem?: CertificateProblem | null;
  presented?: PresentedCertificate | null;
  errorKey?: CertificateErrorKey | null;
  onFetch: () => void;
  onTrust: () => void;
  onClose: () => void;
}

function Field({
  label,
  value,
  mono,
  action,
}: {
  label: string;
  value: string;
  mono?: boolean;
  action?: React.ReactNode;
}) {
  return (
    <div className="grid gap-0.5 sm:grid-cols-[9rem_1fr] sm:gap-3">
      <dt className="text-sm text-muted-foreground">{label}</dt>
      <dd className="flex min-w-0 items-start gap-2">
        <span className={mono ? "min-w-0 font-mono text-sm break-all" : "min-w-0 text-sm break-words"}>
          {value}
        </span>
        {action}
      </dd>
    </div>
  );
}

/**
 * The trust decision itself, without daemon calls. Nothing is fetched until
 * the user presses Fetch, and nothing is trusted until they compare the
 * fingerprint and press Trust: the daemon then fetches again and refuses a
 * different certificate.
 */
export function TrustCertificateView({
  backendLabel,
  phase,
  problem,
  presented,
  errorKey,
  onFetch,
  onTrust,
  onClose,
}: TrustCertificateViewProps) {
  const { t, i18n } = useTranslation(["settings", "common"]);
  const busy = phase === "fetching" || phase === "trusting";
  // A fetched certificate that would replace a pin is a changed certificate,
  // however the dialog was opened.
  const changed =
    phase !== "done" &&
    (problem === "changed" || Boolean(presented?.replaces_fingerprint_sha256));
  const date = (value: string) => formatDateTime(value, i18n.language) ?? value;
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("certificateTrust.title")}</DialogTitle>
        <DialogDescription>
          {t("certificateTrust.description", { backend: backendLabel })}
        </DialogDescription>
      </DialogHeader>
      <div className="space-y-3 text-sm">
        {changed ? (
          <p
            role="alert"
            className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-red-700 dark:text-red-300"
          >
            {t("certificateTrust.changedWarning")}
          </p>
        ) : problem === "untrusted" ? (
          <p className="text-muted-foreground">{t("certificateTrust.untrustedNotice")}</p>
        ) : null}
        {(phase === "intro" || phase === "fetching") && (
          <p className="text-muted-foreground">{t("certificateTrust.introBody")}</p>
        )}
        {presented && (phase === "review" || phase === "trusting" || phase === "done") && (
          <>
            <dl className="space-y-2 rounded-md border px-3 py-3">
              <Field
                label={t("certificateTrust.server")}
                value={`${presented.host}:${presented.port}`}
                mono
              />
              <Field
                label={t("certificateTrust.fingerprint")}
                value={presented.fingerprint_sha256}
                mono
                action={
                  <CopyButton
                    value={presented.fingerprint_sha256}
                    ariaLabel={t("certificateTrust.copyFingerprint")}
                  />
                }
              />
              {presented.replaces_fingerprint_sha256 && phase !== "done" ? (
                <Field
                  label={t("certificateTrust.previousFingerprint")}
                  value={presented.replaces_fingerprint_sha256}
                  mono
                />
              ) : null}
              {presented.subject ? (
                <Field label={t("certificateTrust.subject")} value={presented.subject} />
              ) : null}
              <Field
                label={t("certificateTrust.issuer")}
                value={
                  presented.self_signed
                    ? t("certificateTrust.selfSigned")
                    : presented.issuer ?? t("certificateTrust.unknown")
                }
              />
              {presented.not_before && presented.not_after ? (
                <Field
                  label={t("certificateTrust.validity")}
                  value={t("certificateTrust.validityRange", {
                    from: date(presented.not_before),
                    to: date(presented.not_after),
                  })}
                />
              ) : null}
            </dl>
            {phase !== "done" && (
              <p className="font-medium">{t("certificateTrust.compare")}</p>
            )}
            {presented.replaces_fingerprint_sha256 && phase !== "done" ? (
              <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-amber-800 dark:text-amber-200">
                {t("certificateTrust.replaces")}
              </p>
            ) : null}
            {presented.clears_insecure && phase !== "done" ? (
              <p className="text-muted-foreground">{t("certificateTrust.clearsInsecure")}</p>
            ) : null}
            {phase === "done" ? (
              <p role="status" className="font-medium text-emerald-700 dark:text-emerald-300">
                {t("certificateTrust.trusted", { host: presented.host })}
              </p>
            ) : null}
          </>
        )}
        {phase === "error" && errorKey ? (
          <p role="alert" className="text-red-700 dark:text-red-300">
            {/* dynamic key */}
            {t(`certificateTrust.errors.${errorKey}` as never)}
          </p>
        ) : null}
      </div>
      <DialogFooter>
        {phase === "done" ? (
          <Button onClick={onClose}>{t("common:actions.done")}</Button>
        ) : (
          <>
            <Button variant="outline" onClick={onClose} disabled={busy}>
              {t("common:actions.cancel")}
            </Button>
            {phase === "review" || phase === "trusting" ? (
              <Button onClick={onTrust} disabled={busy}>
                {t("certificateTrust.trust")}
              </Button>
            ) : (
              <Button onClick={onFetch} disabled={busy}>
                {phase === "error"
                  ? t("common:actions.retry")
                  : t("certificateTrust.fetch")}
              </Button>
            )}
          </>
        )}
      </DialogFooter>
    </>
  );
}

export interface TrustCertificateDialogProps {
  open: boolean;
  /** The saved backend's name, as the daemon knows it. */
  backend: string;
  /** What to call it on screen (its display name). */
  backendLabel?: string;
  problem?: CertificateProblem | null;
  onClose: () => void;
  onTrusted?: (result: TrustedCertificate) => void;
}

export function TrustCertificateDialog({
  open,
  backend,
  backendLabel,
  problem,
  onClose,
  onTrusted,
}: TrustCertificateDialogProps) {
  const fetchCertificate = useDaemonMutation<PresentedCertificate>(
    "ui.backends.certificate.fetch",
  );
  const trustCertificate = useDaemonMutation<TrustedCertificate>(
    "ui.backends.certificate.trust",
  );
  const [phase, setPhase] = React.useState<TrustCertificatePhase>("intro");
  const [presented, setPresented] = React.useState<PresentedCertificate | null>(null);
  const [errorKey, setErrorKey] = React.useState<CertificateErrorKey | null>(null);

  const fail = (error: unknown) => {
    setErrorKey(
      certificateErrorKey(
        error instanceof DaemonRequestError ? error.envelope.error?.code : undefined,
      ),
    );
    setPhase("error");
  };
  const close = () => {
    setPhase("intro");
    setPresented(null);
    setErrorKey(null);
    onClose();
  };
  // The user's click is the only thing that reaches the server.
  const onFetch = async () => {
    setPhase("fetching");
    setPresented(null);
    setErrorKey(null);
    try {
      const envelope = await fetchCertificate.mutateAsync({ name: backend });
      if (!envelope.data) throw new Error("empty certificate response");
      setPresented(envelope.data);
      setPhase("review");
    } catch (error) {
      fail(error);
    }
  };
  // Trust sends the fingerprint the user saw; the daemon fetches again and
  // refuses if the server now presents anything else.
  const onTrust = async () => {
    if (!presented) return;
    setPhase("trusting");
    try {
      const envelope = await trustCertificate.mutateAsync({
        name: backend,
        fingerprint: presented.fingerprint_sha256,
        revision: presented.revision,
      });
      if (!envelope.data) throw new Error("empty trust response");
      setPresented(envelope.data);
      setPhase("done");
      onTrusted?.(envelope.data);
    } catch (error) {
      fail(error);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && phase !== "fetching" && phase !== "trusting") close();
      }}
    >
      <DialogContent className="sm:max-w-[560px]">
        <TrustCertificateView
          backendLabel={backendLabel ?? backend}
          phase={phase}
          problem={problem}
          presented={presented}
          errorKey={errorKey}
          onFetch={() => void onFetch()}
          onTrust={() => void onTrust()}
          onClose={close}
        />
      </DialogContent>
    </Dialog>
  );
}
