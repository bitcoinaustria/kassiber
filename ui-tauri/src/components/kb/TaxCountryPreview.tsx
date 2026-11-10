import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { isPreviewTaxCountry } from "@/lib/taxCountries";

// A country not yet reviewed by a tax professional carries this badge
// wherever it is named or chosen.
export function TaxCountryPreviewBadge() {
  const { t } = useTranslation("common");
  return (
    <Badge variant="outline" className="ml-1.5 align-middle">
      {t("taxCountry.previewBadge")}
    </Badge>
  );
}

// A country's label with its preview badge.
export function TaxCountryLabel({
  country,
  label,
}: {
  country: string | null | undefined;
  label: string;
}) {
  return (
    <span className="inline-flex items-center">
      {label}
      {isPreviewTaxCountry(country) && <TaxCountryPreviewBadge />}
    </span>
  );
}

// The notice a preview country's book shows next to its choice and on its
// reports; the same text the exported reports carry.
export function TaxCountryPreviewNotice({
  country,
  className,
}: {
  country: string | null | undefined;
  className?: string;
}) {
  const { t } = useTranslation("common");
  if (!isPreviewTaxCountry(country)) return null;
  return (
    <p
      role="note"
      data-testid="tax-country-preview-notice"
      className={
        className ??
        "m-0 rounded-md border border-line px-3 py-2 text-xs leading-5 text-ink"
      }
    >
      <span className="font-medium">{t("taxCountry.previewBadge")}: </span>
      {t("taxCountry.previewNotice")}
    </p>
  );
}
