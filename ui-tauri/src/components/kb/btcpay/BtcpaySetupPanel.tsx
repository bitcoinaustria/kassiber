import * as React from "react";
import { useTranslation } from "react-i18next";
import { ExternalLink, KeyRound, Loader2, ShieldAlert, ShieldCheck } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDaemonMutation } from "@/daemon/client";
import { openExternalUrl } from "@/daemon/transport";
import { currentUiLocale } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import { SetupField } from "../SetupField";
import {
  BTCPAY_ROUTE_ACTIONS,
  DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
  actionAvailable,
  buildCreateRoutes,
  btcpayRouteKey,
  defaultRouteChoices,
  keyRiskTone,
  looksLikeServerUrl,
  planRoutes,
  railKind,
  settlementWalletOptions,
  storeFreshness,
  storeName,
  summarizeRoutes,
  validateBtcpaySetup,
  walletOptionsForMethod,
  type BtcpayCapability,
  type BtcpayCreateRoute,
  type BtcpayInstanceArgs,
  type BtcpayKeyGuide,
  type BtcpayKeyPreset,
  type BtcpayManualRoute,
  type BtcpayPlan,
  type BtcpayPlanMethod,
  type BtcpayPlanStore,
  type BtcpayRouteAction,
  type BtcpayRouteChoice,
  type BtcpaySetupIssue,
  type BtcpayTone,
  type BtcpayWalletOption,
} from "./btcpaySetupModel";

export interface BtcpaySavedInstance {
  name: string;
  display_name?: string;
  is_default?: boolean;
}

export type BtcpayInstanceIssue =
  | "choose_instance"
  | "instance_name"
  | "server_url"
  | "api_key";

export interface BtcpaySetupDraft {
  instance: BtcpayInstanceArgs | null;
  isNewInstance: boolean;
  plan: BtcpayPlan | null;
  routes: BtcpayCreateRoute[];
  instanceIssue: BtcpayInstanceIssue | null;
  issue: BtcpaySetupIssue | null;
}

const KEY_PRESETS: readonly BtcpayKeyPreset[] = ["read_only", "wallet_history"];
const STORE_CAPABILITIES = [
  "invoices",
  "payouts",
  "payment_requests",
  "wallet_history",
] as const satisfies readonly BtcpayCapability[];
type BtcpayStoreScope = "all" | "single";
const STORE_SCOPES: readonly BtcpayStoreScope[] = ["all", "single"];
// BTCPay's authorize page grants all stores or asks for exactly one store.
// Read-only keys default to all stores; store-modifying keys to one store.
const DEFAULT_SCOPE: Record<BtcpayKeyPreset, BtcpayStoreScope> = {
  read_only: "all",
  wallet_history: "single",
};
const SELECT_CLASS =
  "h-9 w-full rounded-md border border-input bg-background px-3 text-sm disabled:cursor-not-allowed disabled:opacity-60";

const TONE_CLASSES: Record<BtcpayTone, string> = {
  good: "border-emerald-500/40 bg-emerald-500/10 text-emerald-800 dark:text-emerald-200",
  warning: "border-amber-500/40 bg-amber-500/10 text-amber-900 dark:text-amber-200",
  danger: "border-destructive/50 bg-destructive/10 text-destructive",
  neutral: "border-border bg-muted/40 text-muted-foreground",
};

const WARNING_TONES: Record<string, BtcpayTone> = {
  key_server_admin: "danger",
  unencrypted_transport: "danger",
  key_can_modify_store: "warning",
  key_can_write: "warning",
  server_not_synced: "warning",
  store_catalog_unavailable: "warning",
  no_store_access: "warning",
  shared_store_wallet: "neutral",
  stale_store_data: "warning",
};

