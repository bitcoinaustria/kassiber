import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { LanguageCode } from "@/i18n/config";
import { MAX_AUTO_SCALE } from "@/lib/appAutoScale";
import {
  DEFAULT_EXPLORER_SETTINGS,
  type ExplorerSettings,
} from "@/lib/explorer";
import type { AppUpdateCheck } from "@/lib/appUpdate";

// The supported language set lives in `@/i18n/config` so adding a language is
// a one-place change; the store type follows it.
type Lang = LanguageCode;
type Currency = "btc" | "eur";
export type DataMode = "mock" | "real" | "regtest";
export type ThemePreference = "system" | "light" | "dark";
export type NotificationTone = "info" | "success" | "warning" | "error";
export type BookChartPeriod =
  | "auto"
  | "30days"
  | "3months"
  | "6months"
  | "ytd"
  | "1year"
  | "5years"
  | "10years"
  | "15years"
  | "all";

export const DEFAULT_APP_SCALE = 0.9;
export const DEFAULT_THEME: ThemePreference = "dark";
export const MIN_APP_SCALE = 0.8;
export const MAX_APP_SCALE = 1.2;
export const APP_SCALE_STEP = 0.05;

export function isDaemonDataMode(dataMode: DataMode) {
  return dataMode === "real" || dataMode === "regtest";
}

export interface NotificationProgress {
  value?: number;
  indeterminate?: boolean;
  label?: string;
}

export interface AppNotification {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  dedupeKey?: string;
  progress?: NotificationProgress;
  createdAt: string;
  // Optional language-independent click target (an app route path). When set,
  // the header notification routes here instead of guessing from the (often
  // localized) title, so a translated title still routes correctly.
  target?: string;
}

export interface ActiveMaintenanceProgress {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  progress: NotificationProgress;
  details?: string[];
  state: "running" | "failed";
  phase?: string;
  startedAt: string;
  updatedAt: string;
}

export interface AppLockPolicy {
  autoLockWhenIdle: boolean;
  idleMinutes: number;
  requirePassphraseOnLaunch: boolean;
  lockOnWindowClose: boolean;
  touchIdUnlock: boolean;
}

export interface ImportedProjectIdentity {
  stateRoot: string;
  dataRoot: string;
  database: string;
}

export interface AiModelSelection {
  provider: string;
  model: string;
}

/**
 * Captured onboarding intent for the local books. Only `name`/`workspace`
 * are read by the running UI today (see AppHeader); the remaining fields are
 * captured in the webview so the upcoming native sidecar handoff can seed a
 * books/backend without re-prompting.
 *
 * `country` is the legacy identity field; new callers should prefer
 * `taxCountry`. Today only `at` and `generic` map to a real rp2 plugin
 * (see `kassiber.tax_policy`), so `country` resolves to `"AT" | "Generic"`.
 *
 * `encrypted` means the UI requested daemon-backed SQLCipher initialization
 * during onboarding. The passphrase is never written into this persisted
 * store; the user must re-enter it when a locked daemon session needs to open
 * the database.
 *
 * `aiSetupMode` records welcome-flow intent for a newly-created books set.
 * The running UI uses `aiFeaturesEnabled` as the global feature switch after
 * onboarding, so users can enable or disable the assistant later.
 */
export interface Identity {
  name: string;
  workspace: string;
  country: string;
  encrypted: boolean;
  profile?: string;
  taxCountry?: "at" | "generic";
  fiatCurrency?: string;
  taxLongTermDays?: number;
  /**
   * Token matches `kassiber.tax_policy` / rp2 plugin tokens. Generic supports
   * FIFO/LIFO/HIFO/LOFO + plain MOVING_AVERAGE; AT defaults to MOVING_AVERAGE_AT
   * but also accepts the lot methods.
   */
  gainsAlgorithm?:
    | "FIFO"
    | "LIFO"
    | "HIFO"
    | "LOFO"
    | "MOVING_AVERAGE"
    | "MOVING_AVERAGE_AT";
  databaseMode?: "sqlcipher" | "plaintext";
  migrateCredentials?: boolean;
  backendSetupMode?: "default" | "custom" | "skip";
  backendKind?:
    | "esplora"
    | "electrum"
    | "bitcoinrpc"
    | "btcpay"
    | "liquid-esplora"
    | "custom";
  backendName?: string;
  backendUrl?: string;
  backendTrustSsl?: boolean;
  backendCertificate?: string;
  backendProxy?: {
    host: string;
    port: string;
  } | null;
  aiSetupMode?: "local" | "remote" | "disabled";
  aiProviderKind?: "local" | "remote" | "tee";
  aiProviderName?: string;
  aiBaseUrl?: string;
  importedProject?: ImportedProjectIdentity;
}

