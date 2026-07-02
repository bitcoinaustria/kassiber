import { useTranslation } from "react-i18next";

import type { ConnectionKind } from "@/mocks/seed";

// Maps the stable backend `kind` to a `chrome:protocol.*` label key. The keys
// are display labels (some translated, the Bitcoin jargon ones kept English);
// `kind` stays the stable lookup id.
const PROTOCOL_LABEL_KEYS: Record<ConnectionKind, string> = {
  xpub: "onChain",
  address: "onChain",
  descriptor: "onChain",
  "silent-payment": "onChain",
  samourai: "onChain",
  "core-ln": "lightning",
  lnd: "lightning",
  nwc: "nwc",
  cashu: "ecash",
  btcpay: "merchant",
  kraken: "exchange",
  bitstamp: "exchange",
  coinbase: "exchange",
  bitpanda: "exchange",
  river: "exchange",
  bullbitcoin: "exchange",
  coinfinity: "exchange",
  strike: "platform",
  phoenix: "lightning",
  custom: "custom",
  csv: "file",
  bip329: "labels",
};

export function ProtocolChip({ kind }: { kind: ConnectionKind }) {
  const { t } = useTranslation("chrome");
  return (
    <span className="font-mono text-[8px] font-semibold tracking-[0.14em] text-ink-3">
      {t(`protocol.${PROTOCOL_LABEL_KEYS[kind]}` as never) /* dynamic key */}
    </span>
  );
}
