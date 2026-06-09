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
  sync: "network-bitcoin",
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
