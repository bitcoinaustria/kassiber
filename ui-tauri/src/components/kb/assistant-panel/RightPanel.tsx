/**
 * The right panel, after T3 Code's: docked beside the page, which keeps its
 * width minus the panel's, with a tab per open thing: the assistant, the
 * panel's own surfaces (the review queue, recent changes, the log tail) and
 * transaction details. `+` adds a tab, and a panel without tabs shows a
 * launcher whose rows answer to single keys. On narrow windows it is a sheet
 * over the page instead. The title-bar toggle shows and hides it; docked, it
 * can also take the whole page column.
 *
 * Every tab is resizable from the panel's left edge; details remember one
 * width, and the assistant and the surfaces another, in the UI store.
 */

import * as React from "react";
import { Dialog as DialogPrimitive } from "radix-ui";
import { useTranslation } from "react-i18next";
import {
  GitCompareArrows,
  ListChecks,
  Maximize2,
  Minimize2,
  Pin,
  Plus,
  Receipt,
  ScrollText,
  Sparkles,
  X,
  type LucideIcon,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/ui/kbd";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import {
  detailTab,
  useRightPanelStore,
  type RightPanelSurface,
  type RightPanelTab,
} from "@/store/rightPanel";
import { useUiStore } from "@/store/ui";

import { AssistantPanelActions, AssistantPanelBody } from "./AssistantPanel";
import {
  ASSISTANT_PANEL_BOUNDS,
  ASSISTANT_PANEL_SHORTCUT,
  DETAIL_PANEL_BOUNDS,
  RIGHT_PANEL_MIN_PAGE_WIDTH,
  RIGHT_PANEL_SHEET_MARGIN,
  clampPanelWidth,
  draggedPanelWidth,
  focusAssistantComposer,
  keyedPanelWidth,
  panelMaxWidth,
  type PanelWidthLimits,
} from "./assistantPanelLayout";
import { ChangesSurface } from "./surfaces/ChangesSurface";
import { LogsSurface } from "./surfaces/LogsSurface";
import { PanelTransactionDetail } from "./surfaces/PanelTransactionDetail";
import { ReviewSurface } from "./surfaces/ReviewSurface";

function clearDragStyles() {
  document.body.style.removeProperty("cursor");
  document.body.style.removeProperty("user-select");
}

const viewportWidth = () =>
  typeof window === "undefined" ? null : window.innerWidth || null;

function subscribeToResize(onChange: () => void) {
  window.addEventListener("resize", onChange);
  return () => window.removeEventListener("resize", onChange);
}

/** The window width, so the sheet's separator reports the width on screen. */
function useViewportWidth(): number | null {
  return React.useSyncExternalStore(subscribeToResize, viewportWidth, () => null);
}

/** The sheet's width: the remembered one, never covering the page's strip. */
const SHEET_WIDTH_CLASS =
  "w-[min(var(--right-panel-width),calc(100vw-var(--right-panel-margin)))] min-w-[min(var(--right-panel-min-width),100vw)]";

/**
 * Layers the launcher's single-key shortcuts must not reach under: another
 * dialog (not the panel's own sheet), a menu, a listbox.
 */
const SHORTCUT_BLOCKING_LAYERS =
  '[role="dialog"]:not([data-right-panel]), [role="alertdialog"], [role="menu"], [role="listbox"]';

/**
 * The launcher takes focus when it appears in place of the panel's tabs or
 * from the title-bar toggle, not from wherever the user is working.
 */
function focusLauncherOnMount(node: HTMLDivElement | null) {
  if (!node) return;
  const active = document.activeElement;
  if (
    !active ||
    active === document.body ||
    active.matches("[data-right-panel-toggle]") ||
    active.closest("[data-right-panel]")
  ) {
    node.focus();
  }
}

function targetsTypingContext(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])') !==
      null
  );
}

interface TabSpec {
  id: RightPanelTab;
  label: string;
  closeLabel: string;
  icon: LucideIcon;
  onClose: () => void;
  /** A page's preview: the next transaction it opens takes this tab. */
  preview?: boolean;
  /** Keeps a preview open in a tab of its own. */
  onPin?: () => void;
}

