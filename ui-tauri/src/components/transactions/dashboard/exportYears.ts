/** Last completed calendar year when the book covers it; else its newest year. */
export function defaultExportYear(years: number[], now = new Date()): number | null {
  const previous = now.getFullYear() - 1;
  if (years.includes(previous)) return previous;
  return years[0] ?? null;
}
