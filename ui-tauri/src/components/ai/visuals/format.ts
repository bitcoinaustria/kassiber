import { formatCompactFiatAmount, formatFiatAmount } from "@/lib/currency";

/** Number formatting for assistant visuals, in the UI language. */
export interface VisualFormat {
  locale: string;
  fiat: (value: number) => string;
  fiatCompact: (value: number) => string;
  btc: (value: number) => string;
  btcCompact: (value: number) => string;
  sats: (value: number) => string;
  asset: (value: number, asset: string) => string;
  count: (value: number) => string;
  percent: (value: number) => string;
  signed: (formatted: string, value: number) => string;
  date: (iso: string, style?: DateStyle) => string;
}

export type DateStyle = "day" | "week" | "month" | "quarter" | "hour" | "year" | "short";

const BTC_ASSETS = new Set(["BTC", "LBTC", "L-BTC", "LNBTC"]);

export function visualFormat(locale: string, fiatCurrency: string): VisualFormat {
  const plain = (digits: number, minimum = 0) =>
    new Intl.NumberFormat(locale, {
      minimumFractionDigits: minimum,
      maximumFractionDigits: digits,
    });
  const btcFull = plain(8);
  const count = plain(0);
  const percent = new Intl.NumberFormat(locale, {
    style: "percent",
    maximumFractionDigits: 1,
  });
  const btc = (value: number) => `₿ ${btcFull.format(value)}`;
  return {
    locale,
    fiat: (value) => formatFiatAmount(value, fiatCurrency),
    fiatCompact: (value) => formatCompactFiatAmount(value, fiatCurrency),
    btc,
    btcCompact: (value) => {
      const magnitude = Math.abs(value);
      const digits = magnitude >= 10 ? 1 : magnitude >= 1 ? 2 : magnitude >= 0.01 ? 4 : 6;
      return `₿ ${plain(digits).format(value)}`;
    },
    sats: (value) => `${count.format(value)} sat`,
    asset: (value, asset) =>
      BTC_ASSETS.has(asset.toUpperCase()) || !asset
        ? `${btc(value)}${asset && asset.toUpperCase() !== "BTC" ? ` ${asset}` : ""}`
        : `${btcFull.format(value)} ${asset}`,
    count: (value) => count.format(value),
    percent: (value) => percent.format(value),
    signed: (formatted, value) =>
      value > 0 ? `+${formatted}` : value < 0 && !formatted.includes("-") && !formatted.includes("−") ? `−${formatted}` : formatted,
    date: (iso, style = "short") => formatDate(iso, style, locale),
  };
}

function formatDate(iso: string, style: DateStyle, locale: string): string {
  if (/^\d{4}$/.test(iso)) return iso;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const utc = { timeZone: "UTC" } as const;
  switch (style) {
    case "year":
      return String(date.getUTCFullYear());
    case "quarter":
      return `Q${Math.floor(date.getUTCMonth() / 3) + 1} ${date.getUTCFullYear()}`;
    case "month":
      return new Intl.DateTimeFormat(locale, { ...utc, month: "short", year: "numeric" }).format(date);
    case "hour":
      return new Intl.DateTimeFormat(locale, { ...utc, day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(date);
    case "day":
    case "week":
      return new Intl.DateTimeFormat(locale, { ...utc, day: "numeric", month: "short" }).format(date);
    default:
      return new Intl.DateTimeFormat(locale, { ...utc, day: "numeric", month: "short", year: "numeric" }).format(date);
  }
}

/** Proportion of each value in a set, ignoring unknown and negative values. */
export function shares(values: Array<number | null>): number[] {
  const total = values.reduce<number>((sum, value) => sum + Math.max(value ?? 0, 0), 0);
  return values.map((value) => (total > 0 ? Math.max(value ?? 0, 0) / total : 0));
}

/** Quote a CSV cell; a leading formula character is neutralised for spreadsheets. */
export function csvCell(value: string): string {
  const safe = /^[=+\-@\t\r]/.test(value) ? `'${value}` : value;
  return /[",\n\r]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
}
