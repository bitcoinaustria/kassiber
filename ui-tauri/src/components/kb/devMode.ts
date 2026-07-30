// Pre-release gating for surfaces that are not finished yet.
//
// `developerToolsEnabled` (Settings → Desktop → Developer tools) is the single
// switch: off by default, persisted per profile in `kb.ui`, so a developer who
// turns it on keeps it across updates.
//
// Two treatments, both driven by the same flag:
//   - locked: the nav row stays as a signpost but is greyed out and inert.
//   - hidden: the row disappears and direct navigation redirects to Overview.

/** Nav rows that stay visible but do not navigate while dev mode is off. */
export const DEV_LOCKED_ROUTES = [
  "/activity",
  "/custody-gaps",
  "/exit-tax",
  "/privacy-mirror",
] as const;

/** Routes that vanish entirely while dev mode is off. */
export const DEV_HIDDEN_ROUTES = [
  "/egress",
  "/logs",
  "/settings/sync",
] as const;

const DEV_ONLY_ROUTES = new Set<string>([
  ...DEV_LOCKED_ROUTES,
  ...DEV_HIDDEN_ROUTES,
]);

/** Settings sections (see `SettingsSectionId`) hidden while dev mode is off. */
const DEV_ONLY_SETTINGS_SECTIONS = new Set<string>(["data-sync"]);

/** True for any route the app search must not offer while dev mode is off. */
export function isDevOnlyRoute(to: string | undefined | null): boolean {
  return Boolean(to) && DEV_ONLY_ROUTES.has(to as string);
}

export function isDevOnlySettingsSection(id: string): boolean {
  return DEV_ONLY_SETTINGS_SECTIONS.has(id);
}