export interface SourceFundsDraft {
  target?: string;
  targetAmount?: string;
  reportPurpose?: "existing_transaction" | "planned_exchange_sale";
  plannedDestination?: string;
  plannedNote?: string;
  revealMode?: string;
  diagramDetail?: "summary" | "detailed";
  selectedRecipientId?: string;
  /**
   * Case-dossier stage. Older drafts may carry the retired wizard values
   * ("setup" / "review"); readers map those onto the dossier stages.
   */
  currentStep?: "target" | "trace" | "disclose" | "export" | "setup" | "review";
}

/** Captures a "I was setting up X, came here to add a backend, take me back" hop. */
export interface DeferredConnectionSetup {
  sourceId: string;
  reason: string;
  backendKind?: string;
}

export type AssistantDockPosition = "left" | "center" | "right";

export interface UiState {
  lang: Lang;
  currency: Currency;
  dataMode: DataMode;
  theme: ThemePreference;
  appScale: number;
  /**
   * Automatic screen-fit factor derived from the window size (see
   * `lib/appAutoScale.ts`). Ephemeral — recomputed on every launch/resize by
   * `AppScaleController`, never persisted. The effective on-screen scale is
   * `appAutoScale * appScale`; Settings shows that product.
   */
  appAutoScale: number;
  hideSensitive: boolean;
  clearClipboard: boolean;
  explorerSettings: ExplorerSettings;
  appLockPolicy: AppLockPolicy;
  identity: Identity | null;
  aiFeaturesEnabled: boolean;
  developerToolsEnabled: boolean;
  /** Native-hydrated app-wide GitHub update-check permission. */
  automaticUpdateChecks: boolean;
  /** Latest native GitHub release check; transient and never persisted. */
  appUpdate: AppUpdateCheck | null;
  assistantModelSelection: AiModelSelection | null;
  /**
   * macOS-Dock-style auto-hide for the shell assistant dock: parked at the
   * bottom edge with a slim sliver visible, revealed by hovering the edge.
   * An active conversation or focus pins the dock regardless.
   */
  assistantDockAutoHide: boolean;
  assistantDockPosition: AssistantDockPosition;
  /**
   * The user has opened the shell assistant dock at least once. Persisted so
   * the parked pill can teach new users (labeled + elevated) then quiet down
   * to an icon-only corner chip after discovery.
   */
  assistantDockDiscovered: boolean;
  /**
   * User collapsed an in-progress conversation to the docked pill. Ephemeral
   * (not persisted) — a reload always restores the full dock, and clearing the
   * thread or new activity (streaming/consent) resets it.
   */
  assistantDockMinimized: boolean;
  /**
   * The dock is showing a live conversation (expanded, not minimized), so the
   * shell must reserve real bottom padding instead of just the parked-pill
   * sliver. Ephemeral — the dock reports it; it is never persisted.
   */
  assistantDockExpanded: boolean;
  daemonSession: number;
  notifications: AppNotification[];
  activeMaintenanceProgress: ActiveMaintenanceProgress | null;
  /**
   * Book keys (see `bookIdentityKey`) whose initial sync has completed at least
   * once. Drives the one-time first-sync experience: a brand-new book gets the
   * centered setup card, while every refresh after that shows only as the top
   * progress line.
   */
  firstSyncDone: Record<string, true>;
  /**
   * Per-book chart/table period preference shared by Overview and Transactions.
   * The URL `period` query param can still override this for shareable views;
   * this map supplies the book-scoped default across navigation and app restarts.
   */
  bookChartPeriods: Record<string, BookChartPeriod>;
  /**
   * Book keys whose in-progress first-sync card the user collapsed via
   * "Continue in background". Ephemeral (not persisted): re-opening from the
   * book-refresh notification clears it, and a completed sync makes it moot.
   */
  firstSyncCardDismissed: Record<string, true>;
  sourceFundsDrafts: Record<string, SourceFundsDraft>;
  deferredConnectionSetup: DeferredConnectionSetup | null;
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setDataMode: (dataMode: DataMode) => void;
  setTheme: (theme: ThemePreference) => void;
  setAppScale: (appScale: number) => void;
  setAppAutoScale: (appAutoScale: number) => void;
  increaseAppScale: () => void;
  decreaseAppScale: () => void;
  resetAppScale: () => void;
  setHideSensitive: (hideSensitive: boolean) => void;
  setClearClipboard: (clearClipboard: boolean) => void;
  setExplorerSettings: (settings: Partial<ExplorerSettings>) => void;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  setIdentity: (identity: Identity | null) => void;
  setAiFeaturesEnabled: (enabled: boolean) => void;
  setDeveloperToolsEnabled: (enabled: boolean) => void;
  setAutomaticUpdateChecks: (enabled: boolean) => void;
  setAppUpdate: (update: AppUpdateCheck | null) => void;
  setAssistantModelSelection: (selection: AiModelSelection | null) => void;
  setAssistantDockAutoHide: (enabled: boolean) => void;
  setAssistantDockPosition: (position: AssistantDockPosition) => void;
  setAssistantDockDiscovered: (discovered: boolean) => void;
  setAssistantDockMinimized: (minimized: boolean) => void;
  setAssistantDockExpanded: (expanded: boolean) => void;
  bumpDaemonSession: () => void;
  addNotification: (
    notification: Omit<AppNotification, "id" | "createdAt">,
  ) => string;
  updateNotification: (
    id: string,
    patch: Partial<Omit<AppNotification, "id" | "createdAt">>,
  ) => void;
  setActiveMaintenanceProgress: (
    progress: ActiveMaintenanceProgress | null,
  ) => void;
  clearActiveMaintenanceProgress: (id?: string) => void;
  markFirstSyncDone: (bookKey: string) => void;
  dismissFirstSyncCard: (bookKey: string) => void;
  reopenFirstSyncCard: (bookKey: string) => void;
  clearNotification: (id: string) => void;
  clearNotifications: () => void;
  setBookChartPeriod: (
    bookKey: string,
    period: BookChartPeriod,
  ) => void;
  setSourceFundsDraft: (profileKey: string, draft: SourceFundsDraft) => void;
  clearSourceFundsDraft: (profileKey: string) => void;
  setDeferredConnectionSetup: (intent: DeferredConnectionSetup | null) => void;
  clearDeferredConnectionSetup: () => void;
}