/** Something the panel can show, offered by `+` and the launcher. */
interface Surface {
  id: "assistant" | RightPanelSurface | "details";
  label: string;
  icon: LucideIcon;
  /** A single key, without modifiers. */
  shortcut: string;
  available: boolean;
  /** Why it is unavailable. */
  hint: string;
  open: () => void;
}

const SURFACE_ICONS: Record<RightPanelSurface, LucideIcon> = {
  review: ListChecks,
  changes: GitCompareArrows,
  logs: ScrollText,
};

/** The panel's own surfaces that only developer tools unlock, like their pages. */
const DEVELOPER_SURFACES: ReadonlySet<RightPanelSurface> = new Set(["changes", "logs"]);

/** A tab id as a DOM id fragment. */
const tabDomId = (tab: RightPanelTab) => tab.replace(":", "-");

function surfaceForKey(surfaces: readonly Surface[], event: KeyboardEvent | React.KeyboardEvent) {
  if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return null;
  if ("isComposing" in event && event.isComposing) return null;
  return (
    surfaces.find(
      (surface) => surface.available && surface.shortcut.toLowerCase() === event.key.toLowerCase(),
    ) ?? null
  );
}

export function RightPanel({
  open,
  assistantTab,
  assistantAvailable,
  sheet,
}: {
  /** The panel is on screen. */
  open: boolean;
  /** The assistant is one of its tabs. */
  assistantTab: boolean;
  /** The assistant can be added (not on the full assistant page). */
  assistantAvailable: boolean;
  /** A sheet over the page (narrow window) rather than docked beside it. */
  sheet: boolean;
}) {
  const { t } = useTranslation("assistant");
  const registerHost = useRightPanelStore((s) => s.registerHost);
  const details = useRightPanelStore((s) => s.details);
  const openSurfaces = useRightPanelStore((s) => s.surfaces);
  const transactions = useRightPanelStore((s) => s.transactions);
  const storedTab = useRightPanelStore((s) => s.activeTab);
  const setActiveTab = useRightPanelStore((s) => s.setActiveTab);
  const openSurface = useRightPanelStore((s) => s.openSurface);
  const closeSurface = useRightPanelStore((s) => s.closeSurface);
  const pinDetail = useRightPanelStore((s) => s.pinDetail);
  const panelOpen = useUiStore((s) => s.rightPanelOpen);
  const setPanelOpen = useUiStore((s) => s.setRightPanelOpen);
  const setAssistantOpen = useUiStore((s) => s.setAssistantPanelOpen);
  const openAssistantPanel = useUiStore((s) => s.openAssistantPanel);
  const revealRequest = useUiStore((s) => s.assistantPanelRevealRequest);
  const developerTools = useUiStore((s) => s.developerToolsEnabled);

  React.useLayoutEffect(() => registerHost(), [registerHost]);

  // Asking for the assistant (shortcut, launcher, "Ask about this") brings
  // its tab to the front even while a detail is open.
  const seenRevealRef = React.useRef(revealRequest);
  React.useEffect(() => {
    if (revealRequest === seenRevealRef.current) return;
    seenRevealRef.current = revealRequest;
    setActiveTab("assistant");
  }, [revealRequest, setActiveTab]);

  // A detail lives only while it is in view: hiding the panel closes it,
  // where the assistant's and the surfaces' tabs stay for the next time.
  React.useEffect(() => {
    if (panelOpen) return;
    for (const detail of details) detail.close();
  }, [panelOpen, details]);

  // Turning developer tools off takes their surfaces with it, as it locks
  // their pages.
  const surfaceAvailable = (surface: RightPanelSurface) =>
    developerTools || !DEVELOPER_SURFACES.has(surface);
  React.useEffect(() => {
    if (developerTools) return;
    for (const surface of openSurfaces) {
      if (DEVELOPER_SURFACES.has(surface)) closeSurface(surface);
    }
  }, [developerTools, openSurfaces, closeSurface]);

  const surfaceLabel = (surface: RightPanelSurface) =>
    surface === "review"
      ? t("panel.surfaceReview")
      : surface === "changes"
        ? t("panel.surfaceChanges")
        : t("panel.surfaceLogs");

  const tabs: TabSpec[] = [];
  if (assistantTab) {
    tabs.push({
      id: "assistant",
      label: t("panel.title"),
      closeLabel: t("panel.close"),
      icon: Sparkles,
      onClose: () => setAssistantOpen(false),
    });
  }
  for (const surface of openSurfaces) {
    if (!surfaceAvailable(surface)) continue;
    const label = surfaceLabel(surface);
    tabs.push({
      id: surface,
      label,
      closeLabel: t("panel.closeTab", { label }),
      icon: SURFACE_ICONS[surface],
      onClose: () => closeSurface(surface),
    });
  }
  for (const detail of details) {
    const preview = detail.owner === null;
    tabs.push({
      id: detailTab(detail.key),
      label: detail.label,
      closeLabel: t("panel.closeTab", { label: detail.label }),
      icon: Receipt,
      onClose: detail.close,
      preview,
      onPin: preview && detail.transactionId ? () => pinDetail(detail.key) : undefined,
    });
  }
  const activeTab: RightPanelTab | null = tabs.some((tab) => tab.id === storedTab)
    ? storedTab
    : (tabs[0]?.id ?? null);

  const panelSurface = (surface: RightPanelSurface, shortcut: string): Surface => ({
    id: surface,
    label: surfaceLabel(surface),
    icon: SURFACE_ICONS[surface],
    shortcut,
    available: surfaceAvailable(surface),
    hint: t("panel.developerOnly"),
    open: () => openSurface(surface),
  });
  const latestDetail = details.at(-1);
  const surfaces: Surface[] = [
    {
      id: "assistant",
      label: t("panel.title"),
      icon: Sparkles,
      shortcut: "A",
      available: assistantAvailable,
      hint: t("panel.assistantUnavailable"),
      open: () => openAssistantPanel({ focusComposer: true }),
    },
    panelSurface("review", "Q"),
    panelSurface("changes", "H"),
    panelSurface("logs", "L"),
    {
      id: "details",
      label: t("panel.surfaceDetails"),
      icon: Receipt,
      shortcut: "D",
      available: latestDetail !== undefined,
      hint: t("panel.detailsHint"),
      open: () => {
        if (latestDetail) setActiveTab(detailTab(latestDetail.key));
      },
    },
  ];

  const hidePanel = () => setPanelOpen(false);
  const body = (
    <RightPanelBody
      sheet={sheet}
      tabs={tabs}
      activeTab={activeTab}
      assistantTab={assistantTab}
      openSurfaces={openSurfaces.filter(surfaceAvailable)}
      surfaces={surfaces}
    />
  );
  // The panel's own transactions render through their tab's slot, wherever
  // the panel is; they mount here so they outlive switching tabs.
  const kept = transactions.map((entry) => (
    <PanelTransactionDetail key={entry.id} entry={entry} />
  ));

  if (sheet) {
    return (
      <>
        <RightPanelSheet
          open={open}
          activeTab={activeTab}
          label={tabs.find((tab) => tab.id === activeTab)?.label}
          onClose={hidePanel}
        >
          {body}
        </RightPanelSheet>
        {kept}
      </>
    );
  }
  return (
    <>
      {open ? (
        <DockedPanel label={tabs.find((tab) => tab.id === activeTab)?.label} activeTab={activeTab}>
          {body}
        </DockedPanel>
      ) : null}
      {kept}
    </>
  );
}

