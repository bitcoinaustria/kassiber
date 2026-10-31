import { create } from "zustand";

import { useUiStore } from "@/store/ui";

/**
 * The right panel's tabs, after T3 Code's right panel: the assistant, the
 * panel's own surfaces (the review queue, recent changes, the log tail) and
 * any open transaction details are tabs of one panel beside the page.
 *
 * Whether the panel is open lives in the UI store (`rightPanelOpen`), and so
 * does the assistant tab (`assistantPanelOpen`). A detail registers itself
 * here while open and renders into its own slot through a portal, so whoever
 * owns its state keeps it. A page owns at most one detail, its preview: the
 * next transaction the page opens takes the same tab. A detail the panel
 * owns (opened from one of its surfaces, or a pinned preview) keeps its tab
 * until it is closed, so several transactions can stay open side by side.
 */
export type RightPanelSurface = "review" | "changes" | "logs";
export type RightPanelDetailTab = `detail:${number}`;
export type RightPanelTab = "assistant" | RightPanelSurface | RightPanelDetailTab;

export const detailTab = (key: number): RightPanelDetailTab => `detail:${key}`;

export interface RightPanelDetailInput {
  label: string;
  /** Closes the detail through its owner (its `onOpenChange(false)`). */
  close: () => void;
  /** The transaction on show, which pinning hands to the panel. */
  transactionId: string | null;
  /** The panel's own transaction entry, or null for a page's preview. */
  owner: number | null;
}

export interface RightPanelDetail extends RightPanelDetailInput {
  /** Changes whenever a different detail takes a tab. */
  key: number;
  /** The tab that was in front when this one opened, to return to. */
  returnTo: RightPanelTab;
}

/** A transaction the panel keeps open itself, by id. */
export interface PanelTransaction {
  id: number;
  transactionId: string;
  initialTab: string;
}

interface RightPanelState {
  /** Mounted panel hosts; a detail with none falls back to its own sheet. */
  hosts: number;
  activeTab: RightPanelTab;
  details: RightPanelDetail[];
  detailSlots: Record<number, HTMLElement>;
  /**
   * The detail that opened the panel, while nothing else has been brought
   * forward since: closing it closes the panel again rather than leaving
   * the launcher behind.
   */
  openedFor: number | null;
  surfaces: RightPanelSurface[];
  transactions: PanelTransaction[];
  /**
   * The width the docked panel takes from the page, or null while it is not
   * docked beside it (hidden, a sheet, or maximized over the page).
   */
  dockedWidth: number | null;
  setDockedWidth: (width: number | null) => void;
  registerHost: () => () => void;
  setActiveTab: (tab: RightPanelTab) => void;
  showDetail: (detail: RightPanelDetailInput) => number;
  updateDetail: (key: number, detail: RightPanelDetailInput) => void;
  hideDetail: (key: number) => void;
  setDetailSlot: (key: number, slot: HTMLElement | null) => void;
  openSurface: (surface: RightPanelSurface) => void;
  closeSurface: (surface: RightPanelSurface) => void;
  /** Opens a transaction in a tab of its own, or brings its tab forward. */
  openTransaction: (transactionId: string, initialTab?: string) => void;
  closeTransaction: (id: number) => void;
  /** Follows a coin inside a kept tab: the same tab shows the next transaction. */
  retargetTransaction: (id: number, transactionId: string) => void;
  /** Keeps a page's preview open in its own tab. */
  pinDetail: (key: number) => void;
}

let nextDetailKey = 1;
let nextTransactionId = 1;

function withoutSlot(slots: Record<number, HTMLElement>, key: number) {
  const next = { ...slots };
  delete next[key];
  return next;
}

function revealPanel() {
  const ui = useUiStore.getState();
  if (!ui.rightPanelOpen) ui.setRightPanelOpen(true);
}

