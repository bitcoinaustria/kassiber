import { useTranslation } from "react-i18next";

interface QuarantineUnavailableProps {
  message?: string;
}

export function QuarantineUnavailable({ message }: QuarantineUnavailableProps) {
  const { t } = useTranslation("journals");
  return (
    <div className="kb-surface p-(--kb-card-padding)" role="alert">
      <h2 className="text-sm font-semibold">
        {t("quarantine.unavailable.title")}
      </h2>
      <p className="mt-1 text-sm text-muted-foreground">
        {message ?? t("quarantine.unavailable.fallback")}
      </p>
    </div>
  );
}
