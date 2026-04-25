import type { ConnectionKind } from "@/mocks/seed";

const PROTOCOL_LABELS: Record<ConnectionKind, string> = {
  xpub: "ON-CHAIN",
  descriptor: "ON-CHAIN",
  "core-ln": "LIGHTNING",
  lnd: "LIGHTNING",
  nwc: "NWC",
  cashu: "ECASH",
  btcpay: "MERCHANT",
  kraken: "EXCHANGE",
  bitstamp: "EXCHANGE",
  coinbase: "EXCHANGE",
  bitpanda: "EXCHANGE",
  river: "EXCHANGE",
  strike: "EXCHANGE",
  csv: "FILE",
  bip329: "LABELS",
};

export function ProtocolChip({ kind }: { kind: ConnectionKind }) {
  return (
    <span className="font-mono text-[8px] font-semibold tracking-[0.14em] text-ink-3">
      {PROTOCOL_LABELS[kind]}
    </span>
  );
}
