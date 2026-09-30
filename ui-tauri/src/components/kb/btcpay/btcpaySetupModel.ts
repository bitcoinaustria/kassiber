/**
 * Pure model for the BTCPay Server connection setup.
 *
 * The Python core inspects the merchant's server and returns a plan
 * (`ui.connections.btcpay.discover`) with a recommendation per store payment
 * method. This module only turns that plan plus the user's choices into the
 * `ui.connections.btcpay.create` payload; it never decides accounting on its
 * own. The core re-validates every route when the setup is created.
 */

export type BtcpayRouteAction =
  | "wallet_source"
  | "existing_wallet"
  | "payment_ledger"
  | "provenance_only"
  | "skip";

export const BTCPAY_ROUTE_ACTIONS: readonly BtcpayRouteAction[] = [
  "wallet_source",
  "existing_wallet",
  "payment_ledger",
  "provenance_only",
  "skip",
];

export type BtcpayKeyPreset = "read_only" | "wallet_history";

export type BtcpayKeyRisk =
  | "read_only"
  | "can_write"
  | "can_modify_store"
  | "server_admin"
  | "unknown";

export type BtcpayRail = "onchain" | "lightning" | "lnurl" | "plugin";

export interface BtcpayKeyGuide {
  preset: BtcpayKeyPreset;
  label?: string;
  summary?: string;
  permissions: string[];
  permission_labels?: string[];
  capabilities?: string[];
  manual_steps?: string[];
  authorize_url?: string;
  server_url?: string | null;
}

export interface BtcpayActionAvailability {
  available: boolean;
  reason: string | null;
  reason_text?: string | null;
}

export interface BtcpayRecommendation {
  action: BtcpayRouteAction;
  wallet: string | null;
  reason: string | null;
  reason_text?: string | null;
}

export interface BtcpayExistingRoute {
  action: BtcpayRouteAction;
  backend?: string;
  wallet?: string | null;
  wallet_id?: string | null;
  store_id: string;
  payment_method_id: string;
}

export interface BtcpayPlanMethod {
  store_id: string;
  payment_method_id: string;
  label: string;
  enabled: boolean;
  sync_supported: boolean;
  rail: BtcpayRail | string;
  currency?: string;
  chain?: string | null;
  bitcoin_asset?: boolean;
  settlement?: string;
  settlement_hint?: string;
  /** Set for Lightning and bitcoin plugin rails BTCPay can book as a ledger. */
  ledger_group?: string | null;
  wallet_history_available?: boolean | null;
  wallet_fingerprint?: string | null;
  network?: string | null;
  preview_error?: string | null;
  network_compatible?: boolean | null;
  existing_routes?: BtcpayExistingRoute[];
  configured_via?: string[];
  owned_by?: { wallet: string; wallet_id: string } | null;
  same_wallet_imported_by?: Array<{ wallet: string; store_id: string }>;
  shared_with?: Array<{ store_id: string; payment_method_id: string }>;
  lightning_wallets?: Array<{ wallet: string; kind: string }>;
  actions?: Partial<Record<BtcpayRouteAction, BtcpayActionAvailability>>;
  recommendation?: BtcpayRecommendation;
}

export type BtcpayCapability =
  | "store_catalog"
  | "wallet_preview"
  | "invoices"
  | "payment_requests"
  | "payouts"
  | "pull_payments"
  | "wallet_history";

/** When Kassiber last loaded a configured store; BTCPay never pushes updates. */
export interface BtcpayStoreSyncState {
  last_attempt_at: string | null;
  last_success_at: string | null;
  last_error_code: string | null;
  age_seconds: number | null;
  stale: boolean;
  never_synced: boolean;
}

export interface BtcpayPlanStore {
  id: string;
  name: string;
  default_currency?: string | null;
  archived?: boolean;
  capabilities?: Partial<Record<BtcpayCapability, boolean | null>>;
  missing_permissions?: string[];
  payment_methods_error?: string | null;
  sync_state?: BtcpayStoreSyncState | null;
}

export interface BtcpayPlanWarning {
  code: string;
  message: string;
  severity?: string;
  store_id?: string;
  store_ids?: string[];
  wallet_fingerprint?: string;
}

