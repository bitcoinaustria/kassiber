/**
 * Throwaway browser-mock fixtures for the exit-tax (Wegzugsbesteuerung) report.
 *
 * Mirrors the `ui.reports.exit_tax_preview` daemon contract (see
 * kassiber/core/exit_tax.py). Bitcoin-only, EUR. Regenerated once the
 * Pydantic→JSON Schema pipeline lands, like the other mock fixtures.
 */

export type ExitTaxRegime = "neu" | "alt";
export type ExitTaxDestination = "eu_eea" | "third_country";
export type ExitTaxCollectionTiming = "deferred" | "immediate";

export interface ExitTaxLot {
  asset: string;
  regime: ExitTaxRegime;
  quantitySats: number;
  marketValue: number | null;
  costBasis: number | null;
  gain: number | null;
  taxable: boolean;
  category: string;
  kennzahl: number | null;
}

export interface ExitTaxFmvSource {
  asset: string;
  rate: number | null;
  pair: string;
  asOf: string | null;
  source: "cache" | "transaction" | "missing";
}

export interface ExitTaxWalletHolding {
  wallet: string;
  asset: string;
  quantitySats: number;
  marketValue: number | null;
}

export interface ExitTaxReport {
  workspace: string;
  profile: string;
  jurisdictionCode: "AT" | "generic";
  fiatCurrency: string;
  departureDate: string;
  destination: ExitTaxDestination;
  method: string;
  fmvSource: ExitTaxFmvSource[];
  totals: {
    neuQuantitySats: number;
    neuMarketValue: number | null;
    neuCostBasis: number | null;
    neuGain: number | null;
    altQuantitySats: number;
    altMarketValue: number | null;
    taxableGain: number | null;
    estimatedTaxRate: number | null;
    estimatedTax: number | null;
    collectionTiming: ExitTaxCollectionTiming;
  };
  lots: ExitTaxLot[];
  walletHoldings: ExitTaxWalletHolding[];
  assumptions: string[];
  reviewGate: string;
  status: { needsJournals: boolean; quarantines: number };
}

const REVIEW_GATE =
  "Estimate only — not tax advice. The exit-tax (Wegzugsbesteuerung) liability " +
  "across all of your assets is determined by your Steuerberater. Hand this draft " +
  "to your tax adviser for review before relying on it or filing.";

const BASE_ASSUMPTIONS = [
  "EXIT: Altbestand (acquired before 2021-03-01 Europe/Vienna) is treated as tax-free and excluded from the deemed-disposal base; only Neubestand unrealized gains are taxed.",
  "EXIT-002: Neubestand gains estimated at the 27.5% Sondersteuersatz (§ 27a EStG). Loss offset beyond the same income type is out of scope.",
  "EXIT-005: Fair market value uses the best available cached rate at or before the departure date; it does not imply intraday coverage.",
];

const DEFERRAL_NOTE: Record<ExitTaxDestination, string> = {
  eu_eea:
    "EXIT-002: EU/EEA destination — tax is assessed but, on application, not collected until you actually sell (Nichtfestsetzung, § 27 Abs 6 Z 1 lit a). Not the 7-year business-asset Ratenzahlung.",
  third_country:
    "EXIT-002: Third-country (non-EU/EEA) destination — tax is due immediately on departure.",
};

const FUTURE_TIGHTENING =
  "EXIT-003: A planned 1 July 2026 tightening (annual proof when deferred gains exceed EUR 100,000) is not modelled — verify against enacted law.";

/** Deterministic Bitcoin-only fixture: 1.0 BTC Altbestand + 0.4 BTC Neubestand. */
export function buildExitTaxFixture(
  departureDate = "2026-06-16",
  destination: ExitTaxDestination = "eu_eea",
): ExitTaxReport {
  const fmv = 60000;
  const neuSats = 40_000_000; // 0.4 BTC
  const altSats = 100_000_000; // 1.0 BTC
  const neuMarket = (neuSats / 1e8) * fmv; // 24000
  const neuBasis = 16000;
  const neuGain = neuMarket - neuBasis; // 8000
  const altMarket = (altSats / 1e8) * fmv; // 60000
  const taxableGain = Math.max(0, neuGain);
  const rate = 0.275;
  const assumptions = [...BASE_ASSUMPTIONS, DEFERRAL_NOTE[destination]];
  if (destination === "eu_eea") assumptions.push(FUTURE_TIGHTENING);

  return {
    workspace: "Books",
    profile: "Departing",
    jurisdictionCode: "AT",
    fiatCurrency: "EUR",
    departureDate,
    destination,
    method: "moving_average_at",
    fmvSource: [
      { asset: "BTC", rate: fmv, pair: "BTC-EUR", asOf: "2026-06-15T00:00:00Z", source: "cache" },
    ],
    totals: {
      neuQuantitySats: neuSats,
      neuMarketValue: neuMarket,
      neuCostBasis: neuBasis,
      neuGain,
      altQuantitySats: altSats,
      altMarketValue: altMarket,
      taxableGain,
      estimatedTaxRate: rate,
      estimatedTax: taxableGain * rate,
      collectionTiming: destination === "eu_eea" ? "deferred" : "immediate",
    },
    lots: [
      {
        asset: "BTC",
        regime: "neu",
        quantitySats: neuSats,
        marketValue: neuMarket,
        costBasis: neuBasis,
        gain: neuGain,
        taxable: true,
        category: "neu_gain",
        kennzahl: 174,
      },
      {
        asset: "BTC",
        regime: "alt",
        quantitySats: altSats,
        marketValue: altMarket,
        costBasis: null,
        gain: null,
        taxable: false,
        category: "alt_taxfree",
        kennzahl: null,
      },
    ],
    walletHoldings: [
      { wallet: "Cold storage", asset: "BTC", quantitySats: altSats, marketValue: altMarket },
      { wallet: "Hot wallet", asset: "BTC", quantitySats: neuSats, marketValue: neuMarket },
    ],
    assumptions,
    reviewGate: REVIEW_GATE,
    status: { needsJournals: false, quarantines: 0 },
  };
}
