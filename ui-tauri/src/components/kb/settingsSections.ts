// Mapping from a settings hash slug (`#privacy`, `#bitcoin`, …) to the
// canonical section id rendered by SettingsScreen's section rail. The native
// menu, the deep-link parser (Rust side), direct URL navigation, and in-app
// rail clicks all funnel through this helper, so the canonical section names
// live here rather than duplicated across the app.
//
// The Rust `DEEP_LINK_SETTINGS_SECTIONS` allowlist (mirrored by
// `SettingsMenuSection` in `menuIntent.ts`) emits both canonical section slugs
// and older menu aliases. All of those must keep resolving here.

export type SettingsSectionId =
  | "general-appearance"
  | "network-bitcoin"
  | "network-lightning"
  | "network-liquid"
  | "network-market"
  | "security-privacy"
  | "security-lock"
  | "assistant-ai"
  | "data-sync"
  | "data-storage"
  | "desktop-terminal"
  | "desktop-developer";

export const PENDING_SETTINGS_BACKEND_EDIT_KEY =
  "kassiber:settings-backend-edit";

const SETTINGS_SECTION_FOR_SLUG: Record<string, SettingsSectionId> = {
  // General
  appearance: "general-appearance",
  display: "general-appearance",
  explorers: "network-bitcoin",
  explorer: "network-bitcoin",
  // Network & layers
  bitcoin: "network-bitcoin",
  backends: "network-bitcoin",
  sync: "data-sync",
  lightning: "network-lightning",
  liquid: "network-liquid",
  market: "network-market",
  rates: "network-market",
  // Privacy & security
  privacy: "security-privacy",
  security: "security-lock",
  lock: "security-lock",
  // Assistant
  ai: "assistant-ai",
  assistant: "assistant-ai",
  // Data
  data: "data-storage",
  storage: "data-storage",
  replication: "data-sync",
  // Desktop
  terminal: "desktop-terminal",
  desktop: "desktop-terminal",
  developer: "desktop-developer",
  logs: "desktop-developer",
};

export function settingsSectionForHash(hash: string): SettingsSectionId | null {
  const normalized = hash.replace(/^#/, "").trim().toLowerCase();
  return SETTINGS_SECTION_FOR_SLUG[normalized] ?? null;
}

/**
 * Canonical URL slug per section — the one that appears in the address bar.
 *
 * `SETTINGS_SECTION_FOR_SLUG` above is deliberately many-to-one (it absorbs the
 * older menu aliases `#backends`, `#lock`, `#storage`, …); this is the inverse
 * one-to-one map, so a deep link on an alias still lands on a single canonical
 * route rather than minting a second URL for the same panel.
 */
export const SETTINGS_SECTION_SLUG = {
  "general-appearance": "appearance",
  "network-market": "market",
  "network-bitcoin": "bitcoin",
  "network-lightning": "lightning",
  "network-liquid": "liquid",
  "security-privacy": "privacy",
  "security-lock": "security",
  "assistant-ai": "ai",
  "data-sync": "sync",
  "data-storage": "data",
  "desktop-terminal": "terminal",
  "desktop-developer": "developer",
} as const satisfies Record<SettingsSectionId, string>;

export type SettingsSectionSlug =
  (typeof SETTINGS_SECTION_SLUG)[SettingsSectionId];

export const DEFAULT_SETTINGS_SECTION_ID: SettingsSectionId =
  "general-appearance";

// Literal route strings, so TanStack Router's `to` stays type-checked at every
// call site instead of degrading to `string`.
export const SETTINGS_SECTION_ROUTE = {
  "general-appearance": "/settings/appearance",
  "network-market": "/settings/market",
  "network-bitcoin": "/settings/bitcoin",
  "network-lightning": "/settings/lightning",
  "network-liquid": "/settings/liquid",
  "security-privacy": "/settings/privacy",
  "security-lock": "/settings/security",
  "assistant-ai": "/settings/ai",
  "data-sync": "/settings/sync",
  "data-storage": "/settings/data",
  "desktop-terminal": "/settings/terminal",
  "desktop-developer": "/settings/developer",
} as const satisfies Record<SettingsSectionId, `/settings/${SettingsSectionSlug}`>;

export type SettingsSectionRoutePath =
  (typeof SETTINGS_SECTION_ROUTE)[SettingsSectionId];

export function settingsSectionRoutePath(
  id: SettingsSectionId,
): SettingsSectionRoutePath {
  return SETTINGS_SECTION_ROUTE[id];
}

/**
 * Resolve a hash/slug (`"privacy"`, `"#backends"`, `null`) to a section route.
 * Unknown or missing values fall back to the default section, which is what
 * makes a bare `/settings` visit land somewhere useful.
 */
export function settingsSectionRoute(
  hashOrSlug: string | null | undefined,
): SettingsSectionRoutePath {
  const section = hashOrSlug ? settingsSectionForHash(hashOrSlug) : null;
  return settingsSectionRoutePath(section ?? DEFAULT_SETTINGS_SECTION_ID);
}

/**
 * Which settings category a `/settings/<slug>` pathname is showing, or null for
 * any non-settings route. Lets the side nav highlight the current category
 * without threading route params through the shell.
 */
export function settingsSectionForPathname(
  pathname: string,
): SettingsSectionId | null {
  const match = /^\/settings\/([^/?#]+)/.exec(pathname);
  if (!match) return null;
  return settingsSectionForHash(match[1]);
}