export interface BtcpayPlan {
  backend: string;
  server?: {
    version?: string | null;
    fully_synced?: boolean | null;
    tor?: boolean;
    loopback?: boolean;
    transport?: "http" | "https" | string;
  };
  api_key?: {
    label?: string | null;
    known: boolean;
    scope: string;
    store_ids?: string[];
    permissions?: string[];
    risk: BtcpayKeyRisk | string;
    excess_permissions?: string[];
  };
  stores: BtcpayPlanStore[];
  payment_methods: BtcpayPlanMethod[];
  warnings?: BtcpayPlanWarning[];
  sibling_backends?: string[];
  existing_routes?: BtcpayExistingRoute[];
  book_network?: { state?: string | null; environment?: string | null };
  detected_network?: string | null;
  key_upgrade?: BtcpayKeyGuide;
  key_read_only?: BtcpayKeyGuide;
}

export interface BtcpayRouteChoice {
  action: BtcpayRouteAction;
  wallet: string;
}

export interface BtcpayWalletOption {
  label: string;
  kind?: string;
  chain?: string | null;
  sync_source?: string | null;
}

export interface BtcpayManualRoute {
  storeId: string;
  paymentMethodId: string;
  action: BtcpayRouteAction;
  wallet: string;
}

export type BtcpayInstanceArgs =
  | { backend: string }
  | { backend_label: string; server_url: string; api_key: string };

export interface BtcpayCreateRoute {
  store_id: string;
  store_name?: string;
  payment_method_id: string;
  action: BtcpayRouteAction;
  wallet?: string;
  label?: string;
  wallet_fingerprint?: string;
  network?: string;
}

export type BtcpaySetupIssue =
  | "discover_first"
  | "no_routes"
  | "choose_wallet"
  | "no_wallets"
  | "manual_store_id";

export const DEFAULT_BTCPAY_PAYMENT_METHOD_ID = "BTC-CHAIN";

export function btcpayRouteKey(storeId: string, paymentMethodId: string) {
  return `${storeId}\u0000${paymentMethodId}`;
}

/** Wallets BTCPay can be mapped onto: the wallets that hold the funds. */
export function settlementWalletOptions(
  wallets: readonly BtcpayWalletOption[],
): BtcpayWalletOption[] {
  return wallets.filter((wallet) => wallet.sync_source !== "btcpay");
}

/** Settlement wallets on the same chain as the payment method. */
export function walletOptionsForMethod(
  method: Pick<BtcpayPlanMethod, "chain" | "payment_method_id">,
  wallets: readonly BtcpayWalletOption[],
): BtcpayWalletOption[] {
  const options = settlementWalletOptions(wallets);
  const chain =
    method.chain ??
    (method.payment_method_id.toUpperCase().startsWith("LBTC")
      ? "liquid"
      : "bitcoin");
  const sameChain = options.filter(
    (wallet) => !wallet.chain || wallet.chain === chain,
  );
  return sameChain.length ? sameChain : options;
}

export function actionAvailable(
  method: BtcpayPlanMethod,
  action: BtcpayRouteAction,
): boolean {
  const entry = method.actions?.[action];
  if (entry) return entry.available;
  if (action === "wallet_source" || action === "existing_wallet") {
    return method.sync_supported && method.rail === "onchain";
  }
  if (action === "payment_ledger") return Boolean(method.ledger_group);
  return true;
}

export function defaultRouteChoice(
  method: BtcpayPlanMethod,
  wallets: readonly BtcpayWalletOption[],
): BtcpayRouteChoice {
  const recommended = method.recommendation?.action;
  const action: BtcpayRouteAction =
    recommended && actionAvailable(method, recommended)
      ? recommended
      : actionAvailable(method, "provenance_only")
        ? "provenance_only"
        : "skip";
  const candidates = walletOptionsForMethod(method, wallets);
  const preferred =
    method.recommendation?.wallet ?? method.owned_by?.wallet ?? null;
  const wallet =
    (preferred && candidates.some((option) => option.label === preferred)
      ? preferred
      : candidates.length === 1
        ? candidates[0].label
        : "") ?? "";
  return { action, wallet };
}

export function defaultRouteChoices(
  plan: BtcpayPlan,
  wallets: readonly BtcpayWalletOption[],
): Record<string, BtcpayRouteChoice> {
  const choices: Record<string, BtcpayRouteChoice> = {};
  for (const method of plan.payment_methods) {
    choices[btcpayRouteKey(method.store_id, method.payment_method_id)] =
      defaultRouteChoice(method, wallets);
  }
  return choices;
}

