import { MAX_FIX_OPERATIONS } from "./explain";

/** Recorded with each entered price in the transaction's history. */
export const ENTERED_PRICE_REASON = "The owner entered the price on the Review screen.";

/** Whether the owner enters what the whole transaction was worth, or its price per unit. */
export type PriceEntryUnit = "value" | "rate";

// Spaces (including narrow no-break ones) and apostrophes group digits too.
const SPACE_GROUPING = /[\s'’]/;

/**
 * The exact decimal the owner typed, as a plain string the daemon reads
 * without rounding: "2800", "2.800,50", "2,800.50", "2 800,5" or "0,5".
 * Digits are grouped by threes with one kind of separator. A lone separator
 * after one to three digits and before exactly three is read the way the
 * owner's language writes numbers ("2.800" is 2800 in German, 2.8 in
 * English); after a leading zero it is always a decimal ("0.500"). Null when
 * the input is not one unsigned number.
 */
export function exactDecimal(input: string, decimalSeparator: "." | ","): string | null {
  const text = input.trim();
  if (!text || text.length > 40 || !/^[\d.,\s'’]+$/.test(text) || !/\d/.test(text)) return null;
  const dots = text.split(".").length - 1;
  const commas = text.split(",").length - 1;
  let separator: "." | "," | null = null;
  if (dots && commas) {
    separator = text.lastIndexOf(".") > text.lastIndexOf(",") ? "." : ",";
  } else if (dots + commas === 1) {
    const only = dots ? "." : ",";
    const [before, after] = text.split(only);
    const looksGrouped = /^[1-9]\d{0,2}$/.test(before) && /^\d{3}$/.test(after);
    separator = only === decimalSeparator || !looksGrouped ? only : null;
  }
  const at = separator ? text.lastIndexOf(separator) : -1;
  const whole = at >= 0 ? text.slice(0, at) : text;
  const fraction = at >= 0 ? text.slice(at + 1) : "";
  if (at >= 0 && !/^\d+$/.test(fraction)) return null;
  const marks = whole.match(/\D/g) ?? [];
  if (marks.length) {
    // One kind of grouping, never the decimal separator itself.
    const kinds = new Set(marks.map((mark) => (SPACE_GROUPING.test(mark) ? " " : mark)));
    if (kinds.size > 1 || (separator !== null && kinds.has(separator))) return null;
    const groups = whole.split(/\D/);
    if (!/^[1-9]\d{0,2}$/.test(groups[0]) || groups.slice(1).some((group) => !/^\d{3}$/.test(group))) {
      return null;
    }
  }
  const integer = (whole.replace(/\D/g, "") || "0").replace(/^0+(?=\d)/, "");
  return fraction ? `${integer}.${fraction}` : integer;
}

/** Whether a decimal string is above zero. */
export function isPositiveDecimal(value: string) {
  return /[1-9]/.test(value);
}

/** An exact decimal string as the owner's language writes it, every digit kept. */
export function formatExactDecimal(value: string, locale: string) {
  const [integer, fraction] = value.split(".");
  const decimal =
    new Intl.NumberFormat(locale).formatToParts(1.5).find((part) => part.type === "decimal")?.value ?? ".";
  const grouped = BigInt(integer).toLocaleString(locale);
  return fraction ? `${grouped}${decimal}${fraction}` : grouped;
}

export type PriceEntryCheck =
  | { ok: true; operations: Array<Record<string, unknown>> }
  | { ok: false; invalid: string[]; tooMany: boolean };

/**
 * The price overrides for what the owner typed, one per filled row in the
 * order shown. Empty rows are skipped; a row that does not read as a price
 * above zero holds the whole set back, and so do more than one review step
 * takes.
 */
export function priceEntryOperations(
  order: string[],
  entries: Record<string, string>,
  unit: PriceEntryUnit,
  decimalSeparator: "." | ",",
): PriceEntryCheck {
  const invalid: string[] = [];
  const operations: Array<Record<string, unknown>> = [];
  for (const transactionId of order) {
    const typed = entries[transactionId]?.trim();
    if (!typed) continue;
    const value = exactDecimal(typed, decimalSeparator);
    if (value === null || !isPositiveDecimal(value)) {
      invalid.push(transactionId);
      continue;
    }
    operations.push({
      type: "price_override",
      transaction_id: transactionId,
      reason: ENTERED_PRICE_REASON,
      [unit === "rate" ? "fiat_rate" : "fiat_value"]: value,
    });
  }
  const tooMany = operations.length > MAX_FIX_OPERATIONS;
  return invalid.length || tooMany || !operations.length
    ? { ok: false, invalid, tooMany }
    : { ok: true, operations };
}
