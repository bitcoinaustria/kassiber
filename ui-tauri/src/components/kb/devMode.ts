// Pre-release gating for surfaces that are not finished yet.
//
// `developerToolsEnabled` (Settings → Desktop → Developer tools) is the single
// switch: off by default and persisted in this install's `kb.ui` state, so
// whoever turns it on keeps it across updates.
//
// Two treatments, both driven by the same flag:
//   - locked: the nav row stays as a signpost but is greyed out and inert.
//   - hidden: the row disappears and direct navigation redirects to Overview.

import { settingsSectionForHash } from "./settingsSections";

/** Nav rows that stay visible but do not navigate while dev mode is off. */
export const DEV_LOCKED_ROUTES = [
  "/activity",
  "/custody-gaps",
  "/exit-tax",
  "/privacy-mirror",
  "/source-of-funds",
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

const DEV_LOCKED_ROUTE_SET = new Set<string>(DEV_LOCKED_ROUTES);

/**
 * Props that turn a link into an early-stage signpost: still visible (that is
 * the point — it is a sneak peek), greyed out, and inert.
 *
 * `aria-disabled` is what screen readers announce as unavailable and what the
 * shadcn recipes style; the click guard is what actually stops navigation,
 * since the row is still an `<a>` and keyboard activation fires a click too.
 * Deliberately NOT `tabIndex: -1`: a row a keyboard user cannot reach cannot
 * announce itself either. The event is typed structurally so this module stays
 * React-free.
 */
export function devLockProps(
  locked: boolean,
  hint: string,
  className?: string,
) {
  if (!locked) return {};
  return {
    "aria-disabled": true as const,
    className: ["cursor-not-allowed opacity-50", className]
      .filter(Boolean)
      .join(" "),
    title: hint,
    onClick: (event: { preventDefault: () => void }) => event.preventDefault(),
  };
}

/** True for a nav row that should render greyed out and inert. */
export function isDevLockedRoute(route: string): boolean {
  return DEV_LOCKED_ROUTE_SET.has(route);
}

export function isDevHiddenRoute(route: string): boolean {
  return DEV_HIDDEN_ROUTES.includes(route as (typeof DEV_HIDDEN_ROUTES)[number]);
}

/** Settings sections (see `SettingsSectionId`) hidden while dev mode is off. */
const DEV_ONLY_SETTINGS_SECTIONS = new Set<string>(["data-sync"]);

/**
 * Same question for a settings *slug* — what the native menu and the
 * `kassiber://settings/<slug>` deep links carry. Several aliases resolve to one
 * section (`sync`, `replication` → `data-sync`), so route the slug through the
 * canonical map rather than listing the aliases again here.
 */
export function isDevOnlySettingsSlug(
  slug: string | null | undefined,
): boolean {
  const id = slug ? settingsSectionForHash(slug) : null;
  return Boolean(id && isDevOnlySettingsSection(id));
}

/** True for any route the app search must not offer while dev mode is off. */
export function isDevOnlyRoute(to: string | undefined | null): boolean {
  return Boolean(to) && DEV_ONLY_ROUTES.has(to as string);
}

export function isDevOnlySettingsSection(id: string): boolean {
  return DEV_ONLY_SETTINGS_SECTIONS.has(id);
}
