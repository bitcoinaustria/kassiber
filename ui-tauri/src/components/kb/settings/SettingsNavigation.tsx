import {
  Bitcoin,
  Bot,
  Droplets,
  Eye,
  HardDrive,
  LineChart,
  Lock,
  Palette,
  Terminal,
  Wrench,
  Zap,
  type LucideIcon,
} from "lucide-react";

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
  label: string;
  description: string;
  icon: LucideIcon;
}

export const SETTINGS_SECTIONS: SettingsSectionMeta[] = [
  {
    id: "general-appearance",
    slug: "appearance",
    group: "General",
    label: "Appearance",
    description: "Theme, denomination, and interface scale.",
    icon: Palette,
  },
  {
    id: "network-market",
    slug: "market",
    group: "General",
    label: "Market data",
    description: "Fiat reference-rate sources and the local pricing cache.",
    icon: LineChart,
  },
  {
    id: "network-bitcoin",
    slug: "bitcoin",
    group: "On-chain & off-chain data",
    label: "Bitcoin",
    description:
      "Base-layer indexers and nodes used to refresh on-chain wallets.",
    icon: Bitcoin,
  },
  {
    id: "network-lightning",
    slug: "lightning",
    group: "On-chain & off-chain data",
    label: "Lightning",
    description:
      "Read-only Lightning node connections for accounting and profitability.",
    icon: Zap,
  },
  {
    id: "network-liquid",
    slug: "liquid",
    group: "On-chain & off-chain data",
    label: "Liquid",
    description: "Sidechain indexers used to refresh Liquid (L-BTC) wallets.",
    icon: Droplets,
  },
  {
    id: "security-privacy",
    slug: "privacy",
    group: "Privacy & security",
    label: "Privacy",
    description: "Control what is shown on screen and what leaves your machine.",
    icon: Eye,
  },
  {
    id: "security-lock",
    slug: "security",
    group: "Privacy & security",
    label: "Lock & encryption",
    description: "App lock, biometric unlock, and the database passphrase.",
    icon: Lock,
  },
  {
    id: "assistant-ai",
    slug: "ai",
    group: "Assistant",
    label: "AI providers",
    description: "Local and remote assistant endpoints and their data posture.",
    icon: Bot,
  },
  {
    id: "data-storage",
    slug: "data",
    group: "Data",
    label: "Data & storage",
    description: "Backups, label imports, the local database, and reset tools.",
    icon: HardDrive,
  },
  {
    id: "desktop-terminal",
    slug: "terminal",
    group: "Desktop",
    label: "Terminal integration",
    description: "Install the kassiber CLI launcher for your shell.",
    icon: Terminal,
  },
  {
    id: "desktop-developer",
    slug: "developer",
    group: "Desktop",
    label: "Developer tools",
    description: "The in-app Logs view and its in-memory buffer.",
    icon: Wrench,
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
  return (
    <nav
      aria-label="Settings sections"
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
              <p className="kb-mono-caption px-2.5">{group}</p>
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
                        {section.label}
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
