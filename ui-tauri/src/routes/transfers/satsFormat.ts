/**
 * Exact millisatoshi amounts as sats, without passing through a float: the
 * whole sats are formatted from a BigInt and a sub-sat remainder (Lightning)
 * is appended with the locale's decimal separator.
 */
import { localeForLanguage } from "@/i18n/config";
import { currentUiLocale } from "@/lib/localeFormat";

export function formatMsatAsSats(value: string | number, language?: string): string {
  let msat: bigint;
  try {
    msat = BigInt(typeof value === "number" ? Math.trunc(value) : value.trim());
  } catch {
    return "—";
  }
  const negative = msat < 0n;
  const absolute = negative ? -msat : msat;
  const whole = absolute / 1000n;
  const remainder = absolute % 1000n;
  // The app's own locale for the UI language, as every other amount uses.
  const format = new Intl.NumberFormat(language ? localeForLanguage(language) : currentUiLocale());
  let text = format.format(whole);
  if (remainder !== 0n) {
    const separator = format.formatToParts(1.5).find((part) => part.type === "decimal")?.value ?? ".";
    text += separator + remainder.toString().padStart(3, "0").replace(/0+$/, "");
  }
  return `${negative ? "−" : ""}${text} sats`;
}

function parseMsat(value: string | number | bigint): bigint | null {
  try {
    if (typeof value === "bigint") return value;
    return BigInt(typeof value === "number" ? Math.trunc(value) : value.trim());
  } catch {
    return null;
  }
}

/**
 * Exact millisatoshis as bitcoin the way the Transactions table writes it
 * (`₿0.00600000`): eight decimals, and the extra millisatoshi digits only
 * when a Lightning amount has them. No float is involved.
 */
export function formatMsatAsBtc(value: string | number | bigint): string {
  const msat = parseMsat(value);
  if (msat === null) return "—";
  const absolute = msat < 0n ? -msat : msat;
  const whole = absolute / 100_000_000_000n;
  const fraction = (absolute % 100_000_000_000n).toString().padStart(11, "0");
  const sats = fraction.slice(0, 8);
  const subSat = fraction.slice(8).replace(/0+$/, "");
  return `₿${whole.toString()}.${sats}${subSat}`;
}

/**
 * An exact amount of any asset from its stored units (a hundred billion to
 * one, like millisatoshis to a bitcoin): bitcoin as `₿0.00600000`, another
 * asset as its number and code (`120.5 USDT`). No float is involved.
 */
export function formatMsatAsAsset(value: string | number | bigint, asset: string, bitcoin: boolean): string {
  if (bitcoin) return formatMsatAsBtc(value);
  const msat = parseMsat(value);
  if (msat === null) return "—";
  const absolute = msat < 0n ? -msat : msat;
  const whole = absolute / 100_000_000_000n;
  const fraction = (absolute % 100_000_000_000n).toString().padStart(11, "0").replace(/0+$/, "");
  return `${whole.toString()}${fraction ? `.${fraction}` : ""} ${asset}`;
}

/** The sum of exact millisatoshi amounts; unreadable ones are skipped. */
export function sumMsat(values: Iterable<string | number | bigint | null | undefined>): bigint {
  let total = 0n;
  for (const value of values) {
    if (value === null || value === undefined) continue;
    const msat = parseMsat(value);
    if (msat !== null) total += msat;
  }
  return total;
}
