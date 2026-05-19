/**
 * Date formatting helpers shared across detail surfaces.
 *
 * Connection / node / channel detail screens all want the same compact
 * "YYYY-MM-DD HH:mm" rendering of ISO timestamps. Keep the shaping
 * centralised so the formatting stays consistent and easy to evolve.
 */

/**
 * Render an ISO timestamp as a compact "YYYY-MM-DD HH:mm" string.
 *
 * - Returns the em-dash placeholder for empty / null / undefined inputs.
 * - Strips the trailing `Z` and replaces the `T` separator with a space.
 * - Truncates to 16 characters so seconds and fractional seconds are dropped.
 *   Values shorter than 17 characters are passed through unchanged so we do
 *   not mangle already-formatted strings.
 */
export function formatShortDate(value: string | null | undefined): string {
  if (!value) return "—";
  const normalized = value.replace("T", " ").replace(/Z$/, "");
  return normalized.length > 16 ? normalized.slice(0, 16) : normalized;
}