function formatSyncedAt(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

function instanceDisplayName(instance: BtcpaySavedInstance) {
  const label = instance.display_name?.trim() || instance.name;
  return label === instance.name ? label : `${label} (${instance.name})`;
}

function defaultSavedInstance(instances: readonly BtcpaySavedInstance[]) {
  return instances.find((instance) => instance.is_default)?.name ?? instances[0]?.name ?? "";
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

function Notice({
  tone,
  children,
  className,
}: {
  tone: BtcpayTone;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn("rounded-md border px-3 py-2 text-xs", TONE_CLASSES[tone], className)}
    >
      {children}
    </div>
  );
}

export function BtcpaySetupPanel({
  savedInstances,
  wallets,
  connectionLabel,
  showErrors,
  labelField,
  footer,
  onDraftChange,
  onUseCsvInstead,
}: {
  savedInstances: readonly BtcpaySavedInstance[];
  wallets: readonly BtcpayWalletOption[];
  connectionLabel: string;
  showErrors: boolean;
  labelField: React.ReactNode;
  footer?: React.ReactNode;
  onDraftChange: (draft: BtcpaySetupDraft) => void;
  onUseCsvInstead: () => void;
}) {
  const { t } = useTranslation("connections");
  const [instanceMode, setInstanceMode] = React.useState<"saved" | "new">(
    savedInstances.length ? "saved" : "new",
  );
  const [savedInstance, setSavedInstance] = React.useState(() =>
    defaultSavedInstance(savedInstances),
  );
  const [instanceLabel, setInstanceLabel] = React.useState("btcpay");
  const [serverUrl, setServerUrl] = React.useState("");
  const [apiKey, setApiKey] = React.useState("");
  const [preset, setPreset] = React.useState<BtcpayKeyPreset>("read_only");
  const [storeScope, setStoreScope] = React.useState<BtcpayStoreScope>("all");
  const [plan, setPlan] = React.useState<BtcpayPlan | null>(null);
  const [planError, setPlanError] = React.useState<string | null>(null);
  const [authorizeError, setAuthorizeError] = React.useState<string | null>(null);
  const [choices, setChoices] = React.useState<Record<string, BtcpayRouteChoice>>({});
  const [manualRoute, setManualRoute] = React.useState<BtcpayManualRoute>({
    storeId: "",
    paymentMethodId: DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
    action: "provenance_only",
    wallet: "",
  });
  const userChoseInstanceMode = React.useRef(false);

  const discover = useDaemonMutation<BtcpayPlan>("ui.connections.btcpay.discover", {
    invalidateQueries: false,
  });
  const keyGuide = useDaemonMutation<BtcpayKeyGuide>("ui.connections.btcpay.key_guide", {
    invalidateQueries: false,
  });

  // Saved instances load asynchronously; prefer them once they appear unless
  // the user already chose to add a new instance.
  React.useEffect(() => {
    if (userChoseInstanceMode.current) return;
    if (savedInstances.length && !serverUrl) {
      setInstanceMode("saved");
      setSavedInstance((current) => current || defaultSavedInstance(savedInstances));
    } else if (!savedInstances.length) {
      setInstanceMode("new");
    }
  }, [savedInstances, serverUrl]);

  const resetPlan = React.useCallback(() => {
    setPlan(null);
    setPlanError(null);
    setChoices({});
  }, []);

  const instanceIssue: BtcpayInstanceIssue | null =
    instanceMode === "saved"
      ? savedInstance.trim()
        ? null
        : "choose_instance"
      : !instanceLabel.trim()
        ? "instance_name"
        : !looksLikeServerUrl(serverUrl)
          ? "server_url"
          : !apiKey.trim()
            ? "api_key"
            : null;
  const instance: BtcpayInstanceArgs | null = instanceIssue
    ? null
    : instanceMode === "saved"
      ? { backend: savedInstance.trim() }
      : {
          backend_label: instanceLabel.trim(),
          server_url: serverUrl.trim(),
          api_key: apiKey.trim(),
        };

  const entries = React.useMemo(
    () => (plan ? planRoutes(plan, choices, wallets) : []),
    [choices, plan, wallets],
  );
  const effectiveManualRoute = plan && plan.payment_methods.length === 0 ? manualRoute : null;
  const issue = validateBtcpaySetup({
    plan,
    entries,
    manualRoute: effectiveManualRoute,
    wallets,
  });
  const routes = React.useMemo(
    () =>
      plan
        ? buildCreateRoutes({
            plan,
            entries,
            manualRoute: effectiveManualRoute,
            connectionLabel,
          })
        : [],
    [connectionLabel, effectiveManualRoute, entries, plan],
  );

  React.useEffect(() => {
    onDraftChange({
      instance,
      isNewInstance: instanceMode === "new",
      plan,
      routes,
      instanceIssue,
      issue,
    });
    // `instance` is rebuilt every render; its inputs are listed instead.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiKey, instanceIssue, instanceLabel, instanceMode, issue, onDraftChange, plan, routes, savedInstance, serverUrl]);

  const runDiscovery = async () => {
    if (!instance) return;
    setPlanError(null);
    try {
      const envelope = await discover.mutateAsync(instance as Record<string, unknown>);
      const nextPlan = envelope.data ?? null;
      setPlan(nextPlan);
      setChoices(nextPlan ? defaultRouteChoices(nextPlan, wallets) : {});
    } catch (error) {
      setPlan(null);
      setPlanError(errorMessage(error, t("add.btcpay.discoveryFailed")));
    }
  };

  const canRequestKeyLink =
    instanceMode === "saved" ? Boolean(savedInstance) : looksLikeServerUrl(serverUrl);
  const openAuthorize = async (targetPreset: BtcpayKeyPreset) => {
    setAuthorizeError(null);
    try {
      const envelope = await keyGuide.mutateAsync(
        instanceMode === "saved"
          ? { backend: savedInstance, preset: targetPreset, store_scope: storeScope }
          : { server_url: serverUrl.trim(), preset: targetPreset, store_scope: storeScope },
      );
      const url = envelope.data?.authorize_url;
      if (!url) throw new Error(t("add.btcpay.authorizeFailed"));
      await openExternalUrl(url);
    } catch (error) {
      setAuthorizeError(errorMessage(error, t("add.btcpay.authorizeFailed")));
    }
  };

  const setChoice = (key: string, next: Partial<BtcpayRouteChoice>, fallback: BtcpayRouteChoice) => {
    setChoices((current) => ({
      ...current,
      [key]: { ...(current[key] ?? fallback), ...next },
    }));
  };

  const instanceError = (target: BtcpayInstanceIssue) =>
    showErrors && instanceIssue === target
      ? t(
          target === "choose_instance"
            ? "add.btcpay.errorChooseInstance"
            : target === "instance_name"
              ? "add.btcpay.errorInstanceName"
              : target === "server_url"
                ? "add.btcpay.errorServerUrl"
                : "add.btcpay.errorApiKey",
        )
      : undefined;

  const summary = summarizeRoutes(entries);
  const lightningWalletNames = (method: BtcpayPlanMethod) =>
    (method.lightning_wallets ?? []).map((wallet) => wallet.wallet).join(", ");
  const settlementWallets = settlementWalletOptions(wallets);

  const renderStoreFreshness = (store: BtcpayPlanStore) => {
    const freshness = storeFreshness(store.sync_state);
    if (!freshness) return null;
    const syncedAt = store.sync_state?.last_success_at;
    const text =
      freshness === "never"
        ? t("add.btcpay.storeSync.never")
        : freshness === "failed"
          ? t("add.btcpay.storeSync.failed")
          : freshness === "stale"
            ? t("add.btcpay.storeSync.stale")
            : t("add.btcpay.storeSync.at", { when: syncedAt ? formatSyncedAt(syncedAt) : "" });
    return (
      <p
        className={cn(
          "text-[11px]",
          freshness === "fresh" ? "text-muted-foreground" : "text-amber-700 dark:text-amber-300",
        )}
        title={syncedAt ? formatSyncedAt(syncedAt) : undefined}
      >
        {text}
      </p>
    );
  };

  // Lightning and LNURL of one store (or one plugin rail) share a ledger:
  // explain it once, on the first method of each group.
  const ledgerGroups = new Map<string, { first: string; labels: string[] }>();
  for (const entry of entries) {
    if (entry.choice.action !== "payment_ledger") continue;
    const groupKey = `${entry.method.store_id}\u0000${entry.method.ledger_group ?? entry.method.payment_method_id}`;
    const group = ledgerGroups.get(groupKey);
    if (group) group.labels.push(entry.method.label);
    else ledgerGroups.set(groupKey, { first: entry.key, labels: [entry.method.label] });
  }
  const ledgerGroupFor = (method: BtcpayPlanMethod) =>
    ledgerGroups.get(`${method.store_id}\u0000${method.ledger_group ?? method.payment_method_id}`);

  const renderMethod = (method: BtcpayPlanMethod) => {
    const key = btcpayRouteKey(method.store_id, method.payment_method_id);
    const entry = entries.find((candidate) => candidate.key === key);
    const choice = entry?.choice ?? { action: "skip" as const, wallet: "" };
    const store = storeName(plan, method.store_id);
    const walletOptions = walletOptionsForMethod(method, wallets);
    const recommendation = method.recommendation;
    const rail = railKind(method);
    const sharedStores = (method.shared_with ?? [])
      .map((member) => storeName(plan, member.store_id))
      .filter((name, index, all) => all.indexOf(name) === index);
    const reasonKey = recommendation?.reason ? `add.btcpay.reason.${recommendation.reason}` : null;
    // The "Configured" badge already says a route exists.
    const reasonText =
      reasonKey && recommendation?.reason !== "already_configured"
        ? t(reasonKey as "add.btcpay.reason.already_configured", {
            defaultValue: recommendation?.reason_text ?? "",
          })
        : null;
    const ledgerGroup = choice.action === "payment_ledger" ? ledgerGroupFor(method) : undefined;
    return (
      <div
        key={key}
        className={cn(
          // Stack on a narrow pane so the action never truncates.
          "grid gap-3 rounded-md border border-border/60 bg-background/70 p-3 text-sm @xl:grid-cols-[minmax(0,1fr)_minmax(220px,0.75fr)]",
          choice.action === "skip" && "opacity-70",
        )}
      >
        <div className="min-w-0 space-y-1.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{method.label}</span>
            <Badge variant="outline">{t(`add.btcpay.rail.${rail}`)}</Badge>
            <Badge variant="outline" className="font-mono text-[11px]">
              {method.payment_method_id}
            </Badge>
            {(method.existing_routes ?? []).length ? (
              <Badge variant="secondary">{t("add.btcpay.notice.alreadyConfigured")}</Badge>
            ) : null}
          </div>
          {method.settlement ? (
            <p className="text-xs text-muted-foreground">
              {t(`add.btcpay.settlement.${method.settlement}` as "add.btcpay.settlement.plugin", {
                defaultValue: method.settlement_hint ?? "",
              })}
            </p>
          ) : null}
          {reasonText ? (
            <p className="text-xs">
              <span className="font-medium">{t("add.btcpay.recommended")}:</span>{" "}
              {reasonText}
            </p>
          ) : null}
          {method.owned_by ? (
            <Notice tone="good">
              {t("add.btcpay.notice.recognised", { wallet: method.owned_by.wallet })}
            </Notice>
          ) : null}
          {sharedStores.length ? (
            <Notice tone="neutral">
              {t("add.btcpay.notice.sharedWith", { stores: sharedStores.join(", ") })}
            </Notice>
          ) : null}
          {(method.configured_via ?? []).length ? (
            <Notice tone="neutral">
              {t("add.btcpay.notice.configuredVia", {
                names: (method.configured_via ?? []).join(", "),
              })}
            </Notice>
          ) : null}
          {(method.lightning_wallets ?? []).length ? (
            <Notice tone="neutral">
              {t("add.btcpay.notice.lightningLinks", { wallets: lightningWalletNames(method) })}
            </Notice>
          ) : null}
        </div>
        <div className="space-y-2">
          <select
            aria-label={t("add.btcpay.actionLabel", {
              method: method.payment_method_id,
              store,
            })}
            className={SELECT_CLASS}
            value={choice.action}
            onChange={(event) =>
              setChoice(
                key,
                { action: event.target.value as BtcpayRouteAction },
                choice,
              )
            }
          >
            {BTCPAY_ROUTE_ACTIONS.map((action) => {
              const available = actionAvailable(method, action);
              const unavailableReason = method.actions?.[action]?.reason;
              return (
                <option
                  key={action}
                  value={action}
                  disabled={!available}
                  title={
                    !available && unavailableReason
                      ? t(`add.btcpay.reason.${unavailableReason}` as "add.btcpay.reason.not_on_chain", {
                          defaultValue: method.actions?.[action]?.reason_text ?? "",
                        })
                      : undefined
                  }
                >
                  {t(`add.btcpay.action.${action}`)}
                </option>
              );
            })}
          </select>
          {ledgerGroup && ledgerGroup.first === key ? (
            <p className="text-xs text-muted-foreground">
              {ledgerGroup.labels.length > 1
                ? `${t("add.btcpay.notice.ledgerShared", {
                    methods: new Intl.ListFormat(currentUiLocale(), { type: "conjunction" }).format(
                      ledgerGroup.labels,
                    ),
                  })} `
                : ""}
              {t("add.btcpay.notice.ledgerScope")}
            </p>
          ) : ledgerGroup ? (
            <p className="text-xs text-muted-foreground">
              {t("add.btcpay.notice.ledgerSameAs", { method: ledgerGroup.labels[0] })}
            </p>
          ) : null}
          {choice.action === "existing_wallet" ? (
            <select
              aria-label={t("add.btcpay.walletLabel", {
                method: method.payment_method_id,
                store,
              })}
              className={SELECT_CLASS}
              value={choice.wallet}
              disabled={walletOptions.length === 0}
              onChange={(event) => setChoice(key, { wallet: event.target.value }, choice)}
            >
              <option value="" disabled={walletOptions.length > 0}>
                {walletOptions.length ? t("add.btcpay.selectWallet") : t("add.btcpay.noWallets")}
              </option>
              {walletOptions.map((wallet) => (
                <option key={wallet.label} value={wallet.label}>
                  {wallet.label}
                  {wallet.chain ? ` (${wallet.chain})` : ""}
                </option>
              ))}
            </select>
          ) : null}
        </div>
      </div>
    );
  };

  const apiKeyInfo = plan?.api_key;
  const risk = apiKeyInfo?.risk ?? "unknown";
  const riskTone = keyRiskTone(risk);
  const transport = plan?.server
    ? plan.server.tor
      ? "tor"
      : plan.server.loopback
        ? "loopback"
        : plan.server.transport === "https"
          ? "https"
          : "http"
    : null;
  const upgradeUrl = plan?.key_upgrade?.authorize_url;
  const keyIsReadOnly = apiKeyInfo?.known && risk === "read_only";
  const presetPermissions = (value: BtcpayKeyPreset) =>
    value === "read_only"
      ? ["btcpay.store.canviewstoresettings"]
      : ["btcpay.store.canmodifystoresettings"];

  return (
    <>
      <div className="space-y-1 rounded-md border border-border/70 bg-muted/20 p-3">
        <p className="text-sm font-medium">{t("add.btcpay.introTitle")}</p>
        <p className="text-xs text-muted-foreground">{t("add.btcpay.introBody")}</p>
      </div>
      {labelField}
      <div className="grid grid-cols-2 gap-2">
        <Button
          type="button"
          variant={instanceMode === "saved" ? "secondary" : "outline"}
          disabled={savedInstances.length === 0}
          onClick={() => {
            userChoseInstanceMode.current = true;
            setInstanceMode("saved");
            setSavedInstance((current) => current || defaultSavedInstance(savedInstances));
            resetPlan();
          }}
        >
          {t("add.btcpay.savedInstance")}
        </Button>
        <Button
          type="button"
          variant={instanceMode === "new" ? "secondary" : "outline"}
          onClick={() => {
            userChoseInstanceMode.current = true;
            setInstanceMode("new");
            resetPlan();
          }}
        >
          {t("add.btcpay.newInstance")}
        </Button>
      </div>
      {instanceMode === "saved" ? (
        <SetupField
          id="connection-btcpay-instance"
          label={t("add.btcpay.instance")}
          error={instanceError("choose_instance")}
        >
          <select
            id="connection-btcpay-instance"
            className={SELECT_CLASS}
            value={savedInstance}
            onChange={(event) => {
              setSavedInstance(event.target.value);
              resetPlan();
            }}
          >
            <option value="" disabled>
              {t("add.btcpay.selectInstance")}
            </option>
            {savedInstances.map((saved) => (
              <option key={saved.name} value={saved.name}>
                {instanceDisplayName(saved)}
              </option>
            ))}
          </select>
        </SetupField>
      ) : (
        <>
          <SetupField
            id="connection-btcpay-instance-label"
            label={t("add.btcpay.instanceName")}
            error={instanceError("instance_name")}
            helper={t("add.btcpay.instanceNameHelper")}
          >
            <Input
              id="connection-btcpay-instance-label"
              value={instanceLabel}
              onChange={(event) => {
                setInstanceLabel(event.target.value);
                resetPlan();
              }}
            />
          </SetupField>
          <SetupField
            id="connection-btcpay-url"
            label={t("add.btcpay.serverUrl")}
            error={instanceError("server_url")}
            helper={t("add.btcpay.serverUrlHelper")}
          >
            <Input
              id="connection-btcpay-url"
              inputMode="url"
              value={serverUrl}
              placeholder={t("add.btcpay.serverUrlPlaceholder")}
              onChange={(event) => {
                setServerUrl(event.target.value);
                resetPlan();
              }}
            />
          </SetupField>
          <div className="space-y-3 rounded-md border border-border/70 p-3">
            <div className="space-y-1">
              <p className="flex items-center gap-2 text-sm font-medium">
                <KeyRound className="size-4" aria-hidden="true" />
                {t("add.btcpay.keyStepTitle")}
              </p>
              <p className="text-xs text-muted-foreground">{t("add.btcpay.keyStepBody")}</p>
            </div>
            <div role="radiogroup" aria-label={t("add.btcpay.keyStepTitle")} className="grid gap-2 md:grid-cols-2">
              {KEY_PRESETS.map((value) => (
                <button
                  key={value}
                  type="button"
                  role="radio"
                  aria-checked={preset === value}
                  onClick={() => {
                    setPreset(value);
                    setStoreScope(DEFAULT_SCOPE[value]);
                  }}
                  className={cn(
                    "space-y-1 rounded-md border p-3 text-left text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    preset === value
                      ? "border-primary bg-primary/5"
                      : "border-border/70 hover:bg-muted/40",
                  )}
                >
                  <span className="flex items-center gap-1.5 text-sm font-medium">
                    {value === "read_only" ? (
                      <ShieldCheck className="size-4 text-emerald-600" aria-hidden="true" />
                    ) : (
                      <ShieldAlert className="size-4 text-amber-600" aria-hidden="true" />
                    )}
                    {t(`add.btcpay.presets.${value}.title`)}
                  </span>
                  <span className="block text-muted-foreground">
                    {t(`add.btcpay.presets.${value}.body`)}
                  </span>
                  <span className="block font-mono text-[11px] text-muted-foreground">
                    {presetPermissions(value).join(", ")}
                  </span>
                </button>
              ))}
            </div>
            <div className="space-y-1.5">
              <p className="text-xs font-medium">{t("add.btcpay.scope.label")}</p>
              <div role="radiogroup" aria-label={t("add.btcpay.scope.label")} className="grid gap-2 md:grid-cols-2">
                {STORE_SCOPES.map((scope) => (
                  <button
                    key={scope}
                    type="button"
                    role="radio"
                    aria-checked={storeScope === scope}
                    onClick={() => setStoreScope(scope)}
                    className={cn(
                      "rounded-md border px-3 py-2 text-left text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                      storeScope === scope ? "border-primary bg-primary/5" : "border-border/70 hover:bg-muted/40",
                    )}
                  >
                    <span className="block font-medium">{t(`add.btcpay.scope.${scope}.title`)}</span>
                    <span className="block text-muted-foreground">{t(`add.btcpay.scope.${scope}.body`)}</span>
                  </button>
                ))}
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={!canRequestKeyLink || keyGuide.isPending}
                onClick={() => void openAuthorize(preset)}
              >
                {keyGuide.isPending ? (
                  <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                ) : (
                  <ExternalLink className="size-3.5" aria-hidden="true" />
                )}
                {keyGuide.isPending ? t("add.btcpay.openingAuthorize") : t("add.btcpay.openAuthorize")}
              </Button>
              <span className="text-xs text-muted-foreground">
                {canRequestKeyLink
                  ? storeScope === "all"
                    ? t("add.btcpay.authorizeHintAll")
                    : t("add.btcpay.authorizeHintSingle")
                  : t("add.btcpay.authorizeNeedsUrl")}
              </span>
            </div>
            {authorizeError ? <p className="text-xs text-destructive">{authorizeError}</p> : null}
            <details className="text-xs">
              <summary className="cursor-pointer select-none text-muted-foreground hover:text-foreground">
                {t("add.btcpay.manualToggle")}
              </summary>
              <ol className="mt-2 list-decimal space-y-1 pl-5 text-muted-foreground">
                <li>{t("add.btcpay.manualSteps.signIn")}</li>
                <li>{t("add.btcpay.manualSteps.openKeys")}</li>
                <li>
                  {t("add.btcpay.manualSteps.permissions", {
                    permissions: presetPermissions(preset).join(", "),
                  })}
                </li>
                <li>
                  {storeScope === "all"
                    ? t("add.btcpay.manualSteps.storesAll")
                    : t("add.btcpay.manualSteps.storesSingle")}
                </li>
                <li>{t("add.btcpay.manualSteps.copy")}</li>
              </ol>
            </details>
          </div>
          <SetupField
            id="connection-btcpay-api-key"
            label={t("add.btcpay.apiKey")}
            error={instanceError("api_key")}
            helper={t("add.btcpay.apiKeyHelper")}
          >
            <Input
              id="connection-btcpay-api-key"
              type="password"
              autoComplete="off"
              value={apiKey}
              onChange={(event) => {
                setApiKey(event.target.value);
                resetPlan();
              }}
            />
          </SetupField>
        </>
      )}
      <div className="space-y-2">
        <p className="text-sm font-medium">{t("add.btcpay.discoverStepTitle")}</p>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant={plan ? "outline" : "secondary"}
            disabled={!instance || discover.isPending}
            onClick={() => void runDiscovery()}
          >
            {discover.isPending ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : null}
            {discover.isPending ? t("add.btcpay.checking") : t("add.btcpay.checkKey")}
          </Button>
          {plan ? (
            <span className="text-xs text-muted-foreground" aria-live="polite">
              {t("add.btcpay.storesFound", {
                count: plan.stores.length,
                methods: plan.payment_methods.length,
              })}
            </span>
          ) : null}
        </div>
        {planError ? (
          <p className="text-xs text-destructive" role="alert">
            {planError}
          </p>
        ) : null}
        {showErrors && issue === "discover_first" && !planError ? (
          <p className="text-xs text-destructive">{t("add.btcpay.issue.discover_first")}</p>
        ) : null}
      </div>
      {plan ? (
        <div className="space-y-3">
          <div className="grid gap-2 md:grid-cols-2">
            <div className="space-y-1 rounded-md border border-border/70 p-3 text-xs">
              <p className="text-sm font-medium">
                {plan.server?.version
                  ? t("add.btcpay.server.version", { version: plan.server.version })
                  : t("add.btcpay.server.versionUnknown")}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {plan.server?.fully_synced === false ? (
                  <Badge variant="outline" className={TONE_CLASSES.warning}>
                    {t("add.btcpay.server.notSynced")}
                  </Badge>
                ) : plan.server?.fully_synced ? (
                  <Badge variant="outline">{t("add.btcpay.server.synced")}</Badge>
                ) : null}
                {transport ? (
                  <Badge
                    variant="outline"
                    className={transport === "http" ? TONE_CLASSES.danger : undefined}
                  >
                    {t(`add.btcpay.server.transport.${transport}`)}
                  </Badge>
                ) : null}
              </div>
            </div>
            <div className="space-y-1 rounded-md border border-border/70 p-3 text-xs">
              <p className="text-sm font-medium">
                {apiKeyInfo?.label
                  ? t("add.btcpay.key.labelled", { label: apiKeyInfo.label })
                  : t("add.btcpay.key.title")}
              </p>
              <div className="flex flex-wrap gap-1.5">
                <Badge variant="outline" className={TONE_CLASSES[riskTone]}>
                  {t(`add.btcpay.key.risk.${risk}` as "add.btcpay.key.risk.unknown", {
                    defaultValue: risk,
                  })}
                </Badge>
                <Badge variant="outline">
                  {apiKeyInfo?.scope === "selected_stores"
                    ? t("add.btcpay.key.scope.selected_stores", {
                        count: apiKeyInfo.store_ids?.length ?? 0,
                      })
                    : t(`add.btcpay.key.scope.${apiKeyInfo?.scope ?? "unknown"}` as "add.btcpay.key.scope.unknown", {
                        defaultValue: apiKeyInfo?.scope ?? "",
                      })}
                </Badge>
              </div>
              {(apiKeyInfo?.excess_permissions ?? []).length ? (
                <p className="text-muted-foreground">
                  {t("add.btcpay.key.excess", {
                    permissions: (apiKeyInfo?.excess_permissions ?? []).join(", "),
                  })}
                </p>
              ) : null}
            </div>
          </div>
          {keyIsReadOnly ? (
            <Notice tone="neutral" className="space-y-1">
              <p>{t("add.btcpay.key.readOnlyExplainer")}</p>
              {upgradeUrl ? (
                <button
                  type="button"
                  className="inline-flex items-center gap-1 underline underline-offset-2"
                  onClick={() => {
                    void openExternalUrl(upgradeUrl).catch((error) =>
                      setAuthorizeError(errorMessage(error, t("add.btcpay.authorizeFailed"))),
                    );
                  }}
                >
                  <ExternalLink className="size-3" aria-hidden="true" />
                  {t("add.btcpay.key.upgrade")}
                </button>
              ) : null}
            </Notice>
          ) : null}
          {(plan.sibling_backends ?? []).length ? (
            <Notice tone="neutral">
              {t("add.btcpay.key.siblings", { names: (plan.sibling_backends ?? []).join(", ") })}
            </Notice>
          ) : null}
          {(plan.warnings ?? []).map((warning, index) => (
            <Notice
              key={`${warning.code}-${warning.store_id ?? warning.wallet_fingerprint ?? index}`}
              tone={WARNING_TONES[warning.code] ?? "neutral"}
            >
              {t(`add.btcpay.warning.${warning.code}` as "add.btcpay.warning.no_store_access", {
                store: warning.store_id ? storeName(plan, warning.store_id) : "",
                count: warning.store_ids?.length ?? 1,
                defaultValue: warning.message,
              })}
            </Notice>
          ))}
          <div className="space-y-1">
            <p className="text-sm font-medium">{t("add.btcpay.storesTitle")}</p>
            <p className="text-xs text-muted-foreground">{t("add.btcpay.storesBody")}</p>
          </div>
          <Notice tone="neutral">{t("add.btcpay.staleNote")}</Notice>
          <div className="@container space-y-3">
          {plan.stores.map((store) => {
            const methods = plan.payment_methods.filter((method) => method.store_id === store.id);
            const missing = STORE_CAPABILITIES.filter(
              (capability) => store.capabilities?.[capability] === false,
            );
            return (
              <div key={store.id} className="space-y-2 rounded-md border border-border/70 p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{store.name}</p>
                    <p className="truncate font-mono text-[11px] text-muted-foreground">{store.id}</p>
                    {renderStoreFreshness(store)}
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {STORE_CAPABILITIES.map((capability) => {
                      const value = store.capabilities?.[capability];
                      if (value === undefined || value === null) return null;
                      return (
                        <Badge
                          key={capability}
                          variant="outline"
                          className={cn(!value && "line-through opacity-60")}
                          title={value ? undefined : t("add.btcpay.capabilityMissing")}
                        >
                          {t(`add.btcpay.capability.${capability}`)}
                        </Badge>
                      );
                    })}
                  </div>
                </div>
                {missing.length && store.capabilities?.invoices === false ? (
                  <Notice tone="warning">{t("add.btcpay.reason.invoices_permission_missing")}</Notice>
                ) : null}
                {methods.length ? (
                  <div className="space-y-2">{methods.map(renderMethod)}</div>
                ) : (
                  <p className="text-xs text-muted-foreground">{t("add.btcpay.noMethods")}</p>
                )}
              </div>
            );
          })}
          </div>
          {plan.payment_methods.length === 0 ? (
            <div className="space-y-3 rounded-md border border-dashed border-border/70 p-3">
              <div className="space-y-1">
                <p className="text-sm font-medium">{t("add.btcpay.manualTitle")}</p>
                <p className="text-xs text-muted-foreground">{t("add.btcpay.manualBody")}</p>
              </div>
              <div className="grid gap-3 md:grid-cols-2">
                <SetupField
                  id="connection-btcpay-store"
                  label={t("add.btcpay.storeId")}
                  error={
                    showErrors && issue === "manual_store_id"
                      ? t("add.btcpay.issue.manual_store_id")
                      : undefined
                  }
                >
                  <Input
                    id="connection-btcpay-store"
                    list={plan.stores.length ? "connection-btcpay-store-options" : undefined}
                    value={manualRoute.storeId}
                    onChange={(event) =>
                      setManualRoute((current) => ({ ...current, storeId: event.target.value }))
                    }
                  />
                  {plan.stores.length ? (
                    <datalist id="connection-btcpay-store-options">
                      {plan.stores.map((store) => (
                        <option key={store.id} value={store.id}>
                          {store.name}
                        </option>
                      ))}
                    </datalist>
                  ) : null}
                </SetupField>
                <SetupField id="connection-btcpay-payment-method" label={t("add.btcpay.paymentMethodId")}>
                  <Input
                    id="connection-btcpay-payment-method"
                    value={manualRoute.paymentMethodId}
                    placeholder={DEFAULT_BTCPAY_PAYMENT_METHOD_ID}
                    onChange={(event) =>
                      setManualRoute((current) => ({
                        ...current,
                        paymentMethodId: event.target.value,
                      }))
                    }
                  />
                </SetupField>
              </div>
              <div className="grid gap-3 md:grid-cols-2">
                <select
                  aria-label={t("add.btcpay.actionLabel", {
                    method: manualRoute.paymentMethodId || DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
                    store: manualRoute.storeId || t("add.btcpay.storeId"),
                  })}
                  className={SELECT_CLASS}
                  value={manualRoute.action}
                  onChange={(event) =>
                    setManualRoute((current) => ({
                      ...current,
                      action: event.target.value as BtcpayRouteAction,
                    }))
                  }
                >
                  {BTCPAY_ROUTE_ACTIONS.filter((action) => action !== "skip").map((action) => (
                    <option key={action} value={action}>
                      {t(`add.btcpay.action.${action}`)}
                    </option>
                  ))}
                </select>
                {manualRoute.action === "existing_wallet" ? (
                  <select
                    aria-label={t("add.btcpay.walletLabel", {
                      method: manualRoute.paymentMethodId || DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
                      store: manualRoute.storeId || t("add.btcpay.storeId"),
                    })}
                    className={SELECT_CLASS}
                    value={manualRoute.wallet}
                    disabled={settlementWallets.length === 0}
                    onChange={(event) =>
                      setManualRoute((current) => ({ ...current, wallet: event.target.value }))
                    }
                  >
                    <option value="" disabled={settlementWallets.length > 0}>
                      {settlementWallets.length
                        ? t("add.btcpay.selectWallet")
                        : t("add.btcpay.noWallets")}
                    </option>
                    {settlementWallets.map((wallet) => (
                      <option key={wallet.label} value={wallet.label}>
                        {wallet.label}
                      </option>
                    ))}
                  </select>
                ) : null}
              </div>
            </div>
          ) : (
            <div className="space-y-1 rounded-md border border-border/70 bg-muted/20 p-3 text-xs">
              <p className="text-sm font-medium">{t("add.btcpay.summary.title")}</p>
              <ul className="space-y-0.5 text-muted-foreground">
                {BTCPAY_ROUTE_ACTIONS.filter((action) => summary[action] > 0).map((action) => (
                  <li key={action}>{t(`add.btcpay.summary.${action}`, { count: summary[action] })}</li>
                ))}
              </ul>
            </div>
          )}
          {showErrors && issue && issue !== "discover_first" && issue !== "manual_store_id" ? (
            <p className="text-xs text-destructive" role="alert">
              {t(`add.btcpay.issue.${issue}`)}
            </p>
          ) : null}
        </div>
      ) : null}
      <div className="flex justify-end">
        <Button type="button" variant="ghost" size="sm" onClick={onUseCsvInstead}>
          {t("add.btcpay.manualCsvAlternative")}
        </Button>
      </div>
      {footer}
    </>
  );
}