/** Beside the page: a plain region, no backdrop, the page stays usable. */
function DockedPanel({
  label,
  activeTab,
  children,
}: {
  label: string | undefined;
  activeTab: RightPanelTab | null;
  children: React.ReactNode;
}) {
  const { t } = useTranslation("assistant");
  const maximized = useUiStore((s) => s.rightPanelMaximized);
  const asideRef = React.useRef<HTMLElement>(null);
  const resize = usePanelResize({
    activeTab,
    reserve: RIGHT_PANEL_MIN_PAGE_WIDTH,
    containerWidth: () => asideRef.current?.parentElement?.clientWidth || null,
    measure: () => asideRef.current?.getBoundingClientRect().width ?? 0,
    enabled: !maximized,
  });

  // The shell folds its nav sooner while the panel takes this much from the
  // page, so the page keeps the room it would have had without it.
  const setDockedWidth = useRightPanelStore((s) => s.setDockedWidth);
  const dockedWidth = maximized ? null : resize.width;
  React.useEffect(() => {
    setDockedWidth(dockedWidth);
  }, [dockedWidth, setDockedWidth]);
  React.useEffect(() => () => setDockedWidth(null), [setDockedWidth]);

  // CSS caps the panel as the window narrows; render again when the row
  // changes size so the separator reports the width that is on screen.
  const [, setRowWidth] = React.useState<number | null>(null);
  React.useEffect(() => {
    const row = asideRef.current?.parentElement;
    if (!row || typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(() => setRowWidth(row.clientWidth || null));
    observer.observe(row);
    return () => observer.disconnect();
  }, []);

  // Hiding the panel with focus inside hands focus to the title-bar toggle,
  // after T3 Code, rather than dropping it on the body.
  React.useLayoutEffect(() => {
    const aside = asideRef.current;
    return () => {
      if (!aside?.contains(document.activeElement)) return;
      queueMicrotask(() => {
        if (document.activeElement && document.activeElement !== document.body) return;
        document.querySelector<HTMLElement>("[data-right-panel-toggle]")?.focus();
      });
    };
  }, []);

  return (
    <aside
      ref={asideRef}
      aria-label={label ? `${t("panel.sideLabel")}: ${label}` : t("panel.sideLabel")}
      data-right-panel="docked"
      data-active-tab={activeTab ?? undefined}
      data-maximized={maximized ? "" : undefined}
      style={
        maximized
          ? undefined
          : {
              width: resize.width,
              minWidth: resize.bounds.min,
              // A width remembered on a wider window must not squeeze the
              // page below its minimum on this one.
              maxWidth: `max(${resize.bounds.min}px, calc(100% - ${RIGHT_PANEL_MIN_PAGE_WIDTH}px))`,
            }
      }
      className={cn(
        "relative flex h-full min-h-0 min-w-0 flex-col bg-background",
        maximized ? "flex-1" : "shrink-0 border-l border-border",
      )}
    >
      {maximized ? null : resize.handle}
      {children}
    </aside>
  );
}

/** On narrow windows: T3 Code's sheet over the page, with a backdrop. */
function RightPanelSheet({
  open,
  activeTab,
  label,
  onClose,
  children,
}: {
  open: boolean;
  activeTab: RightPanelTab | null;
  label: string | undefined;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const { t } = useTranslation("assistant");
  const contentRef = React.useRef<HTMLDivElement>(null);
  useViewportWidth();
  const resize = usePanelResize({
    activeTab,
    reserve: RIGHT_PANEL_SHEET_MARGIN,
    containerWidth: viewportWidth,
    measure: () => contentRef.current?.getBoundingClientRect().width ?? 0,
    enabled: open,
  });

  // No trigger opens the sheet (a shortcut, a row, a launcher can), so
  // closing hands focus back to wherever it was when the sheet opened.
  // Read in a layout effect, before the dialog moves focus inside.
  const openerRef = React.useRef<HTMLElement | null>(null);
  React.useLayoutEffect(() => {
    if (!open) return;
    const active = document.activeElement;
    openerRef.current =
      active instanceof HTMLElement && active !== document.body ? active : null;
  }, [open]);

  return (
    <DialogPrimitive.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay
          data-slot="right-panel-backdrop"
          className="fixed inset-x-0 top-[var(--kb-window-top-inset)] bottom-0 z-50 bg-background/60 backdrop-blur-xs duration-200 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0"
        />
        <DialogPrimitive.Content
          ref={contentRef}
          aria-describedby={undefined}
          data-right-panel="sheet"
          data-active-tab={activeTab ?? undefined}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const opener = openerRef.current;
            openerRef.current = null;
            if (opener?.isConnected) opener.focus();
          }}
          onOpenAutoFocus={(event) => {
            if (activeTab !== "assistant") return;
            event.preventDefault();
            if (!focusAssistantComposer(contentRef.current)) {
              contentRef.current?.focus();
            }
          }}
          style={
            {
              "--right-panel-width": `${resize.width}px`,
              "--right-panel-min-width": `${resize.bounds.min}px`,
              "--right-panel-margin": `${RIGHT_PANEL_SHEET_MARGIN}px`,
            } as React.CSSProperties
          }
          className={cn(
            // T3 Code's sheet surface: popover fill, a start border, a soft
            // shadow and a 1px top hairline, sliding in 8px as it fades.
            "fixed top-[var(--kb-window-top-inset)] right-0 bottom-0 z-50 flex min-h-0 max-w-full flex-col border-l bg-popover text-popover-foreground shadow-lg/5 outline-none",
            "before:pointer-events-none before:absolute before:inset-0 before:shadow-[0_1px_rgb(0_0_0/4%)] dark:before:shadow-[0_-1px_rgb(255_255_255/6%)]",
            "duration-200 ease-in-out data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:slide-out-to-right-8 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:slide-in-from-right-8",
            SHEET_WIDTH_CLASS,
          )}
        >
          <DialogPrimitive.Title className="sr-only">
            {label ? `${t("panel.sideLabel")}: ${label}` : t("panel.sideLabel")}
          </DialogPrimitive.Title>
          {resize.handle}
          {children}
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

/**
 * The left-edge separator and the width it sets: the details' remembered
 * width while they are in front, the assistant's otherwise.
 */
function usePanelResize({
  activeTab,
  reserve,
  containerWidth,
  measure,
  enabled,
}: {
  activeTab: RightPanelTab | null;
  reserve: number;
  containerWidth: () => number | null;
  measure: () => number;
  enabled: boolean;
}) {
  const { t } = useTranslation("assistant");
  const details = activeTab?.startsWith("detail:") ?? false;
  const bounds = details ? DETAIL_PANEL_BOUNDS : ASSISTANT_PANEL_BOUNDS;
  const assistantWidth = useUiStore((s) => s.assistantPanelWidth);
  const detailWidth = useUiStore((s) => s.detailPanelWidth);
  const setAssistantWidth = useUiStore((s) => s.setAssistantPanelWidth);
  const setDetailWidth = useUiStore((s) => s.setDetailPanelWidth);
  const storedWidth = details ? detailWidth : assistantWidth;
  const setStoredWidth = details ? setDetailWidth : setAssistantWidth;
  const [dragWidth, setDragWidth] = React.useState<number | null>(null);
  const dragRef = React.useRef<{ startX: number; startWidth: number } | null>(null);
  const width = dragWidth ?? storedWidth;
  const limits = (): PanelWidthLimits => ({ bounds, containerWidth: containerWidth(), reserve });

  // CSS keeps the page its minimum, so a width remembered on a wider window
  // can render narrower than stored. Resizing starts from what is on screen,
  // or every step until the stored width caught up would change nothing.
  const renderedWidth = () => {
    const measured = measure();
    return clampPanelWidth(measured > 0 ? measured : width, limits());
  };

  const finishDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    clearDragStyles();
    // A click on the handle is not a resize; keep the remembered width.
    if (dragWidth !== null && dragWidth !== drag.startWidth) setStoredWidth(dragWidth);
    setDragWidth(null);
  };

  // A drag can end without a pointer-up: the panel closes, maximizes, or
  // another tab comes forward, and the handle goes with it. Drop that drag
  // rather than leaving the page stuck with the resize cursor.
  React.useEffect(() => {
    if (!dragRef.current) return;
    dragRef.current = null;
    clearDragStyles();
    setDragWidth(null);
  }, [enabled, details]);
  React.useEffect(
    () => () => {
      if (dragRef.current) {
        dragRef.current = null;
        clearDragStyles();
      }
    },
    [],
  );

  const current = limits();
  const handle = (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("panel.resize")}
      aria-valuemin={bounds.min}
      aria-valuemax={panelMaxWidth(current)}
      aria-valuenow={clampPanelWidth(width, current)}
      tabIndex={0}
      className={cn(
        "group absolute inset-y-0 -left-1.5 z-20 w-3 cursor-col-resize touch-none outline-none",
        "after:absolute after:inset-y-0 after:left-1/2 after:w-px after:-translate-x-1/2 after:transition-colors",
        "hover:after:bg-primary/60 focus-visible:after:bg-primary",
        dragWidth !== null && "after:bg-primary",
      )}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture?.(event.pointerId);
        const startWidth = renderedWidth();
        dragRef.current = { startX: event.clientX, startWidth };
        document.body.style.cursor = "col-resize";
        document.body.style.userSelect = "none";
        setDragWidth(startWidth);
      }}
      onPointerMove={(event) => {
        const drag = dragRef.current;
        if (!drag) return;
        setDragWidth(draggedPanelWidth({ ...drag, clientX: event.clientX }, limits()));
      }}
      onPointerUp={finishDrag}
      onPointerCancel={finishDrag}
      onDoubleClick={() => {
        const next = keyedPanelWidth("Enter", renderedWidth(), limits());
        if (next !== null) setStoredWidth(next);
      }}
      onKeyDown={(event) => {
        const next = keyedPanelWidth(event.key, renderedWidth(), limits());
        if (next === null) return;
        event.preventDefault();
        setStoredWidth(next);
      }}
    />
  );
  return { width, bounds, handle };
}

