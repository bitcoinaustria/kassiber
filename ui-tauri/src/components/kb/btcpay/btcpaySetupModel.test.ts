import { describe, expect, it } from "vitest";

import {
  btcpayRouteKey,
  buildCreateArgs,
  buildCreateRoutes,
  defaultRouteChoices,
  keyRiskTone,
  looksLikeServerUrl,
  planRoutes,
  railKind,
  storeFreshness,
  summarizeRoutes,
  validateBtcpaySetup,
  walletOptionsForMethod,
  walletsToRefresh,
  type BtcpayPlan,
  type BtcpayPlanMethod,
  type BtcpayWalletOption,
} from "./btcpaySetupModel";

const POPUP = "store-popup";
const CAFE = "store-cafe";
const MAIN = "store-main";

function method(overrides: Partial<BtcpayPlanMethod>): BtcpayPlanMethod {
  return {
    store_id: MAIN,
    payment_method_id: "BTC-CHAIN",
    label: "Bitcoin on-chain",
    enabled: true,
    sync_supported: true,
    rail: "onchain",
    chain: "bitcoin",
    bitcoin_asset: true,
    settlement: "btc_onchain",
    wallet_fingerprint: null,
    network: "regtest",
    existing_routes: [],
    configured_via: [],
    owned_by: null,
    shared_with: [],
    actions: {
      wallet_source: { available: true, reason: null },
      existing_wallet: { available: true, reason: null },
      provenance_only: { available: true, reason: null },
      skip: { available: true, reason: null },
    },
    recommendation: { action: "wallet_source", wallet: null, reason: "import_btcpay_wallet_history" },
    ...overrides,
  };
}

// Shape mirrors a live BTCPay 2.3.9 plan: two stores share one wallet that
// Kassiber already tracks; a café store uses a read-only key with Lightning.
const readOnlyPlan: BtcpayPlan = {
  backend: "shop-btcpay",
  api_key: { known: true, scope: "all_stores", risk: "read_only", label: "kassiber-ro-all" },
  detected_network: "regtest",
  stores: [
    { id: POPUP, name: "Pop-up Stand" },
    { id: CAFE, name: "Café" },
    { id: MAIN, name: "Main Store" },
  ],
  payment_methods: [
    method({
      store_id: POPUP,
      wallet_fingerprint: "btcpay-wallet:shared",
      owned_by: { wallet: "Shop Trezor", wallet_id: "w-1" },
      shared_with: [{ store_id: MAIN, payment_method_id: "BTC-CHAIN" }],
      actions: {
        wallet_source: { available: false, reason: "wallet_history_permission_missing" },
        existing_wallet: { available: true, reason: null },
        provenance_only: { available: true, reason: null },
        skip: { available: true, reason: null },
      },
      recommendation: { action: "existing_wallet", wallet: "Shop Trezor", reason: "wallet_recognised" },
    }),
    method({
      store_id: CAFE,
      payment_method_id: "BTC-LN",
      label: "Lightning",
      sync_supported: false,
      rail: "lightning",
      network: null,
      settlement: "lightning",
      ledger_group: "lightning",
      actions: {
        wallet_source: { available: false, reason: "not_on_chain" },
        existing_wallet: { available: false, reason: "not_on_chain" },
        payment_ledger: { available: true, reason: null },
        provenance_only: { available: true, reason: null },
        skip: { available: true, reason: null },
      },
      recommendation: { action: "payment_ledger", wallet: null, reason: "lightning_payment_ledger" },
    }),
    method({
      store_id: CAFE,
      wallet_fingerprint: "btcpay-wallet:cafe",
      actions: {
        wallet_source: { available: false, reason: "wallet_history_permission_missing" },
        existing_wallet: { available: true, reason: null },
        provenance_only: { available: true, reason: null },
        skip: { available: true, reason: null },
      },
      recommendation: { action: "provenance_only", wallet: null, reason: "read_only_key_upgrade" },
    }),
    method({
      store_id: MAIN,
      wallet_fingerprint: "btcpay-wallet:shared",
      owned_by: { wallet: "Shop Trezor", wallet_id: "w-1" },
      shared_with: [{ store_id: POPUP, payment_method_id: "BTC-CHAIN" }],
      recommendation: { action: "existing_wallet", wallet: "Shop Trezor", reason: "wallet_recognised" },
    }),
  ],
};

const wallets: BtcpayWalletOption[] = [
  { label: "Shop Trezor", kind: "descriptor", chain: "bitcoin", sync_source: "" },
  { label: "Liquid Hot", kind: "descriptor", chain: "liquid", sync_source: "" },
  { label: "Old BTCPay Source", kind: "custom", chain: "bitcoin", sync_source: "btcpay" },
];

