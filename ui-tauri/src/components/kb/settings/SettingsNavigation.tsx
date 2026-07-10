import {
  Bitcoin,
  Bot,
  Droplets,
  Eye,
  HardDrive,
  LineChart,
  Lock,
  Palette,
  RefreshCw,
  Terminal,
  Wrench,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import type { SettingsSectionId } from "../settingsSections";

export type SettingsGroup =
  | "General"
  | "On-chain & off-chain data"
  | "Privacy & security"
  | "Assistant"
  | "Data"
  | "Desktop";

export interface SettingsSectionMeta {
  id: SettingsSectionId;
  slug: string;
  group: SettingsGroup;
  groupKey: string;
  label: string;
  labelKey: string;
  description: string;
  descriptionKey: string;
  icon: LucideIcon;
}

// Stable English `group`/`label`/`description` feed the (non-localized) app
// search index; `*Key` fields drive the localized rail/header via t().
export const SETTINGS_GROUP_KEYS: Record<SettingsGroup, string> = {
  General: "nav.group.general",
  "On-chain & off-chain data": "nav.group.data",
  "Privacy & security": "nav.group.privacySecurity",
  Assistant: "nav.group.assistant",
  Data: "nav.group.dataGroup",
  Desktop: "nav.group.desktop",
};

export const SETTINGS_SECTIONS: SettingsSectionMeta[] = [
  {
    id: "general-appearance",
    slug: "appearance",
    group: "General",
    groupKey: SETTINGS_GROUP_KEYS.General,
    label: "Appearance",
    labelKey: "nav.section.appearance.label",
    description: "Theme, denomination, and interface scale.",
    descriptionKey: "nav.section.appearance.description",
    icon: Palette,
  },
  {
    id: "network-market",
    slug: "market",
    group: "General",
    groupKey: SETTINGS_GROUP_KEYS.General,
    label: "Market data",
    labelKey: "nav.section.market.label",
    description: "Fiat reference-rate sources and the local pricing cache.",
    descriptionKey: "nav.section.market.description",
    icon: LineChart,
  },
  {
    id: "network-bitcoin",
    slug: "bitcoin",
    group: "On-chain & off-chain data",
    groupKey: SETTINGS_GROUP_KEYS["On-chain & off-chain data"],
    label: "Bitcoin",
    labelKey: "nav.section.bitcoin.label",
    description:
      "Base-layer indexers and nodes used to refresh on-chain wallets.",
    descriptionKey: "nav.section.bitcoin.description",
    icon: Bitcoin,
  },
  {
    id: "network-lightning",
    slug: "lightning",
    group: "On-chain & off-chain data",
    groupKey: SETTINGS_GROUP_KEYS["On-chain & off-chain data"],
    label: "Lightning",
    labelKey: "nav.section.lightning.label",
    description:
      "Read-only Lightning node connections for accounting and profitability.",
    descriptionKey: "nav.section.lightning.description",
    icon: Zap,
  },
  {
    id: "network-liquid",
    slug: "liquid",
    group: "On-chain & off-chain data",
    groupKey: SETTINGS_GROUP_KEYS["On-chain & off-chain data"],
    label: "Liquid",
    labelKey: "nav.section.liquid.label",
    description: "Sidechain indexers used to refresh Liquid (L-BTC) wallets.",
    descriptionKey: "nav.section.liquid.description",
    icon: Droplets,
  },
  {
    id: "security-privacy",
    slug: "privacy",
    group: "Privacy & security",
    groupKey: SETTINGS_GROUP_KEYS["Privacy & security"],
    label: "Privacy",
    labelKey: "nav.section.privacy.label",
    description: "Control what is shown on screen and what leaves your machine.",
    descriptionKey: "nav.section.privacy.description",
    icon: Eye,
  },
  {
    id: "security-lock",
    slug: "security",
    group: "Privacy & security",
    groupKey: SETTINGS_GROUP_KEYS["Privacy & security"],
    label: "Lock & encryption",
    labelKey: "nav.section.lock.label",
    description: "App lock, biometric unlock, and the database passphrase.",
    descriptionKey: "nav.section.lock.description",
    icon: Lock,
  },
  {
    id: "assistant-ai",
    slug: "ai",
    group: "Assistant",
    groupKey: SETTINGS_GROUP_KEYS.Assistant,
    label: "AI providers",
    labelKey: "nav.section.ai.label",
    description: "Local and remote assistant endpoints and their data posture.",
    descriptionKey: "nav.section.ai.description",
    icon: Bot,
  },
  {
    id: "data-sync",
    slug: "sync",
    group: "Data",
    groupKey: SETTINGS_GROUP_KEYS.Data,
    label: "Device sync",
    labelKey: "nav.section.sync.label",
    description: "Encrypted multi-device and team replication through storage you control.",
    descriptionKey: "nav.section.sync.description",
    icon: RefreshCw,
  },
  {
    id: "data-storage",
    slug: "data",
    group: "Data",
    groupKey: SETTINGS_GROUP_KEYS.Data,
    label: "Data & storage",
    labelKey: "nav.section.storage.label",
    description: "Backups, label imports, the local database, and reset tools.",
    descriptionKey: "nav.section.storage.description",
    icon: HardDrive,
  },
  {
    id: "desktop-terminal",
    slug: "terminal",
    group: "Desktop",
    groupKey: SETTINGS_GROUP_KEYS.Desktop,
    label: "Terminal integration",
    labelKey: "nav.section.terminal.label",
    description: "Install the kassiber CLI launcher for your shell.",
    descriptionKey: "nav.section.terminal.description",
    icon: Terminal,
  },
  {
    id: "desktop-developer",
    slug: "developer",
    group: "Desktop",
    groupKey: SETTINGS_GROUP_KEYS.Desktop,
    label: "Developer tools",
    labelKey: "nav.section.developer.label",
    description: "The in-app Logs view and its in-memory buffer.",
    icon: Wrench,
    descriptionKey: "nav.section.developer.description",
  },
];

export const SETTINGS_GROUP_ORDER: SettingsGroup[] = [
  "General",
  "On-chain & off-chain data",
  "Privacy & security",
  "Assistant",
  "Data",
  "Desktop",
];

export const DEFAULT_SETTINGS_SECTION: SettingsSectionId = "general-appearance";

export function sectionMeta(id: SettingsSectionId): SettingsSectionMeta {
  return (
    SETTINGS_SECTIONS.find((section) => section.id === id) ??
    SETTINGS_SECTIONS[0]
  );
}

export function SettingsRail({
  activeId,
  onSelect,
  counts,
}: {
  activeId: SettingsSectionId;
  onSelect: (id: SettingsSectionId) => void;
  counts: Partial<Record<SettingsSectionId, number>>;
}) {
  const { t } = useTranslation("settings");
  return (
    <nav
      aria-label={t("nav.ariaLabel")}
      className="lg:sticky lg:top-4 lg:w-[236px] lg:shrink-0 lg:self-start"
    >
      <div className="flex flex-col gap-5">
        {SETTINGS_GROUP_ORDER.map((group) => {
          const items = SETTINGS_SECTIONS.filter(
            (section) => section.group === group,
          );
          if (items.length === 0) return null;
          return (
            <div key={group} className="space-y-1.5">
              <p className="kb-mono-caption px-2.5">
                {/* dynamic key */}
                {t(SETTINGS_GROUP_KEYS[group] as never)}
              </p>
              <div className="flex flex-wrap gap-1 lg:flex-col">
                {items.map((section) => {
                  const Icon = section.icon;
                  const active = section.id === activeId;
                  const count = counts[section.id];
                  return (
                    <button
                      key={section.id}
                      type="button"
                      aria-current={active ? "page" : undefined}
                      onClick={() => onSelect(section.id)}
                      className={cn(
                        "flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm transition-colors",
                        active
                          ? "bg-muted font-medium text-foreground"
                          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                      )}
                    >
                      <Icon className="size-4 shrink-0" aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate">
                        {/* dynamic key */}
                        {t(section.labelKey as never)}
                      </span>
                      {typeof count === "number" && count > 0 ? (
                        <span className="text-xs tabular-nums text-muted-foreground">
                          {count}
                        </span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </nav>
  );
}