function RightPanelBody({
  sheet,
  tabs,
  activeTab,
  assistantTab,
  openSurfaces,
  surfaces,
}: {
  sheet: boolean;
  tabs: TabSpec[];
  activeTab: RightPanelTab | null;
  assistantTab: boolean;
  openSurfaces: RightPanelSurface[];
  surfaces: Surface[];
}) {
  const { t } = useTranslation("assistant");
  const setActiveTab = useRightPanelStore((s) => s.setActiveTab);
  const details = useRightPanelStore((s) => s.details);
  const maximized = useUiStore((s) => s.rightPanelMaximized);
  const setMaximized = useUiStore((s) => s.setRightPanelMaximized);
  const assistantInFront = activeTab === "assistant";
  const front = tabs.find((tab) => tab.id === activeTab);
  const maximizeLabel = maximized ? t("panel.restore") : t("panel.maximize");

  return (
    <>
      <div
        className="flex h-10 shrink-0 items-center gap-1 border-b border-border/60 pr-2 pl-2"
        data-right-panel-tabbar=""
      >
        <div className="flex min-w-0 flex-1 items-center gap-1">
          {tabs.length > 0 ? (
            <div
              role="tablist"
              aria-label={t("panel.tabs")}
              className="flex min-w-0 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
            >
              {tabs.map((tab) => (
                <PanelTab
                  key={tab.id}
                  tab={tab}
                  tabs={tabs}
                  active={tab.id === activeTab}
                  onActivate={setActiveTab}
                />
              ))}
            </div>
          ) : null}
          <AddSurfaceMenu surfaces={surfaces} />
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          {assistantInFront ? <AssistantPanelActions /> : null}
          {front?.onPin ? (
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              aria-label={t("panel.pin")}
              title={t("panel.pinHint")}
              onClick={front.onPin}
            >
              <Pin className="size-3.5" aria-hidden="true" />
            </Button>
          ) : null}
          {sheet ? (
            <DialogPrimitive.Close asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                aria-label={t("panel.hide")}
                title={`${t("panel.hide")} (Esc)`}
              >
                <X className="size-3.5" aria-hidden="true" />
              </Button>
            </DialogPrimitive.Close>
          ) : (
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              aria-label={maximizeLabel}
              aria-pressed={maximized}
              title={maximizeLabel}
              onClick={() => setMaximized(!maximized)}
            >
              {maximized ? (
                <Minimize2 className="size-3.5" aria-hidden="true" />
              ) : (
                <Maximize2 className="size-3.5" aria-hidden="true" />
              )}
            </Button>
          )}
        </div>
      </div>
      {assistantTab ? (
        <div
          role="tabpanel"
          id="right-panel-assistant"
          aria-labelledby="right-panel-tab-assistant"
          hidden={!assistantInFront}
          className="flex min-h-0 flex-1 flex-col"
        >
          <AssistantPanelBody inFront={assistantInFront} />
        </div>
      ) : null}
      {/* Surfaces stay mounted behind other tabs, keeping their filters
          and scroll. */}
      {openSurfaces.map((surface) => (
        <div
          key={surface}
          role="tabpanel"
          id={`right-panel-${surface}`}
          aria-labelledby={`right-panel-tab-${surface}`}
          hidden={activeTab !== surface}
          className="flex min-h-0 flex-1 flex-col"
          data-right-panel-surface={surface}
        >
          {surface === "review" ? (
            <ReviewSurface />
          ) : surface === "changes" ? (
            <ChangesSurface />
          ) : (
            <LogsSurface />
          )}
        </div>
      ))}
      {details.map((detail) => (
        <DetailSlot key={detail.key} detailKey={detail.key} active={activeTab === detailTab(detail.key)} />
      ))}
      {tabs.length === 0 ? <RightPanelLauncher surfaces={surfaces} /> : null}
    </>
  );
}