describe("btcpaySetupModel", () => {
  it("preselects the core's recommendation and the recognised wallet", () => {
    const choices = defaultRouteChoices(readOnlyPlan, wallets);

    expect(choices[btcpayRouteKey(POPUP, "BTC-CHAIN")]).toEqual({
      action: "existing_wallet",
      wallet: "Shop Trezor",
    });
    expect(choices[btcpayRouteKey(CAFE, "BTC-LN")].action).toBe("payment_ledger");
    expect(choices[btcpayRouteKey(CAFE, "BTC-CHAIN")].action).toBe("provenance_only");
  });

  it("never keeps an action the core marked unavailable", () => {
    const entries = planRoutes(
      readOnlyPlan,
      { [btcpayRouteKey(POPUP, "BTC-CHAIN")]: { action: "wallet_source", wallet: "" } },
      wallets,
    );

    expect(entries[0].choice.action).toBe("existing_wallet");
  });

  it("builds create routes with fingerprints, networks, and store names", () => {
    const entries = planRoutes(readOnlyPlan, defaultRouteChoices(readOnlyPlan, wallets), wallets);
    const routes = buildCreateRoutes({
      plan: readOnlyPlan,
      entries,
      manualRoute: null,
      connectionLabel: "Shop",
    });

    expect(routes).toHaveLength(4);
    expect(routes[0]).toEqual({
      store_id: POPUP,
      store_name: "Pop-up Stand",
      payment_method_id: "BTC-CHAIN",
      action: "existing_wallet",
      wallet: "Shop Trezor",
      wallet_fingerprint: "btcpay-wallet:shared",
      network: "regtest",
    });
    // A Lightning ledger has no addresses and takes the server's network.
    expect(routes[1]).toEqual({
      store_id: CAFE,
      store_name: "Café",
      payment_method_id: "BTC-LN",
      action: "payment_ledger",
      network: "regtest",
    });
    expect(routes[2]).not.toHaveProperty("wallet");
  });

  it("drops skipped routes unless they remove an existing one", () => {
    const plan: BtcpayPlan = {
      ...readOnlyPlan,
      payment_methods: [
        method({ store_id: MAIN }),
        method({
          store_id: CAFE,
          existing_routes: [
            {
              action: "existing_wallet",
              wallet: "Shop Trezor",
              store_id: CAFE,
              payment_method_id: "BTC-CHAIN",
            },
          ],
        }),
      ],
    };
    const skipAll = {
      [btcpayRouteKey(MAIN, "BTC-CHAIN")]: { action: "skip" as const, wallet: "" },
      [btcpayRouteKey(CAFE, "BTC-CHAIN")]: { action: "skip" as const, wallet: "" },
    };
    const entries = planRoutes(plan, skipAll, wallets);
    const routes = buildCreateRoutes({ plan, entries, manualRoute: null, connectionLabel: "Shop" });

    expect(routes).toEqual([
      expect.objectContaining({ store_id: CAFE, action: "skip", wallet: "Shop Trezor" }),
    ]);
    expect(validateBtcpaySetup({ plan, entries, manualRoute: null, wallets })).toBeNull();
  });

  it("names a single imported wallet after the connection", () => {
    const plan: BtcpayPlan = { ...readOnlyPlan, payment_methods: [method({ store_id: MAIN })] };
    const entries = planRoutes(plan, defaultRouteChoices(plan, wallets), wallets);
    const routes = buildCreateRoutes({ plan, entries, manualRoute: null, connectionLabel: " Shop " });

    expect(routes[0]).toMatchObject({ action: "wallet_source", label: "Shop" });
  });

  it("validates missing plans, wallets, and empty selections", () => {
    expect(validateBtcpaySetup({ plan: null, entries: [], manualRoute: null, wallets })).toBe(
      "discover_first",
    );
    const entries = planRoutes(readOnlyPlan, defaultRouteChoices(readOnlyPlan, wallets), wallets);
    expect(validateBtcpaySetup({ plan: readOnlyPlan, entries, manualRoute: null, wallets })).toBeNull();
    expect(
      validateBtcpaySetup({ plan: readOnlyPlan, entries, manualRoute: null, wallets: [] }),
    ).toBe("no_wallets");
    const unchosen = entries.map((entry) =>
      entry.choice.action === "existing_wallet"
        ? { ...entry, choice: { ...entry.choice, wallet: "" } }
        : entry,
    );
    expect(
      validateBtcpaySetup({ plan: readOnlyPlan, entries: unchosen, manualRoute: null, wallets }),
    ).toBe("choose_wallet");
    const skipped = entries.map((entry) => ({ ...entry, choice: { action: "skip" as const, wallet: "" } }));
    expect(
      validateBtcpaySetup({ plan: readOnlyPlan, entries: skipped, manualRoute: null, wallets }),
    ).toBe("no_routes");
  });

  it("supports a manual store route when the key cannot list payment methods", () => {
    const plan: BtcpayPlan = { ...readOnlyPlan, payment_methods: [] };
    const manual = { storeId: "", paymentMethodId: "btc-chain", action: "provenance_only" as const, wallet: "" };

    expect(validateBtcpaySetup({ plan, entries: [], manualRoute: manual, wallets })).toBe(
      "manual_store_id",
    );
    const filled = { ...manual, storeId: " STORE1 " };
    expect(validateBtcpaySetup({ plan, entries: [], manualRoute: filled, wallets })).toBeNull();
    expect(
      buildCreateRoutes({ plan, entries: [], manualRoute: filled, connectionLabel: "Shop" }),
    ).toEqual([
      {
        store_id: "STORE1",
        payment_method_id: "BTC-CHAIN",
        action: "provenance_only",
        network: "regtest",
      },
    ]);
  });

  it("passes the detected network only when creating a new instance", () => {
    const routes = [{ store_id: MAIN, payment_method_id: "BTC-CHAIN", action: "provenance_only" as const }];
    const inline = buildCreateArgs({
      instance: { backend_label: "Shop", server_url: "https://pay.example", api_key: "key" },
      isNewInstance: true,
      plan: readOnlyPlan,
      routes,
      connectionLabel: " Shop ",
    });
    expect(inline).toMatchObject({ mode: "account", label: "Shop", network: "regtest", sync_provenance: true });
    const saved = buildCreateArgs({
      instance: { backend: "shop-btcpay" },
      isNewInstance: false,
      plan: readOnlyPlan,
      routes,
      connectionLabel: "Shop",
    });
    expect(saved).not.toHaveProperty("network");
  });

  it("refreshes created sources and each mapped wallet once", () => {
    const refresh = walletsToRefresh(
      [
        { store_id: POPUP, payment_method_id: "BTC-CHAIN", action: "existing_wallet", wallet: "Shop Trezor" },
        { store_id: MAIN, payment_method_id: "BTC-CHAIN", action: "existing_wallet", wallet: "Shop Trezor" },
        { store_id: CAFE, payment_method_id: "BTC-LN", action: "provenance_only" },
      ],
      ["Shop - Café - BTC-CHAIN"],
    );

    expect(refresh).toEqual({
      walletSources: ["Shop - Café - BTC-CHAIN"],
      mappedWallets: ["Shop Trezor"],
    });
  });

  it("offers settlement wallets on the matching chain and never BTCPay sources", () => {
    expect(
      walletOptionsForMethod({ chain: "bitcoin", payment_method_id: "BTC-CHAIN" }, wallets).map(
        (wallet) => wallet.label,
      ),
    ).toEqual(["Shop Trezor"]);
    expect(
      walletOptionsForMethod({ chain: null, payment_method_id: "LBTC-CHAIN" }, wallets).map(
        (wallet) => wallet.label,
      ),
    ).toEqual(["Liquid Hot"]);
  });

  it("summarises choices and classifies rails and key risk", () => {
    const entries = planRoutes(readOnlyPlan, defaultRouteChoices(readOnlyPlan, wallets), wallets);
    expect(summarizeRoutes(entries)).toEqual({
      wallet_source: 0,
      existing_wallet: 2,
      payment_ledger: 1,
      provenance_only: 1,
      skip: 0,
    });
    expect(railKind({ rail: "onchain", chain: "liquid", bitcoin_asset: true })).toBe("liquid");
    expect(railKind({ rail: "onchain", chain: null, bitcoin_asset: false })).toBe("other_asset");
    expect(railKind({ rail: "plugin", chain: null, bitcoin_asset: false })).toBe("plugin");
    expect(keyRiskTone("read_only")).toBe("good");
    expect(keyRiskTone("can_modify_store")).toBe("warning");
    expect(keyRiskTone("server_admin")).toBe("danger");
    expect(keyRiskTone(undefined)).toBe("neutral");
  });

  it("reports how current a store's BTCPay data is", () => {
    const base = {
      last_attempt_at: "2026-01-02T00:00:00Z",
      last_success_at: "2026-01-02T00:00:00Z",
      last_error_code: null,
      age_seconds: 60,
      stale: false,
      never_synced: false,
    };
    expect(storeFreshness(undefined)).toBeNull();
    expect(storeFreshness(base)).toBe("fresh");
    expect(storeFreshness({ ...base, stale: true })).toBe("stale");
    expect(storeFreshness({ ...base, stale: true, last_error_code: "auth_error" })).toBe("failed");
    expect(storeFreshness({ ...base, last_success_at: null, never_synced: true, stale: true })).toBe("never");
  });

  it("only offers a payment ledger for rails the core can book", () => {
    const lightning = readOnlyPlan.payment_methods[1];
    expect(planRoutes(readOnlyPlan, {}, wallets)[1].choice.action).toBe("payment_ledger");
    expect(
      planRoutes(
        {
          ...readOnlyPlan,
          payment_methods: [
            { ...lightning, actions: undefined, ledger_group: null, recommendation: undefined },
          ],
        },
        { [btcpayRouteKey(CAFE, "BTC-LN")]: { action: "payment_ledger", wallet: "" } },
        wallets,
      )[0].choice.action,
    ).toBe("provenance_only");
  });

  it("recognises server URLs worth asking the core about", () => {
    expect(looksLikeServerUrl("https://pay.example.com/stores/abc")).toBe(true);
    expect(looksLikeServerUrl("http://127.0.0.1:18549")).toBe(true);
    expect(looksLikeServerUrl("pay.example.com")).toBe(false);
    expect(looksLikeServerUrl("")).toBe(false);
  });
});
