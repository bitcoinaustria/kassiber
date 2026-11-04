/**
 * The parts of `ui.reports.capital_gains` the Reports screen reads. The
 * payload also carries the year's newest rows for the assistant; the screen
 * shows the year's totals and their breakdown and leaves rows to Transactions.
 */

export type CostBasisMethod =
  | "fifo"
  | "lifo"
  | "hifo"
  | "lofo"
  | "moving_average"
  | "moving_average_at";

export interface KennzahlRow {
  code: string;
  label: string;
  form?: string;
  formSection?: string;
  amount: number | null;
  amountEurCents?: number;
  rowCount: number;
  source?: "daemon" | "mock" | "pending";
  note?: string;
}

/**
 * One group of the year's rows: an Austrian field (`172`, `174`, `176`,
 * `801`), `tax_free` Altbestand, tax-neutral `swaps`, or `unassigned`; for a
 * generic book `short_term`, `long_term`, or `income`.
 */
export interface ReportBreakdownBucket {
  key: string;
  rowCount: number;
  transactionCount: number;
  /** Its transactions, or `null` when too many for an exact link. */
  transactionIds: string[] | null;
  costEur: number;
  proceedsEur: number;
  incomeEur: number;
  gainLossEur: number;
}

export interface CapitalGainsReport {
  /** `AT` or `GENERIC`, from the book's tax country. */
  jurisdictionCode: string;
  /** The book's fiat; every amount in the report is in it. */
  fiatCurrency?: string;
  /** Generic books' long-term holding period; `null` for Austria. */
  longTermDays?: number | null;
  year: number;
  availableYears?: number[];
  method: CostBasisMethod;
  /** Over every disposal and income row of the year. */
  totals?: {
    rowCount: number;
    costEur: number;
    proceedsEur: number;
    incomeEur: number;
    gainLossEur: number;
  };
  breakdown?: ReportBreakdownBucket[];
  kennzahlRows?: KennzahlRow[];
  /**
   * `false` for an Austrian year before 2022, which has no crypto Kennzahlen
   * (KZ 172, 174, 176): none of them and no estimate. Missing from an older
   * daemon means they apply.
   */
  e1kvApplies?: boolean;
  /**
   * Rows classified for a crypto Kennzahl but dated before 1 March 2022 (a
   * 2022 book's January and February): the E 1kv export refuses the year.
   */
  rowsBeforeCryptoKennzahlen?: number;
  /** The core's indicative Austrian tax (EUR books only), blockers ignored. */
  indicativeTax?: {
    estimatedTaxEurCents: number | null;
    rate: string;
    basis: string;
    progressiveRateAmountsEurCents: Record<string, number>;
    notes: string[];
  } | null;
  status?: {
    needsJournals: boolean;
    quarantines: number;
  };
}
