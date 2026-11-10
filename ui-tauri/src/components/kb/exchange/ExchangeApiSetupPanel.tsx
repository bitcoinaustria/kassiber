import * as React from "react";
import { useTranslation } from "react-i18next";
import { ExternalLink, KeyRound, Loader2, ShieldAlert, ShieldCheck } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDaemonMutation } from "@/daemon/client";
import { openExternalUrl } from "@/daemon/transport";
import { cn } from "@/lib/utils";

import { SetupField } from "../SetupField";
import { ExchangeErrorNotice } from "./ExchangeErrorNotice";
import {
  EXCHANGE_KEY_GUIDES,
  EXCHANGE_PROVIDER_NAMES,
  credentialArgs,
  credentialIssue,
  exchangeErrorInfo,
  exchangeNeedsSecret,
  sameCredentials,
  type ExchangeCredentials,
  type ExchangeErrorInfo,
  type ExchangeKind,
  type ExchangeSetupDraft,
  type ExchangeTestResult,
} from "./exchangeApiModel";

// Static literal keys so the typed `t()` resolves them.
const GUIDE_KEYS = {
  kraken: "exchange.guide.kraken",
  coinbase: "exchange.guide.coinbase",
  binance: "exchange.guide.binance",
  bitpanda: "exchange.guide.bitpanda",
} as const satisfies Record<ExchangeKind, string>;
const SECRET_LABEL_KEYS = {
  kraken: "exchange.field.secret.kraken",
  coinbase: "exchange.field.secret.coinbase",
  binance: "exchange.field.secret.binance",
  bitpanda: "exchange.field.secret.coinbase",
} as const satisfies Record<ExchangeKind, string>;

interface TestOutcome {
  credentials: ExchangeCredentials;
  result: ExchangeTestResult | null;
  error: ExchangeErrorInfo | null;
}

/**
 * Exchange API setup: the read-only key guide, the pasted key and secret, and
 * Test connection, the first and only request before the connection is saved.
 * The credentials stay in this form until the dialog saves or closes; the
 * daemon never sends them back.
 */