export function choiceForMethod(
  method: BtcpayPlanMethod,
  choices: Record<string, BtcpayRouteChoice>,
  wallets: readonly BtcpayWalletOption[],
): BtcpayRouteChoice {
  const stored = choices[btcpayRouteKey(method.store_id, method.payment_method_id)];
  if (stored && actionAvailable(method, stored.action)) return stored;
  return defaultRouteChoice(method, wallets);
}

export interface BtcpayRoutePlanEntry {
  key: string;
  method: BtcpayPlanMethod;
  store: BtcpayPlanStore | undefined;
  choice: BtcpayRouteChoice;
}

export function planRoutes(
  plan: BtcpayPlan,
  choices: Record<string, BtcpayRouteChoice>,
  wallets: readonly BtcpayWalletOption[],
): BtcpayRoutePlanEntry[] {
  const stores = new Map(plan.stores.map((store) => [store.id, store]));
  return plan.payment_methods.map((method) => ({
    key: btcpayRouteKey(method.store_id, method.payment_method_id),
    method,
    store: stores.get(method.store_id),
    choice: choiceForMethod(method, choices, wallets),
  }));
}

export function summarizeRoutes(entries: readonly { choice: BtcpayRouteChoice }[]) {
  const counts: Record<BtcpayRouteAction, number> = {
    wallet_source: 0,
    existing_wallet: 0,
    payment_ledger: 0,
    provenance_only: 0,
    skip: 0,
  };
  for (const entry of entries) counts[entry.choice.action] += 1;
  return counts;
}

export function validateBtcpaySetup({
  plan,
  entries,
  manualRoute,
  wallets,
}: {
  plan: BtcpayPlan | null;
  entries: readonly BtcpayRoutePlanEntry[];
  manualRoute: BtcpayManualRoute | null;
  wallets: readonly BtcpayWalletOption[];
}): BtcpaySetupIssue | null {
  if (!plan) return "discover_first";
  if (plan.payment_methods.length === 0) {
    if (!manualRoute) return "no_routes";
    if (!manualRoute.storeId.trim()) return "manual_store_id";
    if (manualRoute.action === "skip") return "no_routes";
    if (manualRoute.action === "existing_wallet") {
      if (settlementWalletOptions(wallets).length === 0) return "no_wallets";
      if (!manualRoute.wallet) return "choose_wallet";
    }
    return null;
  }
  const active = entries.filter((entry) => entry.choice.action !== "skip");
  const changesExisting = entries.some(
    (entry) =>
      entry.choice.action === "skip" &&
      (entry.method.existing_routes ?? []).length > 0,
  );
  if (active.length === 0 && !changesExisting) return "no_routes";
  const mapped = active.filter((entry) => entry.choice.action === "existing_wallet");
  if (mapped.length) {
    if (settlementWalletOptions(wallets).length === 0) return "no_wallets";
    if (mapped.some((entry) => !entry.choice.wallet)) return "choose_wallet";
  }
  return null;
}

