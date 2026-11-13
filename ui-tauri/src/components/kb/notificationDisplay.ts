/** A progress value clamped to 0–100 for a bar width. */
export function notificationProgressValue(value: number | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}