export function ExchangeApiSetupPanel({
  exchange,
  labelField,
  footer,
  showErrors,
  onDraftChange,
}: {
  exchange: ExchangeKind;
  labelField: React.ReactNode;
  footer?: React.ReactNode;
  showErrors: boolean;
  onDraftChange: (draft: ExchangeSetupDraft) => void;
}) {
  const { t } = useTranslation("connections");
  const provider = EXCHANGE_PROVIDER_NAMES[exchange];
  const guide = EXCHANGE_KEY_GUIDES[exchange];
  const needsSecret = exchangeNeedsSecret(exchange);
  const [apiKey, setApiKey] = React.useState("");
  const [apiSecret, setApiSecret] = React.useState("");
  const [outcome, setOutcome] = React.useState<TestOutcome | null>(null);
  const [linkError, setLinkError] = React.useState<string | null>(null);
  const testConnection = useDaemonMutation<ExchangeTestResult>(
    "ui.connections.exchange.test",
    { invalidateQueries: false },
  );

  const credentials = React.useMemo<ExchangeCredentials>(
    () => ({ exchange, apiKey, apiSecret }),
    [exchange, apiKey, apiSecret],
  );
  const issue = credentialIssue(credentials);
  const tested =
    Boolean(outcome?.result?.ok) && sameCredentials(credentials, outcome?.credentials ?? null);
  const stale = Boolean(outcome?.result?.ok) && !tested;

  React.useEffect(() => {
    onDraftChange({
      credentials,
      tested,
      issue: issue ?? (tested ? null : "test_first"),
    });
  }, [credentials, issue, onDraftChange, tested]);

  const runTest = async () => {
    if (issue) return;
    const sent = { ...credentials };
    try {
      const envelope = await testConnection.mutateAsync(credentialArgs(sent));
      setOutcome({ credentials: sent, result: envelope.data ?? null, error: null });
    } catch (error) {
      setOutcome({ credentials: sent, result: null, error: exchangeErrorInfo(error) });
    }
  };

  const fieldError = (field: "api_key" | "api_secret") =>
    showErrors && issue === field ? t(`exchange.issue.${field}`) : undefined;

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        {t("exchange.intro", { provider })}
      </p>
      {labelField}

      <section
        aria-labelledby="exchange-key-guide-title"
        className="space-y-2 rounded-md border bg-muted/30 px-3 py-3 text-sm"
      >
        <h3 id="exchange-key-guide-title" className="flex items-center gap-2 font-medium">
          <KeyRound className="size-4" aria-hidden="true" />
          {t("exchange.guide.title")}
        </h3>
        <p className="text-xs text-muted-foreground">{t(GUIDE_KEYS[exchange])}</p>
        <div className="grid gap-2 text-xs sm:grid-cols-2">
          <div>
            <div className="font-medium">{t("exchange.guide.grant")}</div>
            <ul className="mt-1 list-disc pl-4" aria-label={t("exchange.guide.grant")}>
              {guide.grant.map((scope) => (
                <li key={scope}>
                  <code>{scope}</code>
                </li>
              ))}
            </ul>
          </div>
          <div>
            <div className="font-medium">{t("exchange.guide.deny")}</div>
            <ul className="mt-1 list-disc pl-4" aria-label={t("exchange.guide.deny")}>
              {guide.deny.map((scope) => (
                <li key={scope}>
                  <code>{scope}</code>
                </li>
              ))}
            </ul>
          </div>
        </div>
        <button
          type="button"
          className="inline-flex items-center gap-1 text-xs underline underline-offset-2"
          onClick={() => {
            setLinkError(null);
            void openExternalUrl(guide.keyPageUrl).catch((error) =>
              setLinkError(error instanceof Error ? error.message : String(error)),
            );
          }}
        >
          <ExternalLink className="size-3" aria-hidden="true" />
          {t("exchange.guide.open", { provider })}
        </button>
        {linkError ? <p className="text-xs text-destructive">{linkError}</p> : null}
      </section>

      <SetupField
        id="connection-exchange-api-key"
        label={t("exchange.field.apiKey")}
        error={fieldError("api_key")}
      >
        <Input
          id="connection-exchange-api-key"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={apiKey}
          onChange={(event) => setApiKey(event.target.value)}
        />
      </SetupField>
      {needsSecret ? (
        <SetupField
          id="connection-exchange-api-secret"
          label={t(SECRET_LABEL_KEYS[exchange])}
          error={fieldError("api_secret")}
        >
          <Input
            id="connection-exchange-api-secret"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={apiSecret}
            onChange={(event) => setApiSecret(event.target.value)}
          />
        </SetupField>
      ) : null}
      <p className="text-xs text-muted-foreground">{t("exchange.field.stored")}</p>

      <div className="space-y-2">
        <p className="text-xs text-muted-foreground">
          {t("exchange.test.explain", { provider })}
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={Boolean(issue) || testConnection.isPending}
          aria-busy={testConnection.isPending}
          onClick={() => void runTest()}
        >
          {testConnection.isPending ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : null}
          {testConnection.isPending ? t("exchange.test.pending") : t("exchange.test.button")}
        </Button>
        {tested && outcome?.result ? (
          <div
            role="status"
            className={cn(
              "flex items-start gap-2 rounded-md border px-3 py-2 text-xs",
              outcome.result.read_only
                ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-800 dark:text-emerald-200"
                : "border-amber-500/40 bg-amber-500/10 text-amber-900 dark:text-amber-200",
            )}
          >
            {outcome.result.read_only ? (
              <ShieldCheck className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            ) : (
              <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            )}
            <span>
              {outcome.result.read_only
                ? t("exchange.test.okReadOnly", { provider })
                : t("exchange.test.okUnknown", { provider })}
            </span>
          </div>
        ) : null}
        {stale ? (
          <p role="status" className="text-xs text-amber-700 dark:text-amber-300">
            {t("exchange.test.changed")}
          </p>
        ) : null}
        {outcome?.error && sameCredentials(credentials, outcome.credentials) ? (
          <ExchangeErrorNotice provider={provider} error={outcome.error} />
        ) : null}
        {showErrors && !issue && !tested ? (
          <p className="text-xs text-destructive">{t("exchange.issue.test_first")}</p>
        ) : null}
      </div>
      {footer}
    </div>
  );
}