const DEFAULT_APP_LOCK_POLICY: AppLockPolicy = {
  autoLockWhenIdle: false,
  idleMinutes: 5,
  requirePassphraseOnLaunch: false,
  lockOnWindowClose: false,
  touchIdUnlock: false,
};

function normalizeIdentity(identity: Identity | null): Identity | null {
  if (!identity) return identity;
  const isLegacyMockIdentity =
    !identity.importedProject &&
    identity.workspace === "Demo Workspace" &&
    (identity.name === "mock profile" || identity.profile === "mock");

  if (!isLegacyMockIdentity) return identity;

  return {
    ...identity,
    name: identity.name === "mock profile" ? "mock books" : identity.name,
    profile: identity.profile === "mock" ? "mock books" : identity.profile,
    workspace: "My Books",
  };
}

function isRegtestIdentity(identity: Identity | null): boolean {
  if (!identity) return false;
  const workspace = identity.workspace?.trim().toLowerCase();
  const dataRoot = identity.importedProject?.dataRoot?.trim().toLowerCase();
  return (
    workspace === "regtest demo" ||
    Boolean(dataRoot && dataRoot.includes("regtest-demo"))
  );
}

function normalizeStoredDataMode(
  dataMode: DataMode | undefined,
  identity: Identity | null,
): DataMode {
  if (dataMode === "mock") {
    return isRegtestIdentity(identity) ? "regtest" : "real";
  }
  return dataMode ?? "real";
}

/**
 * Stable key for the active book, used to remember whether its initial sync has
 * happened. An imported project is keyed by its database path; an
 * onboarding-only identity falls back to its workspace/profile scope.
 */