/**
 * Where one detail renders through its portal. The ref is stable, so the
 * portal target survives every render and switching tabs.
 */
function DetailSlot({ detailKey, active }: { detailKey: number; active: boolean }) {
  const setDetailSlot = useRightPanelStore((s) => s.setDetailSlot);
  const ref = React.useCallback(
    (node: HTMLDivElement | null) => setDetailSlot(detailKey, node),
    [detailKey, setDetailSlot],
  );
  const id = tabDomId(detailTab(detailKey));
  return (
    <div
      ref={ref}
      role="tabpanel"
      id={`right-panel-${id}`}
      aria-labelledby={`right-panel-tab-${id}`}
      hidden={!active}
      className="flex min-h-0 flex-1 flex-col"
    />
  );
}

/**
 * T3 Code's panel tab: a compact chip, filled while in front. Its icon turns
 * into the close mark on hover; a middle click or Delete closes it too. A
 * page's preview reads in italics, and a double click keeps it open.
 */
function PanelTab({
  tab,
  tabs,
  active,
  onActivate,
}: {
  tab: TabSpec;
  tabs: TabSpec[];
  active: boolean;
  onActivate: (tab: RightPanelTab) => void;
}) {
  const { t } = useTranslation("assistant");
  const previewHint = t("panel.previewHint");
  const Icon = tab.icon;
  const closeTitle =
    tab.id === "assistant"
      ? `${tab.closeLabel} (${formatShortcut([...ASSISTANT_PANEL_SHORTCUT])})`
      : tab.closeLabel;
  return (
    <div
      className={cn(
        "group/tab flex h-6 max-w-44 shrink-0 items-center gap-0.5 rounded-md pr-2 pl-1 text-xs",
        active
          ? "bg-accent text-foreground"
          : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
      )}
      onMouseDown={(event) => {
        // Linux pastes the primary selection on a middle click.
        if (event.button === 1) event.preventDefault();
      }}
      onAuxClick={(event) => {
        if (event.button !== 1) return;
        event.preventDefault();
        tab.onClose();
      }}
    >
      <button
        type="button"
        tabIndex={-1}
        aria-label={tab.closeLabel}
        title={closeTitle}
        className="group/close relative flex size-4 shrink-0 items-center justify-center rounded-sm hover:bg-muted"
        onClick={tab.onClose}
      >
        <Icon
          className="size-3 group-hover/tab:hidden group-focus-visible/close:hidden"
          aria-hidden="true"
        />
        <X
          className="hidden size-3 group-hover/tab:block group-focus-visible/close:block"
          aria-hidden="true"
        />
      </button>
      <button
        type="button"
        role="tab"
        id={`right-panel-tab-${tabDomId(tab.id)}`}
        aria-selected={active}
        aria-controls={`right-panel-${tabDomId(tab.id)}`}
        aria-keyshortcuts="Delete"
        tabIndex={active ? 0 : -1}
        className="flex h-full min-w-0 items-center rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        title={tab.preview ? `${tab.label} · ${previewHint}` : undefined}
        onClick={() => onActivate(tab.id)}
        onDoubleClick={tab.onPin}
        onKeyDown={(event) => {
          if (event.key === "Delete" || event.key === "Backspace") {
            event.preventDefault();
            tab.onClose();
            return;
          }
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const index = tabs.findIndex((other) => other.id === tab.id);
          const step = event.key === "ArrowRight" ? 1 : -1;
          const next = tabs[(index + step + tabs.length) % tabs.length];
          onActivate(next.id);
          document.getElementById(`right-panel-tab-${tabDomId(next.id)}`)?.focus();
        }}
      >
        <span className={cn("truncate", tab.preview && "italic")}>{tab.label}</span>
      </button>
    </div>
  );
}

