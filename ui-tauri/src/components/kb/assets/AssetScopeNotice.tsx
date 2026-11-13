import * as React from "react";
import { useTranslation } from "react-i18next";
import { Coins } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import { formatCount } from "@/lib/localeFormat";

import {
  shownTickers,
  tickerSummary,
  type AssetScope,
  type ImportAssets,
} from "./importAssets";

/** The acknowledgement a plaintext book's wallet changes ask for. */
export const PLAINTEXT_CHANGE_ACK = "CHANGE LOCAL DATA";

/**
 * What happens after the user turns other crypto assets on:
 * - `sync`: the exchange is synced again;
 * - `import`: the same file is imported again (the caller kept its path);
 * - `pick_file`: the user has to choose the file again, because the path the
 *   import read is not kept. Kassiber never re-reads a file on its own.
 */
export type AssetScopeRerun = "sync" | "import" | "pick_file";

function useTickerText() {
  const { t } = useTranslation("connections");
  return (tickers: string[]) => {
    const { shown, more } = shownTickers(tickers);
    const list = shown.join(", ");
    return more > 0 ? t("assets.tickersMore", { list, count: more }) : list;
  };
}

/**
 * The asset scope part of an exchange card or an import result
 * (docs/plan/23-crypto-assets.md): which other crypto assets the `bitcoin`
 * scope skipped, with the action that tracks them; that a `crypto` wallet
 * tracks them, with the way back; non-crypto holdings that are never
 * imported; and the swaps the import paired. Changing the scope is a wallet
 * change, so it asks for the passphrase (or, in a plaintext book, the typed
 * acknowledgement) like any other edit of the wallet.
 */
