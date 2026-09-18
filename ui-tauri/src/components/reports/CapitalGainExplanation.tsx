import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useDaemon } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle, DialogTrigger } from "@/components/ui/dialog";

export interface ResultReference {
  database_id: string;
  workspace_id: string;
  profile_id: string;
  input_version: number;
  processed_at: string | null;
  entry_id: string;
}
interface Source {
  transaction_id: string | null;
  engine_event_id: string;
  occurred_at: string;
  asset: string;
  spot_price_exact: string;
  fiat_fee_exact: string | null;
  crypto_fee_msat: number;
  crypto_fee_msat_exact: string;
  pricing: Record<string, string | null>;
  inherited_basis?: {
    status: string;
    relations: Array<Record<string, string | null>>;
    source_calculations: Array<{
      transaction_id: string; asset: string; status: string;
      calculation: Calculation | null;
      totals: { proceeds_exact: string; cost_basis_exact: string; gain_loss_exact: string; quantity_msat_exact: string } | null;
    }>;
  };
}
interface Fragment {
  quantity_msat: number;
  quantity_msat_exact: string;
  cost_basis_exact: string;
  proceeds_exact: string;
  gain_loss_exact: string;
  unit_basis_override_exact: string | null;
  acquisition_basis_exact: string | null;
  acquisition_quantity_msat: number | null;
  acquisition_quantity_msat_exact: string | null;
  event: Source;
  lot: Source | null;
}
interface Calculation { method: string; fragments: Fragment[] }
interface Explanation {
  reference: ResultReference;
  status: "available" | "engine_detail_unavailable" | "engine_detail_mismatch";
  currency: string;
  book_label: string;
  quarantines: number;
  totals: { cost_basis_exact: string; proceeds_exact: string; gain_loss_exact: string };
  calculation: Calculation | null;
  custody_decisions: Array<Record<string, string | null>>;
  custody_truncated: boolean;
}

export function CapitalGainExplanation({ reference, hideSensitive }: { reference: ResultReference; hideSensitive: boolean }) {
  const { t } = useTranslation("journals");
  const [open, setOpen] = useState(false);
  return <Dialog open={open} onOpenChange={setOpen}>
    <DialogTrigger asChild><Button size="sm" variant="ghost">{t("explanation.open")}</Button></DialogTrigger>
    {open && <DialogContent className="max-w-3xl! overflow-y-auto">
      <DialogTitle>{t("explanation.title")}</DialogTitle>
      <DialogDescription>{t("explanation.description")}</DialogDescription>
      <ExplanationBody reference={reference} hideSensitive={hideSensitive} />
    </DialogContent>}
  </Dialog>;
}

export function ExplanationBody({ reference, hideSensitive }: { reference: ResultReference; hideSensitive: boolean }) {
  const { t } = useTranslation("journals");
  const { data, error, isLoading, isFetching } = useDaemon<Explanation>("ui.reports.explain_capital_gain", { reference }, { staleTime: 0, retry: false });
  const result = data?.data;
  if (isLoading || isFetching) return <p>{t("explanation.loading")}</p>;
  if (error || data?.error || !result) return <p role="alert">{error instanceof Error ? error.message : data?.error?.message ?? t("explanation.unavailable")}</p>;
  if (result.status !== "available" || !result.calculation) return <p role="alert">{t(result.status === "engine_detail_mismatch" ? "explanation.mismatch" : "explanation.unavailable")}</p>;
  return <div className={`space-y-4 text-sm ${hideSensitive ? "sensitive" : ""}`}>
    <p className="text-muted-foreground">{result.book_label} · {result.calculation.method} · {result.currency} · {t("explanation.version", { version: reference.input_version })}</p>
    <a className="underline" href={resultLink(reference)}>{t("explanation.link")}</a>
    <p className="font-mono break-all">{result.totals.proceeds_exact} − {result.totals.cost_basis_exact} = {result.totals.gain_loss_exact} {result.currency}</p>
    {result.quarantines > 0 && <p role="status">{t("explanation.quarantines", { count: result.quarantines })}</p>}
    <p className="text-muted-foreground">{t("explanation.poolNote")}</p>
    <CalculationEvidence calculation={result.calculation} currency={result.currency} reference={reference} prefix="" />
    <section><h3 className="font-medium">{t("explanation.custody")}</h3>
      <p className="text-muted-foreground">{t("explanation.custodyContext")}</p>
      {result.custody_decisions.length ? result.custody_decisions.map((decision, index) => <p key={index} className="break-all font-mono text-xs">{decision.source_transaction_id} → {decision.target_transaction_id} · {decision.custody_state} · {decision.basis_state} · {decision.reason}</p>) : <p>{t("explanation.noCustody")}</p>}
      {result.custody_truncated && <p>{t("explanation.custodyTruncated")}</p>}
    </section>
  </div>;
}

