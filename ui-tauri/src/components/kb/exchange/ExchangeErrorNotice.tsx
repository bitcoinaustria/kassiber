import { useTranslation } from "react-i18next";

import {
  exchangeErrorKind,
  refusedPermissions,
  type ExchangeErrorInfo,
  type ExchangeErrorKind,
} from "./exchangeApiModel";

// Static literal keys so the typed `t()` resolves them.
const ERROR_KEYS = {
  auth: "exchange.error.auth",
  rateLimited: "exchange.error.rateLimited",
  offline: "exchange.error.offline",
  consent: "exchange.error.consent",
  protocol: "exchange.error.protocol",
  refused: "exchange.error.refused",
  config: "exchange.error.config",
  timeout: "exchange.error.timeout",
  network: "exchange.error.network",
  http: "exchange.error.http",
  conflict: "exchange.error.conflict",
  needsRust: "exchange.error.needsRust",
  unknown: "exchange.error.unknown",
} as const satisfies Record<ExchangeErrorKind, string>;

/**
 * A failed exchange request, explained: what the error code means for the
 * user, then the daemon's own message and hint (English, like all daemon
 * output). Credentials never reach either; the daemon withholds them.
 */
export function ExchangeErrorNotice({
  provider,
  error,
}: {
  provider: string;
  error: ExchangeErrorInfo;
}) {
  const { t } = useTranslation("connections");
  const kind = exchangeErrorKind(error.code);
  const permissions = refusedPermissions(error);
  return (
    <div
      role="alert"
      className="space-y-1 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive"
    >
      <p className="m-0 font-medium">{t(ERROR_KEYS[kind], { provider })}</p>
      {permissions.length ? (
        <p className="m-0">
          {t("exchange.error.permissions", { list: permissions.join(", ") })}
        </p>
      ) : null}
      {error.message && kind !== "needsRust" ? (
        <p className="m-0 break-words">
          {t("exchange.error.serverSaid", { message: error.message })}
        </p>
      ) : null}
      {error.hint && kind !== "needsRust" ? (
        <p className="m-0 break-words">{error.hint}</p>
      ) : null}
    </div>
  );
}
