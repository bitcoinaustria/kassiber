import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  CERTIFICATE_TRUST_KINDS,
  type CertificateProblem,
  type TrustedCertificate,
} from "@/lib/certificateTrust";

import { TrustCertificateDialog } from "./TrustCertificateDialog";

export interface CertificateTrustOfferProps {
  /** The certificate problem behind the error shown beside this offer. */
  problem: CertificateProblem | null;
  /** The backend that failed, as the wallet resolved it. */
  backend?: { name?: string; kind?: string } | null;
  onTrusted?: (result: TrustedCertificate) => void;
}

/**
 * "Check certificate…" beside an error an untrusted or changed backend
 * certificate caused. Rendering contacts nothing; only the dialog's own
 * Fetch button does.
 */
export function CertificateTrustOffer({ problem, backend, onTrusted }: CertificateTrustOfferProps) {
  const { t } = useTranslation("settings");
  const [open, setOpen] = React.useState(false);
  const name = backend?.name;
  if (!problem || !name || !CERTIFICATE_TRUST_KINDS.includes(backend?.kind ?? "")) {
    return null;
  }
  return (
    <>
      <div className="mt-2">
        <Button size="sm" variant="outline" onClick={() => setOpen(true)}>
          {t("certificateTrust.checkButton")}
        </Button>
      </div>
      <TrustCertificateDialog
        key={name}
        open={open}
        backend={name}
        problem={problem}
        onClose={() => setOpen(false)}
        onTrusted={onTrusted}
      />
    </>
  );
}
