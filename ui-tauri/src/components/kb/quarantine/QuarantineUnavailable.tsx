import { useTranslation } from "react-i18next";

import { screenPanelClassName } from "@/lib/screen-layout";

interface QuarantineUnavailableProps {
  message?: string;
}

export function QuarantineUnavailable({ message }: QuarantineUnavailableProps) {
  const { t } = useTranslation("journals");
  return (
    <div className={screenPanelClassName}>
      <div className="rounded-lg border bg-card p-4">
        <h2 className="text-base font-semibold">
          {t("quarantine.unavailable.title")}
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {message ?? t("quarantine.unavailable.fallback")}
        </p>
      </div>
    </div>
  );
}