export function buildCreateRoutes({
  plan,
  entries,
  manualRoute,
  connectionLabel,
}: {
  plan: BtcpayPlan;
  entries: readonly BtcpayRoutePlanEntry[];
  manualRoute: BtcpayManualRoute | null;
  connectionLabel: string;
}): BtcpayCreateRoute[] {
  if (plan.payment_methods.length === 0) {
    if (!manualRoute) return [];
    const paymentMethodId =
      manualRoute.paymentMethodId.trim().toUpperCase() ||
      DEFAULT_BTCPAY_PAYMENT_METHOD_ID;
    return [
      {
        store_id: manualRoute.storeId.trim(),
        payment_method_id: paymentMethodId,
        action: manualRoute.action,
        ...(manualRoute.action === "existing_wallet"
          ? { wallet: manualRoute.wallet }
          : {}),
        ...(manualRoute.action === "wallet_source"
          ? { label: connectionLabel.trim() }
          : {}),
        ...(plan.detected_network ? { network: plan.detected_network } : {}),
      },
    ];
  }
  const walletSources = entries.filter(
    (entry) => entry.choice.action === "wallet_source",
  );
  return entries
    // Re-sending an unchanged route is idempotent in the core; a skip is only
    // sent when it removes a route that exists today.
    .filter(
      (entry) =>
        entry.choice.action !== "skip" ||
        (entry.method.existing_routes ?? []).length > 0,
    )
    .map((entry) => {
      const route: BtcpayCreateRoute = {
        store_id: entry.method.store_id,
        payment_method_id: entry.method.payment_method_id,
        action: entry.choice.action,
      };
      if (entry.store?.name) route.store_name = entry.store.name;
      if (entry.choice.action === "existing_wallet") {
        route.wallet = entry.choice.wallet;
      }
      if (entry.choice.action === "skip") {
        const mappedWallet = (entry.method.existing_routes ?? []).find(
          (existing) => existing.action === "existing_wallet" && existing.wallet,
        )?.wallet;
        if (mappedWallet) route.wallet = mappedWallet;
      }
      if (entry.choice.action === "wallet_source" && walletSources.length === 1) {
        route.label = connectionLabel.trim();
      }
      if (entry.method.wallet_fingerprint) {
        route.wallet_fingerprint = entry.method.wallet_fingerprint;
      }
      // Lightning and plugin rails expose no addresses; a ledger takes the
      // server's on-chain network.
      const network =
        entry.method.network ??
        (entry.choice.action === "payment_ledger" ? plan.detected_network : null);
      if (network) route.network = network;
      return route;
    });
}

export function buildCreateArgs({
  instance,
  isNewInstance,
  plan,
  routes,
  connectionLabel,
}: {
  instance: BtcpayInstanceArgs;
  isNewInstance: boolean;
  plan: BtcpayPlan;
  routes: BtcpayCreateRoute[];
  connectionLabel: string;
}): Record<string, unknown> {
  return {
    ...instance,
    mode: "account",
    label: connectionLabel.trim(),
    routes,
    sync_provenance: true,
    ...(isNewInstance && plan.detected_network
      ? { network: plan.detected_network }
      : {}),
  };
}

/**
 * Wallets the post-create refresh should sync. `createdWalletSources` holds
 * every wallet that takes its balance from BTCPay: imported wallet history and
 * payment ledgers.
 */
export function walletsToRefresh(
  routes: readonly BtcpayCreateRoute[],
  createdWalletSources: readonly string[],
): { walletSources: string[]; mappedWallets: string[] } {
  const mappedWallets = Array.from(
    new Set(
      routes
        .filter((route) => route.action === "existing_wallet" && route.wallet)
        .map((route) => route.wallet as string),
    ),
  );
  return { walletSources: [...createdWalletSources], mappedWallets };
}

export type BtcpayTone = "good" | "warning" | "danger" | "neutral";

export function keyRiskTone(risk: string | undefined | null): BtcpayTone {
  switch (risk) {
    case "read_only":
      return "good";
    case "can_write":
    case "can_modify_store":
      return "warning";
    case "server_admin":
      return "danger";
    default:
      return "neutral";
  }
}

export function railKind(method: Pick<BtcpayPlanMethod, "rail" | "chain" | "bitcoin_asset">):
  | "onchain"
  | "liquid"
  | "lightning"
  | "lnurl"
  | "plugin"
  | "other_asset" {
  if (method.bitcoin_asset === false) {
    return method.rail === "plugin" ? "plugin" : "other_asset";
  }
  if (method.rail === "onchain") return method.chain === "liquid" ? "liquid" : "onchain";
  if (method.rail === "lightning") return "lightning";
  if (method.rail === "lnurl") return "lnurl";
  return "plugin";
}

/** Basic client-side check before asking the core for a key link. */
export function looksLikeServerUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!/^https?:\/\//i.test(trimmed)) return false;
  try {
    const url = new URL(trimmed);
    return Boolean(url.hostname);
  } catch {
    return false;
  }
}

export function storeName(plan: BtcpayPlan | null, storeId: string): string {
  return plan?.stores.find((store) => store.id === storeId)?.name ?? storeId;
}

export type BtcpayStoreFreshness = "never" | "failed" | "stale" | "fresh";

export function storeFreshness(
  state: BtcpayStoreSyncState | null | undefined,
): BtcpayStoreFreshness | null {
  if (!state) return null;
  if (state.never_synced) return "never";
  if (state.last_error_code) return "failed";
  return state.stale ? "stale" : "fresh";
}