export function AssetScopeNotice({
  wallet,
  scope,
  assets,
  swapPairs,
  rerun,
  disabled,
  onScopeChanged,
}: {
  /** The wallet id (or label) `ui.wallets.update` names. */
  wallet: string;
  /** The wallet's current scope. */
  scope: AssetScope;
  /** What the last import or sync left out, when it said. */
  assets: ImportAssets | null;
  swapPairs?: number | null;
  rerun: AssetScopeRerun;
  disabled?: boolean;
  /** Runs after the scope was stored: sync again, import again, or nothing. */
  onScopeChanged?: (scope: AssetScope) => void | Promise<void>;
}) {
  const { t } = useTranslation("connections");
  const tickerText = useTickerText();
  const status = useDaemon<{ database_encrypted?: boolean }>("status");
  const encrypted = Boolean(
    status.data?.kind === "status" && status.data.data?.database_encrypted,
  );
  const updateWallet = useDaemonMutation("ui.wallets.update");
  const [confirming, setConfirming] = React.useState<AssetScope | null>(null);
  const [secret, setSecret] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [pickAgain, setPickAgain] = React.useState(false);

  const skipped = tickerSummary(assets?.skipped);
  const excluded = tickerSummary(assets?.excluded);
  const offerTracking = scope === "bitcoin" && skipped.rows > 0;
  const pairs = swapPairs ?? 0;
  if (
    !offerTracking &&
    scope !== "crypto" &&
    excluded.rows === 0 &&
    pairs <= 0 &&
    !pickAgain &&
    !error
  ) {
    return null;
  }

  const begin = (next: AssetScope) => {
    setConfirming(next);
    setSecret("");
    setError(null);
  };

  const confirm = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!confirming) return;
    if (encrypted && !secret) {
      setError(t("assets.confirm.errorPassphrase"));
      return;
    }
    if (!encrypted && secret.trim() !== PLAINTEXT_CHANGE_ACK) {
      setError(t("assets.confirm.errorAck", { ack: PLAINTEXT_CHANGE_ACK }));
      return;
    }
    setError(null);
    const next = confirming;
    try {
      await updateWallet.mutateAsync({
        wallet,
        asset_scope: next,
        auth_response: encrypted
          ? { passphrase_secret: secret }
          : { plaintext_change_ack: PLAINTEXT_CHANGE_ACK },
      });
    } catch (failure) {
      const code =
        failure instanceof DaemonRequestError ? failure.envelope.error?.code : undefined;
      setError(
        code === "local_auth_denied"
          ? t("assets.confirm.errorDenied")
          : failure instanceof Error
            ? failure.message
            : String(failure),
      );
      return;
    }
    setConfirming(null);
    setSecret("");
    setPickAgain(next === "crypto" && rerun === "pick_file");
    try {
      await onScopeChanged?.(next);
    } catch (failure) {
      // The scope is stored; only the sync or import after it failed.
      setError(failure instanceof Error ? failure.message : String(failure));
    }
  };

  return (
    <div className="space-y-2 rounded-md border border-border/60 p-3 text-xs" data-testid="asset-scope">
      {offerTracking ? (
        <div className="space-y-1">
          <p className="m-0 flex items-center gap-1.5 font-medium">
            <Coins className="size-3.5" aria-hidden="true" />
            {t("assets.found", {
              tickers: tickerText(skipped.tickers),
              count: skipped.rows,
              rows: formatCount(skipped.rows),
            })}
          </p>
          <p className="m-0 text-muted-foreground">{t("assets.foundBody")}</p>
        </div>
      ) : null}
      {scope === "crypto" ? (
        <p className="m-0 flex items-center gap-1.5 font-medium">
          <Coins className="size-3.5" aria-hidden="true" />
          {t("assets.tracked")}
        </p>
      ) : null}
      {pickAgain && scope === "crypto" ? (
        <p className="m-0 text-muted-foreground" role="status">
          {t("assets.pickAgain")}
        </p>
      ) : null}
      {excluded.rows > 0 ? (
        <p className="m-0 text-muted-foreground">
          {t("assets.excluded", {
            tickers: tickerText(excluded.tickers),
            count: excluded.rows,
            rows: formatCount(excluded.rows),
          })}
        </p>
      ) : null}
      {pairs > 0 ? (
        <p className="m-0 text-muted-foreground">
          {t("assets.swapPairs", { count: pairs, pairs: formatCount(pairs) })}
        </p>
      ) : null}

      {confirming ? (
        <form className="space-y-2 rounded-md bg-muted/40 p-2.5" onSubmit={(e) => void confirm(e)}>
          <p className="m-0 font-medium">
            {confirming === "crypto" ? t("assets.confirm.trackTitle") : t("assets.confirm.untrackTitle")}
          </p>
          <p className="m-0 text-muted-foreground">
            {confirming === "crypto"
              ? t(`assets.confirm.trackBody.${rerun}`)
              : t("assets.confirm.untrackBody")}
          </p>
          <div className="space-y-1">
            <Label htmlFor="asset-scope-secret" className="text-xs">
              {encrypted
                ? t("assets.confirm.passphrase")
                : t("assets.confirm.ack", { ack: PLAINTEXT_CHANGE_ACK })}
            </Label>
            <Input
              id="asset-scope-secret"
              type={encrypted ? "password" : "text"}
              autoComplete={encrypted ? "current-password" : "off"}
              placeholder={encrypted ? undefined : PLAINTEXT_CHANGE_ACK}
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
            />
          </div>
          {error ? (
            <p className="m-0 text-destructive" role="alert">
              {error}
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <Button type="submit" size="sm" disabled={disabled || updateWallet.isPending}>
              {confirming === "crypto"
                ? t(`assets.confirm.trackButton.${rerun}`)
                : t("assets.confirm.untrackButton")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => {
                setConfirming(null);
                setError(null);
              }}
            >
              {t("assets.confirm.cancel")}
            </Button>
          </div>
        </form>
      ) : null}
      {error && !confirming ? (
        <p className="m-0 text-destructive" role="alert">
          {error}
        </p>
      ) : null}
      {confirming ? null : offerTracking ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={disabled}
          onClick={() => begin("crypto")}
        >
          {t("assets.track")}
        </Button>
      ) : scope === "crypto" ? (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-auto px-0 text-xs underline-offset-2 hover:underline"
          disabled={disabled}
          onClick={() => begin("bitcoin")}
        >
          {t("assets.untrack")}
        </Button>
      ) : null}
    </div>
  );
}
