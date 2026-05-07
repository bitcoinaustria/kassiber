/**
 * Mock data for the Reports screen — capital-gains export flow.
 *
 * Lifted from claude-design/screens/tax.jsx. These shapes drive the UI
 * translation against realistic data; the Pydantic→JSON Schema pipeline
 * (Phase 1.2 §2.2) will replace these with schema-driven factories.
 */

export type CostBasisMethod =
  | "fifo"
  | "lifo"
  | "hifo"
  | "lofo"
  | "moving_average"
  | "moving_average_at";

export interface Jurisdiction {
  code: string;
  name: string;
  policy: string;
  rate: number;
  rateLabel: string;
  defaultMethod: CostBasisMethod;
  methodLocked?: boolean;
  methodNote?: string;
  internalsNonTaxable: boolean;
  longTermDays: number;
  ccy: string;
  locale: string;
}

export const JURISDICTIONS: Record<string, Jurisdiction> = {
  AT: {
    code: "AT",
    name: "Austria",
    policy: "§27a/§27b EStG · KESt 27,5 %",
    rate: 0.275,
    rateLabel: "KESt 27,5 %",
    defaultMethod: "moving_average_at",
    methodLocked: true,
    methodNote: "Old stock: FIFO; new stock: average cost",
    internalsNonTaxable: true,
    longTermDays: 365,
    ccy: "€",
    locale: "de-AT",
  },
  DE: {
    code: "DE",
    name: "Germany",
    policy: "§ 23 EStG · private sales",
    rate: 0.26375,
    rateLabel: "Est. 26,375 %",
    defaultMethod: "fifo",
    internalsNonTaxable: true,
    longTermDays: 365,
    ccy: "€",
    locale: "de-DE",
  },
  CH: {
    code: "CH",
    name: "Switzerland",
    policy: "Private wealth · tax-exempt",
    rate: 0.0,
    rateLabel: "Private · 0 %",
    defaultMethod: "fifo",
    internalsNonTaxable: true,
    longTermDays: 0,
    ccy: "CHF",
    locale: "de-CH",
  },
  XX: {
    code: "XX",
    name: "Generic",
    policy: "Generic capital-gains preset",
    rate: 0.2,
    rateLabel: "Est. 20 %",
    defaultMethod: "fifo",
    internalsNonTaxable: true,
    longTermDays: 365,
    ccy: "€",
    locale: "en-GB",
  },
};

export type LotType = "LT" | "ST";

export interface DisposedLot {
  acquired: string;
  disposed: string;
  sats: number;
  costEur: number;
  proceedsEur: number;
  type: LotType;
}

export interface KennzahlRow {
  code: string;
  label: string;
  amount: number | null;
  rowCount: number;
  source?: "daemon" | "mock" | "pending";
  note?: string;
}

export const MOCK_LOTS: DisposedLot[] = [
  { acquired: "2022-03-18", disposed: "2025-11-04", sats: 12_000_000, costEur: 3_851.20, proceedsEur: 8_204.18, type: "LT" },
  { acquired: "2023-07-02", disposed: "2025-11-04", sats: 8_000_000, costEur: 2_412.00, proceedsEur: 5_469.45, type: "LT" },
  { acquired: "2024-11-14", disposed: "2025-12-01", sats: 3_500_000, costEur: 2_188.70, proceedsEur: 2_392.08, type: "ST" },
  { acquired: "2025-02-09", disposed: "2025-12-20", sats: 1_800_000, costEur: 1_011.55, proceedsEur: 1_290.12, type: "ST" },
  { acquired: "2025-04-22", disposed: "2026-01-08", sats: 900_000, costEur: 614.90, proceedsEur: 635.14, type: "ST" },
];

export interface CapitalGainsReport {
  jurisdictionCode: string;
  year: number;
  availableYears?: number[];
  method: CostBasisMethod;
  lots: DisposedLot[];
  kennzahlRows?: KennzahlRow[];
  status?: {
    needsJournals: boolean;
    quarantines: number;
  };
}

export const MOCK_CAPITAL_GAINS: CapitalGainsReport = {
  jurisdictionCode: "AT",
  year: 2025,
  availableYears: [2026, 2025],
  method: "moving_average_at",
  lots: MOCK_LOTS,
  kennzahlRows: [
    {
      code: "172",
      label: "Foreign recurring crypto income",
      amount: 239.74,
      rowCount: 2,
      source: "mock",
    },
    {
      code: "174",
      label: "Foreign realized crypto gains",
      amount: 3535.55,
      rowCount: 3,
      source: "mock",
    },
    {
      code: "176",
      label: "Foreign realized crypto losses",
      amount: 0,
      rowCount: 0,
      source: "mock",
    },
    {
      code: "801",
      label: "Legacy holdings speculation gains",
      amount: 7410.43,
      rowCount: 2,
      source: "mock",
      note: "Outside E 1kv",
    },
  ],
};