export function bookIdentityKey(identity: Identity | null): string | null {
  if (!identity) return null;
  if (identity.importedProject?.database) {
    return `db:${identity.importedProject.database}`;
  }
  const scope = [identity.workspace, identity.profile ?? identity.name]
    .map((part) => part?.trim())
    .filter((part): part is string => Boolean(part));
  return scope.length > 0 ? `id:${scope.join("/")}` : null;
}

function stripNotificationProgress(
  notifications: AppNotification[] | undefined,
): AppNotification[] {
  return (notifications ?? []).map((notification) => {
    const clone = { ...notification };
    delete clone.progress;
    return clone;
  });
}

const BOOK_CHART_PERIODS = new Set<BookChartPeriod>([
  "auto",
  "30days",
  "3months",
  "6months",
  "ytd",
  "1year",
  "5years",
  "10years",
  "15years",
  "all",
]);

function normalizeBookChartPeriods(
  periods: unknown,
): Record<string, BookChartPeriod> {
  if (!periods || typeof periods !== "object") return {};
  return Object.fromEntries(
    Object.entries(periods as Record<string, unknown>).filter(
      (entry): entry is [string, BookChartPeriod] =>
        typeof entry[1] === "string" &&
        BOOK_CHART_PERIODS.has(entry[1] as BookChartPeriod),
    ),
  );
}

export function normalizeAppScale(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return DEFAULT_APP_SCALE;
  }
  const stepped = Math.round(value / APP_SCALE_STEP) * APP_SCALE_STEP;
  const clamped = Math.min(MAX_APP_SCALE, Math.max(MIN_APP_SCALE, stepped));
  return Number(clamped.toFixed(2));
}

