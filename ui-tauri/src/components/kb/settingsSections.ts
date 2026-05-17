// Mapping from a settings hash slug (`#privacy`, `#ai`, …) to the integration
// id used by the SettingsIntegrations4 panel. Both the native menu and the
// `kassiber://settings/<section>` deep link funnel through this helper, so
// the canonical section names live here rather than duplicated in Rust.

const SETTINGS_SECTION_INTEGRATION: Record<string, string> = {
  privacy: "privacy-sensitive",
  developer: "privacy-developer-tools",
  logs: "privacy-developer-tools",
  display: "display-currency",
  desktop: "terminal-command",
  terminal: "terminal-command",
  explorer: "explorer-links",
  explorers: "explorer-links",
  security: "security-lock-now",
  backends: "sync-add-backend",
  sync: "sync-add-backend",
  rates: "rate-providers",
  ai: "ai-providers",
  assistant: "ai-providers",
  data: "data-root",
};

export function selectedIntegrationForHash(hash: string): string | null {
  const normalized = hash.replace(/^#/, "").trim().toLowerCase();
  return SETTINGS_SECTION_INTEGRATION[normalized] ?? null;
}
