import { localeForFiat } from "@/lib/currency";

export type AccountKind = "asset" | "liability" | "equity" | "income" | "expense";
export type Book = { currency: string; minor_unit_exponent: number; timezone: string; entity_kind: string; accounting_regime: string };
export type BalanceRow = { account_code: string; name: string; kind: AccountKind; debit_minor: string; credit_minor: string; balance_minor: string };
export type Reports = { trial_balance: { rows: BalanceRow[]; debit_minor: string; credit_minor: string; balanced: boolean }; statements: { profit_and_loss: BalanceRow[]; balance_sheet: BalanceRow[]; profit_minor: string; unappropriated_result_minor: string; balanced: boolean } };

export function formatMinor(value: string, book: Pick<Book, "currency" | "minor_unit_exponent">): string {
  const amount = BigInt(value);
  const absolute = amount < 0n ? -amount : amount;
  const scale = 10n ** BigInt(book.minor_unit_exponent);
  const formatter = new Intl.NumberFormat(localeForFiat(book.currency), {
    style: "currency", currency: book.currency,
    minimumFractionDigits: book.minor_unit_exponent, maximumFractionDigits: book.minor_unit_exponent,
  });
  const fraction = (absolute % scale).toString().padStart(book.minor_unit_exponent, "0");
  const result = formatter.formatToParts(absolute / scale).map((part) => part.type === "fraction" ? fraction : part.value).join("");
  return amount < 0n ? `−${result}` : result;
}