export const useRightPanelStore = create<RightPanelState>((set, get) => ({
  hosts: 0,
  activeTab: "assistant",
  details: [],
  detailSlots: {},
  openedFor: null,
  surfaces: [],
  transactions: [],
  dockedWidth: null,
  setDockedWidth: (dockedWidth) => set({ dockedWidth }),
  registerHost: () => {
    set((state) => ({ hosts: state.hosts + 1 }));
    return () => set((state) => ({ hosts: Math.max(0, state.hosts - 1) }));
  },
  setActiveTab: (activeTab) =>
    set((state) => ({
      activeTab,
      openedFor:
        state.openedFor !== null && activeTab === detailTab(state.openedFor)
          ? state.openedFor
          : null,
    })),
  showDetail: (detail) => {
    const key = nextDetailKey++;
    const ui = useUiStore.getState();
    const opensPanel = !ui.rightPanelOpen;
    if (opensPanel) ui.setRightPanelOpen(true);
    set((state) => ({
      details: [...state.details, { ...detail, key, returnTo: state.activeTab }],
      activeTab: detailTab(key),
      openedFor: opensPanel ? key : null,
    }));
    return key;
  },
  updateDetail: (key, detail) =>
    set((state) => ({
      details: state.details.map((entry) =>
        entry.key === key ? { ...entry, ...detail, key, returnTo: entry.returnTo } : entry,
      ),
    })),
  hideDetail: (key) => {
    const state = get();
    const index = state.details.findIndex((entry) => entry.key === key);
    if (index < 0) return;
    const closing = state.details[index];
    if (state.openedFor === key) useUiStore.getState().setRightPanelOpen(false);
    const details = state.details.filter((entry) => entry.key !== key);
    const detailSlots = withoutSlot(state.detailSlots, key);
    let activeTab = state.activeTab;
    if (activeTab === detailTab(key)) {
      const back = closing.returnTo;
      const backIsDetail = back.startsWith("detail:");
      activeTab =
        !backIsDetail || details.some((entry) => detailTab(entry.key) === back)
          ? back
          : details.length
            ? detailTab(details[Math.min(index, details.length - 1)].key)
            : "assistant";
    }
    set({ details, detailSlots, openedFor: state.openedFor === key ? null : state.openedFor, activeTab });
  },
  setDetailSlot: (key, slot) =>
    set((state) => {
      if ((state.detailSlots[key] ?? null) === slot) return state;
      if (slot) return { detailSlots: { ...state.detailSlots, [key]: slot } };
      return { detailSlots: withoutSlot(state.detailSlots, key) };
    }),
  openSurface: (surface) => {
    revealPanel();
    set((state) => ({
      surfaces: state.surfaces.includes(surface) ? state.surfaces : [...state.surfaces, surface],
      activeTab: surface,
      openedFor: null,
    }));
  },
  closeSurface: (surface) =>
    set((state) => {
      const index = state.surfaces.indexOf(surface);
      if (index < 0) return state;
      const surfaces = state.surfaces.filter((entry) => entry !== surface);
      // Closing the surface in front brings its neighbour forward, or the
      // assistant; the panel falls back from there if that tab is gone too.
      const activeTab =
        state.activeTab === surface
          ? (surfaces[Math.min(index, surfaces.length - 1)] ?? "assistant")
          : state.activeTab;
      return { surfaces, activeTab };
    }),
  openTransaction: (transactionId, initialTab = "details") => {
    revealPanel();
    const state = get();
    const owned = state.transactions.find((entry) => entry.transactionId === transactionId);
    if (owned) {
      const detail = state.details.find((entry) => entry.owner === owned.id);
      if (detail) state.setActiveTab(detailTab(detail.key));
      return;
    }
    set({
      transactions: [
        ...state.transactions,
        { id: nextTransactionId++, transactionId, initialTab },
      ],
    });
  },
  closeTransaction: (id) =>
    set((state) => ({ transactions: state.transactions.filter((entry) => entry.id !== id) })),
  retargetTransaction: (id, transactionId) =>
    set((state) => ({
      transactions: state.transactions.map((entry) =>
        entry.id === id ? { ...entry, transactionId, initialTab: "details" } : entry,
      ),
    })),
  pinDetail: (key) => {
    const detail = get().details.find((entry) => entry.key === key);
    if (!detail || detail.owner !== null || !detail.transactionId) return;
    const transactionId = detail.transactionId;
    // Pinned, the transaction stays in view: letting go of the preview must
    // not close a panel that opened for it.
    set({ openedFor: null });
    // The page lets go of its preview; the panel opens the same transaction
    // in a tab of its own, where the page's next one no longer replaces it.
    detail.close();
    get().openTransaction(transactionId);
  },
}));
