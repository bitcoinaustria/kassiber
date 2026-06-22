import { APP_ROUTE_PATHS, type AppRoutePath } from "./menuIntent";

function asRoutePath(value: string | undefined): AppRoutePath | undefined {
  return value && (APP_ROUTE_PATHS as readonly string[]).includes(value)
    ? (value as AppRoutePath)
    : undefined;
}

// Map a notification title to the screen that explains it.
export function notificationRouteFor(title: string): AppRoutePath | undefined {
  const normalized = title.toLowerCase();
  // Failures / "needs attention" go to Logs, which captures freshness job
  // errors (freshness._mark_error logs to the RAM ring). Checked FIRST so a
  // title like "Book refresh needs attention" isn't captured by the generic
  // "book"/"sync" keywords below and sent to an unrelated (empty) screen.
  if (
    normalized.includes("needs attention") ||
    normalized.includes("failed") ||
    normalized.includes("error") ||
    normalized.includes("daemon")
  ) {
    return "/logs";
  }
  if (normalized.includes("journal")) return "/journals";
  if (
    normalized.includes("quarantine") ||
    normalized.includes("quarantäne") ||
    normalized.includes("quarantaene")
  ) {
    return "/quarantine";
  }
  if (normalized.includes("sync") || normalized.includes("wallet")) {
    return "/connections";
  }
  if (normalized.includes("report") || normalized.includes("export")) {
    return "/reports";
  }
  if (normalized.includes("book") || normalized.includes("books")) {
    return "/books";
  }
  if (normalized.includes("transaction")) return "/transactions";
  return undefined;
}

// Resolve the click target for a header notification, accounting for the fact
// that /logs is developer-tools-gated: its route guard bounces to /overview
// when developer tools are off. A failure notification that routed straight to
// /logs would therefore dead-end. Send those users to /settings (where the
// developer-tools toggle lives) instead — for both the title router's /logs
// result and the error-tone fallback. Mirrors the search guard in appSearch.ts.
export function notificationTarget(
  title: string,
  tone: string | undefined,
  developerToolsEnabled: boolean,
  explicitTarget?: string,
): AppRoutePath | undefined {
  const logsOrSettings: AppRoutePath = developerToolsEnabled
    ? "/logs"
    : "/settings";
  // An explicit, language-independent target set by the notification's producer
  // takes precedence over title keyword-matching (which only works in English).
  // It still flows through the /logs → /settings developer-tools guard below.
  const target = asRoutePath(explicitTarget) ?? notificationRouteFor(title);
  if (target === "/logs") return logsOrSettings;
  if (target) return target;
  return tone === "error" ? logsOrSettings : undefined;
}