function CalculationEvidence({ calculation, currency, reference, prefix }: { calculation: Calculation; currency: string; reference: ResultReference; prefix: string }) {
  const { t } = useTranslation("journals");
  return <>{calculation.fragments.map((fragment, index) => <section key={index} className="space-y-2 rounded-md border p-3">
    <h3 className="font-medium">{t("explanation.fragment", { index: index + 1 })} · {fragment.quantity_msat_exact} msat</h3>
    <p className="font-mono break-all">{fragment.proceeds_exact} − {fragment.cost_basis_exact} = {fragment.gain_loss_exact} {currency}</p>
    {fragment.unit_basis_override_exact !== null && <p>{t("explanation.unitBasis")}: <span className="font-mono">{fragment.unit_basis_override_exact}</span> {currency}/{fragment.event.asset}</p>}
    <SourceEvidence source={fragment.event} label={t("explanation.disposal")} reference={reference} node={`${prefix}event-${index}`} />
    {fragment.lot && <>
      <p>{t("explanation.acquisitionBasis")}: <span className="font-mono">{fragment.acquisition_basis_exact}</span> {currency} / {fragment.acquisition_quantity_msat_exact} msat</p>
      <SourceEvidence source={fragment.lot} label={t("explanation.acquisition")} reference={reference} node={`${prefix}lot-${index}`} />
      {fragment.lot.inherited_basis && <div className="space-y-2 border-l-2 pl-3">
        <h4 className="font-medium">{t("explanation.inheritedBasis")}</h4>
        <p className="text-muted-foreground">{t("explanation.inheritedContext")}</p>
        {fragment.lot.inherited_basis.relations.map((relation) => <p key={relation.decision_id} className="break-all font-mono text-xs">
          {relation.source_transaction_id} → {relation.target_transaction_id} · {relation.reason} · {relation.policy} · {relation.basis_state}<br />
          {relation.source_quantity_msat_exact} {relation.source_asset} msat → {relation.target_quantity_msat_exact} {relation.target_asset} msat · {t("explanation.fee")}: {relation.swap_fee_msat_exact ?? "—"} msat
        </p>)}
        {fragment.lot.inherited_basis.status !== "available" && <p role="status">{t(fragment.lot.inherited_basis.status === "inherited_detail_truncated" ? "explanation.inheritedTruncated" : fragment.lot.inherited_basis.status === "engine_detail_mismatch" ? "explanation.mismatch" : "explanation.inheritedUnavailable")}</p>}
        {fragment.lot.inherited_basis.source_calculations.map((source, sourceIndex) => <div key={sourceIndex} className="space-y-2">
          <h5>{source.transaction_id} · {source.asset} · {source.calculation?.method}</h5>
          {source.totals && <p className="font-mono break-all">{source.totals.proceeds_exact} − {source.totals.cost_basis_exact} = {source.totals.gain_loss_exact} {currency} · {source.totals.quantity_msat_exact} msat</p>}
          {source.calculation && <CalculationEvidence calculation={source.calculation} currency={currency} reference={reference} prefix={`${prefix}carry-${index}-${sourceIndex}-`} />}
        </div>)}
      </div>}
    </>}
  </section>)}</>;
}

function SourceEvidence({ source, label, reference, node }: { source: Source; label: string; reference: ResultReference; node: string }) {
  const { t } = useTranslation("journals");
  const labels: Record<string, string> = {
    fiat_currency: t("explanation.priceCurrency"), fiat_rate_exact: t("explanation.sourceRate"),
    fiat_value_exact: t("explanation.sourceValue"), pricing_source_kind: t("explanation.priceKind"),
    pricing_quality: t("explanation.priceQuality"), fiat_price_source: t("explanation.priceSource"),
    pricing_timestamp: t("explanation.priceTimestamp"), pricing_provider: t("explanation.priceProvider"),
    pricing_pair: t("explanation.pricePair"), pricing_fetched_at: t("explanation.priceFetched"),
    pricing_granularity: t("explanation.priceGranularity"), pricing_method: t("explanation.priceMethod"),
  };
  return <details id={node} open={(typeof window !== "undefined" && window.location.hash === `#${node}`) || undefined} className="rounded border p-2">
    <summary className="cursor-pointer break-all">{label}: {source.transaction_id ?? t("explanation.missingSource")}</summary>
    <a className="text-xs underline" href={`${resultLink(reference)}#${node}`}>{t("explanation.sourceLink")}</a>
    <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 break-all text-xs">
      <dt>{t("explanation.timestamp")}</dt><dd>{source.occurred_at}</dd>
      <dt>{t("explanation.spotPrice")}</dt><dd>{source.spot_price_exact}</dd>
      <dt>{t("explanation.fee")}</dt><dd>{source.fiat_fee_exact ?? "—"} · {source.crypto_fee_msat_exact} msat</dd>
      {Object.entries(source.pricing).filter(([, value]) => value !== null).map(([key, value]) => <div key={key} className="contents"><dt>{labels[key]}</dt><dd>{value}</dd></div>)}
    </dl>
  </details>;
}

function resultLink(reference: ResultReference) {
  return `/reports?result=${encodeURIComponent(JSON.stringify(reference))}`;
}
