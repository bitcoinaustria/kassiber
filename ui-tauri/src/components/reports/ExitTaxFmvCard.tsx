import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import type { ExitTaxFmvSource } from "@/mocks/exitTax";

export function ExitTaxFmvCard({
  quotes,
  fmtMoney,
}: {
  quotes: ExitTaxFmvSource[];
  fmtMoney: (value: number | null) => string;
}) {
  const { t } = useTranslation("overview");
  const missing = quotes.filter((quote) => quote.source === "missing").map((quote) => quote.asset);
  return (
    <section className="kb-surface p-(--kb-card-padding)">
      <h2 className="text-sm font-medium">{t("exitTaxFmv.title")}</h2>
      {missing.length > 0 ? (
        <p role="status" className="mt-2 text-xs text-amber-700 dark:text-amber-400">
          {t("exitTaxFmv.incomplete", { assets: missing.join(", ") })}
        </p>
      ) : null}
      <ul className="mt-2 grid gap-1 text-xs text-muted-foreground">
        {quotes.map((quote) => (
          <li key={quote.asset} className="flex items-start justify-between gap-2">
            <span>{quote.pair}</span>
            <span className="text-right tabular-nums">
              {fmtMoney(quote.rate)}
              <Badge variant="outline" className="ml-2">
                {t(`exitTaxFmv.source.${quote.source}`)}
              </Badge>
              <span className="mt-1 block">
                {t("exitTaxFmv.asOf")} {quote.asOf ? (
                  <time dateTime={quote.asOf}>{quote.asOf}</time>
                ) : t("exitTaxFmv.unavailable")}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
