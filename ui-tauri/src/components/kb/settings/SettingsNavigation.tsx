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
import { Link } from "@tanstack/react-router";
import { ArrowLeft, Settings as SettingsIcon } from "lucide-react";

import {
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { cn } from "@/lib/utils";
import {
  settingsSectionRoutePath,
  type SettingsSectionId,
  type SettingsSectionSlug,
} from "../settingsSections";

export type SettingsGroup =
  | "General"
  | "On-chain & off-chain data"
  | "Privacy & security"
  | "Assistant"
  | "Data"
  | "Desktop";

export interface SettingsSectionMeta {
  id: SettingsSectionId;
  /**
   * Canonical URL slug. Typed against `SETTINGS_SECTION_SLUG` so a section can
   * never advertise a slug the router has no route for.
   */
  slug: SettingsSectionSlug;
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

export function sectionMeta(id: SettingsSectionId): SettingsSectionMeta {
  return (
    SETTINGS_SECTIONS.find((section) => section.id === id) ??
    SETTINGS_SECTIONS[0]
  );
}

/**
 * Settings navigation as a side-nav "page".
 *
 * On any `/settings/*` route the side nav swaps its book navigation for this
 * one, so the settings categories get the full height of the nav instead of a
 * cramped in-page rail — and each row is a real route, not in-page state.
 * "Back" at the bottom returns to the book navigation, mirroring the Settings
 * entry point that sits in the same footer slot.
 *
 * Returns the nav's content + footer as a fragment (not a `<Sidebar>`), so the
 * shell owns the frame and both nav modes share one collapse/resize behaviour.
 */
export function SettingsSidebarNav({
  activeId,
  counts,
}: {
  activeId: SettingsSectionId;
  counts: Partial<Record<SettingsSectionId, number>>;
}) {
  const { t } = useTranslation("settings");
  const { isMobile, setOpenMobile } = useSidebar();
  const closeMobileNav = () => {
    if (isMobile) setOpenMobile(false);
  };

  return (
    <>
      <SidebarContent className="gap-0 overflow-x-hidden">
        <SidebarGroup className="gap-1 px-2 pt-3 pb-1">
          <div className="flex h-8 items-center gap-2 px-2 text-sm font-semibold text-sidebar-foreground">
            <SettingsIcon className="size-4 shrink-0" aria-hidden="true" />
            <span className="truncate group-data-[collapsible=icon]:hidden">
              {t("page.title")}
            </span>
          </div>
        </SidebarGroup>
        {SETTINGS_GROUP_ORDER.map((group) => {
          const items = SETTINGS_SECTIONS.filter(
            (section) => section.group === group,
          );
          if (items.length === 0) return null;
          return (
            <SidebarGroup key={group} className="gap-1.5 px-2 py-1.5">
              {/* Same label recipe as the book nav's groups (T3Code's
                  "Projects" label), so both nav modes read identically. */}
              <p className="mb-1 px-2 text-xs font-medium text-sidebar-muted-foreground/80 group-data-[collapsible=icon]:hidden">
                {/* dynamic key */}
                {t(SETTINGS_GROUP_KEYS[group] as never)}
              </p>
              <SidebarMenu>
                {items.map((section) => {
                  const Icon = section.icon;
                  const active = section.id === activeId;
                  const count = counts[section.id];
                  // dynamic key
                  const label = t(section.labelKey as never) as string;
                  return (
                    <SidebarMenuItem key={section.id}>
                      <SidebarMenuButton
                        asChild
                        isActive={active}
                        tooltip={label}
                        className={cn(
                          "h-8 gap-2 rounded-md text-sm font-medium",
                          active
                            ? "bg-sidebar-row-active text-sidebar-foreground"
                            : "text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground",
                        )}
                      >
                        <Link
                          to={settingsSectionRoutePath(section.id)}
                          aria-current={active ? "page" : undefined}
                          onClick={closeMobileNav}
                        >
                          <Icon
                            className={cn(
                              "size-4 shrink-0",
                              active ? "" : "opacity-70",
                            )}
                            aria-hidden="true"
                          />
                          <span className="min-w-0 flex-1 truncate">
                            {label}
                          </span>
                          {typeof count === "number" && count > 0 ? (
                            <span className="shrink-0 text-xs tabular-nums text-sidebar-muted-foreground group-data-[collapsible=icon]:hidden">
                              {count}
                            </span>
                          ) : null}
                        </Link>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  );
                })}
              </SidebarMenu>
            </SidebarGroup>
          );
        })}
      </SidebarContent>
      {/* Same hairline the book nav's footer carries, so both nav modes
          separate their footer row from the list above it identically. */}
      <SidebarFooter className="border-t border-sidebar-border/60 p-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              asChild
              tooltip={t("nav.back")}
              className="h-8 gap-2 rounded-md text-sm font-medium text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
            >
              <Link to="/overview" onClick={closeMobileNav}>
                <ArrowLeft className="size-4 shrink-0" aria-hidden="true" />
                <span>{t("nav.back")}</span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </>
  );
}