export function uiStatePartialForStorage(state: UiState) {
  return {
    lang: state.lang,
    currency: state.currency,
    dataMode: normalizeStoredDataMode(state.dataMode, state.identity),
    theme: state.theme,
    hideSensitive: state.hideSensitive,
    clearClipboard: state.clearClipboard,
    appScale: state.appScale,
    explorerSettings: state.explorerSettings,
    appLockPolicy: state.appLockPolicy,
    identity: state.identity,
    aiFeaturesEnabled: state.aiFeaturesEnabled,
    developerToolsEnabled: state.developerToolsEnabled,
    assistantModelSelection: state.assistantModelSelection,
    assistantDockAutoHide: state.assistantDockAutoHide,
    assistantDockPosition: state.assistantDockPosition,
    assistantDockDiscovered: state.assistantDockDiscovered,
    daemonSession: state.daemonSession,
    notifications: stripNotificationProgress(state.notifications),
    firstSyncDone: state.firstSyncDone,
    bookChartPeriods: state.bookChartPeriods,
    sourceFundsDrafts: state.sourceFundsDrafts,
  };
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      lang: "en",
      currency: "btc",
      dataMode: "real",
      theme: DEFAULT_THEME,
      appScale: DEFAULT_APP_SCALE,
      appAutoScale: MAX_AUTO_SCALE,
      hideSensitive: false,
      clearClipboard: true,
      explorerSettings: DEFAULT_EXPLORER_SETTINGS,
      appLockPolicy: DEFAULT_APP_LOCK_POLICY,
      identity: null,
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      automaticUpdateChecks: false,
      appUpdate: null,
      assistantModelSelection: null,
      assistantDockAutoHide: true,
      // Corner park keeps the idle pill off the content axis; Settings can
      // still move it left/center/right.
      assistantDockPosition: "right",
      assistantDockDiscovered: false,
      assistantDockMinimized: false,
      assistantDockExpanded: false,
      daemonSession: 0,
      notifications: [],
      activeMaintenanceProgress: null,
      firstSyncDone: {},
      bookChartPeriods: {},
      firstSyncCardDismissed: {},
      sourceFundsDrafts: {},
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setDataMode: (dataMode) =>
        set((state) => ({
          dataMode: normalizeStoredDataMode(dataMode, state.identity),
        })),
      setTheme: (theme) => set({ theme }),
      setAppScale: (appScale) =>
        set({ appScale: normalizeAppScale(appScale) }),
      setAppAutoScale: (appAutoScale) => set({ appAutoScale }),
      increaseAppScale: () =>
        set((state) => ({
          appScale: normalizeAppScale(state.appScale + APP_SCALE_STEP),
        })),
      decreaseAppScale: () =>
        set((state) => ({
          appScale: normalizeAppScale(state.appScale - APP_SCALE_STEP),
        })),
      resetAppScale: () => set({ appScale: DEFAULT_APP_SCALE }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
      setClearClipboard: (clearClipboard) => set({ clearClipboard }),
      setExplorerSettings: (settings) =>
        set((state) => ({
          explorerSettings: { ...state.explorerSettings, ...settings },
        })),
      setAppLockPolicy: (policy) =>
        set((state) => ({
          appLockPolicy: { ...state.appLockPolicy, ...policy },
        })),
      setIdentity: (identity) =>
        set((state) => {
          const normalized = normalizeIdentity(identity);
          return {
            identity: normalized,
            aiFeaturesEnabled: normalized?.aiSetupMode
              ? normalized.aiSetupMode !== "disabled"
              : state.aiFeaturesEnabled,
          };
        }),
      setAiFeaturesEnabled: (enabled) => set({ aiFeaturesEnabled: enabled }),
      setDeveloperToolsEnabled: (enabled) =>
        set({ developerToolsEnabled: enabled }),
      setAutomaticUpdateChecks: (enabled) =>
        set({
          automaticUpdateChecks: enabled,
          ...(enabled ? {} : { appUpdate: null }),
        }),
      setAppUpdate: (appUpdate) => set({ appUpdate }),
      setAssistantModelSelection: (assistantModelSelection) =>
        set({ assistantModelSelection }),
      setAssistantDockAutoHide: (assistantDockAutoHide) =>
        set({ assistantDockAutoHide }),
      setAssistantDockPosition: (assistantDockPosition) =>
        set({ assistantDockPosition }),
      setAssistantDockDiscovered: (assistantDockDiscovered) =>
        set({ assistantDockDiscovered }),
      setAssistantDockMinimized: (assistantDockMinimized) =>
        set({ assistantDockMinimized }),
      setAssistantDockExpanded: (assistantDockExpanded) =>
        set({ assistantDockExpanded }),
      bumpDaemonSession: () =>
        set((state) => ({ daemonSession: state.daemonSession + 1 })),
      addNotification: (notification) => {
        const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        let existingId: string | null = null;
        set((state) => ({
          notifications: (() => {
            const createdAt = new Date().toISOString();
            if (notification.dedupeKey) {
              const existing = state.notifications.find(
                (item) => item.dedupeKey === notification.dedupeKey,
              );
              if (existing) {
                existingId = existing.id;
                return [
                  {
                    ...existing,
                    ...notification,
                    progress: notification.progress,
                    createdAt,
                  },
                  ...state.notifications.filter((item) => item.id !== existing.id),
                ].slice(0, 12);
              }
            }
            return [
              {
                ...notification,
                id,
                createdAt,
              },
              ...state.notifications,
            ].slice(0, 12);
          })(),
        }));
        return existingId ?? id;
      },
      updateNotification: (id, patch) =>
        set((state) => ({
          notifications: state.notifications.map((notification) =>
            notification.id === id ? { ...notification, ...patch } : notification,
          ),
        })),
      setActiveMaintenanceProgress: (activeMaintenanceProgress) =>
        set({ activeMaintenanceProgress }),
      clearActiveMaintenanceProgress: (id) =>
        set((state) => {
          const current = state.activeMaintenanceProgress;
          if (!current) return state;
          if (id && current.id !== id) return state;
          return { activeMaintenanceProgress: null };
        }),
      markFirstSyncDone: (bookKey) =>
        set((state) =>
          state.firstSyncDone[bookKey]
            ? state
            : { firstSyncDone: { ...state.firstSyncDone, [bookKey]: true } },
        ),
      dismissFirstSyncCard: (bookKey) =>
        set((state) =>
          state.firstSyncCardDismissed[bookKey]
            ? state
            : {
                firstSyncCardDismissed: {
                  ...state.firstSyncCardDismissed,
                  [bookKey]: true,
                },
              },
        ),
      reopenFirstSyncCard: (bookKey) =>
        set((state) => {
          if (!state.firstSyncCardDismissed[bookKey]) return state;
          const next = { ...state.firstSyncCardDismissed };
          delete next[bookKey];
          return { firstSyncCardDismissed: next };
        }),
      clearNotification: (id) =>
        set((state) => ({
          notifications: state.notifications.filter(
            (notification) => notification.id !== id,
          ),
        })),
      clearNotifications: () => set({ notifications: [] }),
      setBookChartPeriod: (bookKey, period) =>
        set((state) =>
          state.bookChartPeriods[bookKey] === period
            ? state
            : {
                bookChartPeriods: {
                  ...state.bookChartPeriods,
                  [bookKey]: period,
                },
              },
        ),
      setSourceFundsDraft: (profileKey, draft) =>
        set((state) => {
          const existing = state.sourceFundsDrafts[profileKey] ?? {};
          return {
            sourceFundsDrafts: {
              ...state.sourceFundsDrafts,
              [profileKey]: { ...existing, ...draft },
            },
          };
        }),
      clearSourceFundsDraft: (profileKey) =>
        set((state) => {
          if (!(profileKey in state.sourceFundsDrafts)) return state;
          const next = { ...state.sourceFundsDrafts };
          delete next[profileKey];
          return { sourceFundsDrafts: next };
        }),
      deferredConnectionSetup: null,
      setDeferredConnectionSetup: (intent) =>
        set({ deferredConnectionSetup: intent }),
      clearDeferredConnectionSetup: () =>
        set({ deferredConnectionSetup: null }),
    }),
    {
      name: "kb.ui",
      partialize: uiStatePartialForStorage,
      merge: (persisted, current) => {
        const restored = persisted as Partial<UiState>;
        const identity = normalizeIdentity(restored.identity ?? current.identity);
        const dataMode = normalizeStoredDataMode(
          restored.dataMode,
          identity ?? current.identity,
        );
        const aiFeaturesEnabled =
          restored.aiFeaturesEnabled ??
          (identity?.aiSetupMode === "disabled"
            ? false
            : current.aiFeaturesEnabled);
        return {
          ...current,
          ...restored,
          dataMode,
          appScale: normalizeAppScale(restored.appScale ?? current.appScale),
          clearClipboard: restored.clearClipboard ?? current.clearClipboard,
          explorerSettings: {
            ...DEFAULT_EXPLORER_SETTINGS,
            ...(restored.explorerSettings ?? current.explorerSettings),
          },
          appLockPolicy: {
            ...DEFAULT_APP_LOCK_POLICY,
            ...(restored.appLockPolicy ?? current.appLockPolicy),
          },
          identity,
          aiFeaturesEnabled,
          developerToolsEnabled:
            restored.developerToolsEnabled ?? current.developerToolsEnabled,
          // The owner-only native/CLI preference is canonical. Never restore
          // this permission from renderer-local storage.
          automaticUpdateChecks: current.automaticUpdateChecks,
          appUpdate: null,
          assistantModelSelection:
            restored.assistantModelSelection ??
            current.assistantModelSelection,
          assistantDockAutoHide:
            restored.assistantDockAutoHide ?? current.assistantDockAutoHide,
          assistantDockPosition:
            restored.assistantDockPosition ?? current.assistantDockPosition,
          // Missing flag on upgrade ⇒ treat as discovered so veterans don't
          // get the new-user pulse again. Brand-new installs keep the initial
          // `false` from create() because merge isn't fed prior dock keys.
          assistantDockDiscovered:
            typeof restored.assistantDockDiscovered === "boolean"
              ? restored.assistantDockDiscovered
              : restored.assistantDockAutoHide !== undefined ||
                  restored.assistantDockPosition !== undefined
                ? true
                : current.assistantDockDiscovered,
          notifications: stripNotificationProgress(
            restored.notifications ?? current.notifications,
          ),
          firstSyncDone: restored.firstSyncDone ?? current.firstSyncDone,
          bookChartPeriods: normalizeBookChartPeriods(
            restored.bookChartPeriods ??
              (restored as { overviewChartPeriods?: unknown })
                .overviewChartPeriods ??
              current.bookChartPeriods,
          ),
          sourceFundsDrafts:
            restored.sourceFundsDrafts ?? current.sourceFundsDrafts,
        };
      },
    },
  ),
);