/** `+` in the tab bar: the launcher's surfaces as a menu, same keys. */
function AddSurfaceMenu({ surfaces }: { surfaces: Surface[] }) {
  const { t } = useTranslation("assistant");
  const [open, setOpen] = React.useState(false);
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          className="shrink-0 text-muted-foreground"
          aria-label={t("panel.add")}
          title={t("panel.add")}
        >
          <Plus className="size-3.5" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        sideOffset={6}
        className="min-w-48"
        onKeyDownCapture={(event) => {
          const surface = surfaceForKey(surfaces, event);
          if (!surface) return;
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          surface.open();
        }}
      >
        {surfaces.map((surface) => {
          const Icon = surface.icon;
          return (
            <DropdownMenuItem
              key={surface.id}
              disabled={!surface.available}
              aria-keyshortcuts={surface.shortcut}
              onSelect={surface.open}
            >
              <Icon className="size-4" aria-hidden="true" />
              {surface.label}
              <DropdownMenuShortcut>{surface.shortcut}</DropdownMenuShortcut>
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The panel without tabs, after T3 Code's "Open a surface": each row opens
 * a surface, by click, arrow keys and Enter, or its key from anywhere while
 * no field, dialog or menu has the keyboard. A row that cannot open says why.
 */
function RightPanelLauncher({ surfaces }: { surfaces: Surface[] }) {
  const { t } = useTranslation("assistant");
  const [highlight, setHighlight] = React.useState(-1);
  const available = surfaces.filter((surface) => surface.available);
  const highlighted = highlight === -1 ? null : (available[highlight % available.length] ?? null);

  const surfacesRef = React.useRef(surfaces);
  React.useLayoutEffect(() => {
    surfacesRef.current = surfaces;
  });
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const surface = surfaceForKey(surfacesRef.current, event);
      if (!surface) return;
      if (document.querySelector(SHORTCUT_BLOCKING_LAYERS)) return;
      if (targetsTypingContext(event.composedPath()[0] ?? event.target)) return;
      event.preventDefault();
      event.stopPropagation();
      surface.open();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
    if (available.length === 0) return;
    const index = highlighted ? available.indexOf(highlighted) : -1;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") {
      event.preventDefault();
      setHighlight((index + 1) % available.length);
    } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
      event.preventDefault();
      setHighlight(index === -1 ? available.length - 1 : (index - 1 + available.length) % available.length);
    } else if (event.key === "Enter" && event.target === event.currentTarget && highlighted) {
      event.preventDefault();
      highlighted.open();
    }
  };

  return (
    <div
      ref={focusLauncherOnMount}
      tabIndex={0}
      role="group"
      aria-label={t("panel.launcherTitle")}
      data-right-panel-launcher=""
      onKeyDown={onKeyDown}
      className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto px-6 outline-none"
    >
      <TooltipProvider delayDuration={150}>
      <div className="w-full max-w-xs py-6">
        <h3 className="mb-3 text-center text-sm font-medium text-foreground">
          {t("panel.launcherTitle")}
        </h3>
        <div className="flex flex-col gap-0.5">
          {surfaces.map((surface) => {
            const Icon = surface.icon;
            if (surface.available) {
              return (
                <button
                  key={surface.id}
                  type="button"
                  aria-keyshortcuts={surface.shortcut}
                  onClick={surface.open}
                  onMouseEnter={() => setHighlight(available.indexOf(surface))}
                  onMouseLeave={() => setHighlight(-1)}
                  className={cn(
                    "flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-sm outline-none transition-colors hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring",
                    highlighted === surface && "bg-accent/60",
                  )}
                >
                  <Icon className="size-4 shrink-0" aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate">{surface.label}</span>
                  <Kbd>{surface.shortcut}</Kbd>
                </button>
              );
            }
            return (
              <Tooltip key={surface.id}>
                <TooltipTrigger asChild>
                  <div
                    tabIndex={0}
                    aria-disabled="true"
                    className="flex h-8 w-full cursor-default items-center gap-2.5 rounded-md px-2.5 text-left text-sm opacity-50 outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <Icon className="size-4 shrink-0" aria-hidden="true" />
                    <span className="min-w-0 flex-1 truncate">{surface.label}</span>
                    <span className="sr-only">{surface.hint}</span>
                    <Kbd>{surface.shortcut}</Kbd>
                  </div>
                </TooltipTrigger>
                <TooltipContent side="top">{surface.hint}</TooltipContent>
              </Tooltip>
            );
          })}
        </div>
      </div>
      </TooltipProvider>
    </div>
  );
}
