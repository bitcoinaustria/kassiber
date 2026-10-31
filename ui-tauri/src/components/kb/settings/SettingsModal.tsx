/**
 * Settings as a large modal over the page that opened it, after T3 Code's
 * `AppRouteModal`: the category rail on the left, a breadcrumb header with the
 * close button, and the category below. Esc, the close button, or a press on
 * the backdrop returns to the page, which stayed mounted underneath.
 */

import * as React from "react";
import { useRouterState } from "@tanstack/react-router";
import { Dialog as DialogPrimitive } from "radix-ui";
import { useTranslation } from "react-i18next";
import { ChevronRight, X } from "lucide-react";

import { SettingsScreen } from "@/components/kb/SettingsScreen";
import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

import { isDevOnlySettingsSection } from "../devMode";
import {
  DEFAULT_SETTINGS_SECTION_ID,
  type SettingsSectionId,
} from "../settingsSections";
import {
  backendRowToSettingsBackend,
  backendsForLayer,
  type BackendSettingsData,
} from "./SettingsModel";
import { SettingsSidebarNav, sectionMeta } from "./SettingsNavigation";
import {
  closeSettings,
  openSettingsSection,
  settingsSectionForLocation,
} from "./settingsModalNavigation";

export function SettingsModal({ daemonEnabled }: { daemonEnabled: boolean }) {
  const { t } = useTranslation("settings");
  const requested = useUiStore((s) => s.settingsSection);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const pathname = useRouterState({ select: (state) => state.location.pathname });

  // Leaving the page (Open logs, Resume setup, a reset) leaves Settings too.
  // A cold load's step from its `/settings/<slug>` route to the page under
  // the modal is not leaving it.
  const pathnameRef = React.useRef(pathname);
  React.useEffect(() => {
    const previous = pathnameRef.current;
    pathnameRef.current = pathname;
    if (previous === pathname || settingsSectionForLocation(previous, null) !== null) return;
    closeSettings();
  }, [pathname]);

  // Turning developer tools off while on a developer-only category falls
  // back to the default one rather than keeping the hidden panel up.
  const section: SettingsSectionId | null =
    requested && !developerToolsEnabled && isDevOnlySettingsSection(requested)
      ? DEFAULT_SETTINGS_SECTION_ID
      : requested;
  const open = section !== null;

  return (
    <DialogPrimitive.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) closeSettings();
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-x-0 top-[var(--kb-window-top-inset)] bottom-0 z-50 bg-background/60 backdrop-blur-xs duration-200 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0 dark:bg-background/64" />
        {/* The viewport centres the surface and lets presses on its margin
            reach the backdrop. */}
        <div className="pointer-events-none fixed inset-x-0 top-[var(--kb-window-top-inset)] bottom-0 z-50 flex items-center justify-center sm:p-6 lg:p-10">
          <DialogPrimitive.Content
            aria-describedby={undefined}
            data-app-modal=""
            onOpenAutoFocus={(event) => {
              // Focus the surface, not its first control: no stray ring.
              event.preventDefault();
              (event.currentTarget as HTMLElement | null)?.focus();
            }}
            className="pointer-events-auto relative flex size-full min-h-0 min-w-0 flex-col overflow-hidden bg-background text-foreground outline-none duration-150 ease-out data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-98 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-98 sm:max-h-[60rem] sm:max-w-[80rem] sm:rounded-2xl sm:border sm:shadow-2xl/40"
          >
            <DialogPrimitive.Title className="sr-only">{t("page.title")}</DialogPrimitive.Title>
            {section ? (
              <SettingsModalBody section={section} daemonEnabled={daemonEnabled} />
            ) : null}
          </DialogPrimitive.Content>
        </div>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function SettingsModalBody({
  section,
  daemonEnabled,
}: {
  section: SettingsSectionId;
  daemonEnabled: boolean;
}) {
  const { t } = useTranslation("settings");
  const scrollRef = React.useRef<HTMLDivElement>(null);
  // The per-layer backend counts the rail shows. Same key the settings panels
  // use, so react-query serves it from cache instead of a second request.
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
    undefined,
    { enabled: daemonEnabled, staleTime: 15_000 },
  );
  const counts = React.useMemo<Partial<Record<SettingsSectionId, number>>>(() => {
    const backends = (backendSettingsQuery.data?.data?.backends ?? []).map(
      backendRowToSettingsBackend,
    );
    return {
      "network-bitcoin": backendsForLayer(backends, "bitcoin").length,
      "network-lightning": backendsForLayer(backends, "lightning").length,
      "network-liquid": backendsForLayer(backends, "liquid").length,
      "network-market": backends.filter((backend) => backend.net === "FX").length,
    };
  }, [backendSettingsQuery.data]);

  // Each category starts at its top.
  React.useLayoutEffect(() => {
    scrollRef.current?.scrollTo?.({ top: 0 });
  }, [section]);

  return (
    <div className="relative flex min-h-0 flex-1">
      <SettingsSidebarNav
        activeId={section}
        counts={counts}
        onSelect={(id) => void openSettingsSection(id)}
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex h-12 shrink-0 items-center gap-3 border-b ps-5 pe-2">
          <nav
            aria-label={t("page.title")}
            className="flex min-w-0 items-center gap-1.5 text-sm"
          >
            <span className="shrink-0 text-muted-foreground">{t("page.title")}</span>
            <ChevronRight
              className="size-3.5 shrink-0 text-muted-foreground/70"
              aria-hidden="true"
            />
            <span className="truncate font-medium text-foreground" aria-current="page">
              {/* dynamic key */}
              {t(sectionMeta(section).labelKey as never)}
            </span>
          </nav>
          <div className="ms-auto flex shrink-0 items-center gap-1">
            <DialogPrimitive.Close asChild>
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                aria-label={t("page.close")}
                title={`${t("page.close")} (Esc)`}
              >
                <X aria-hidden="true" />
              </Button>
            </DialogPrimitive.Close>
          </div>
        </header>
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          <SettingsScreen sectionId={section} />
        </div>
      </div>
    </div>
  );
}
