/**
 * Add-connection picker modal — translated from
 * claude-design/screens/connections.jsx (ConnectionTypePicker).
 *
 * Opened from the Add connection buttons on /connections and the
 * Overview connections card. Picking a kind closes the picker and
 * forwards the selection upward; the parent decides what to do
 * (e.g. open XpubForm for `xpub`, show a coming-soon panel for
 * other kinds).
 */

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { ChevronRight, Lock } from "lucide-react";
import { cn } from "@/lib/utils";

export type ConnectionKindKey =
  | "xpub"
  | "descriptor"
  | "core-ln"
  | "lnd"
  | "nwc"
  | "btcpay"
  | "cashu"
  | "kraken"
  | "bitstamp"
  | "coinbase"
  | "bitpanda"
  | "river"
  | "strike"
  | "csv";

interface SectionDef {
  label: string;
  items: Array<{ k: ConnectionKindKey; name: string; desc: string }>;
}

const SECTIONS: SectionDef[] = [
  {
    label: "Self-custody · On-chain",
    items: [
      { k: "xpub", name: "XPub", desc: "Single-sig on-chain watch" },
      { k: "descriptor", name: "Descriptor", desc: "Multisig wallet descriptor" },
    ],
  },
  {
    label: "Lightning",
    items: [
      { k: "core-ln", name: "Core Lightning", desc: "CLN node RPC" },
      { k: "lnd", name: "LND", desc: "Lightning Network Daemon" },
      { k: "nwc", name: "NWC", desc: "Nostr Wallet Connect" },
    ],
  },
  {
    label: "Services · Merchant",
    items: [
      { k: "btcpay", name: "BTCPay Server", desc: "Merchant API · store read-key" },
      { k: "cashu", name: "Cashu", desc: "Ecash mint wallet" },
    ],
  },
  {
    label: "Exchanges · Read-only API",
    items: [
      { k: "kraken", name: "Kraken", desc: "Read-only API key" },
      { k: "bitstamp", name: "Bitstamp", desc: "Read-only API key" },
      { k: "coinbase", name: "Coinbase", desc: "Read-only API key" },
      { k: "bitpanda", name: "Bitpanda", desc: "Read-only API key" },
      { k: "river", name: "River", desc: "Read-only API key" },
      { k: "strike", name: "Strike", desc: "Read-only API key" },
    ],
  },
  {
    label: "File",
    items: [{ k: "csv", name: "CSV import", desc: "One-shot, from file" }],
  },
];

interface ConnectionTypePickerProps {
  open: boolean;
  onClose: () => void;
  onPick: (kind: ConnectionKindKey) => void;
}

export function ConnectionTypePicker({
  open,
  onClose,
  onPick,
}: ConnectionTypePickerProps) {
  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        showCloseButton={true}
        className={cn(
          "max-w-[720px] gap-0 rounded-none border border-ink bg-paper p-0 shadow-hard-ink",
          "[&>button[data-slot=dialog-close]]:right-3 [&>button[data-slot=dialog-close]]:top-3",
        )}
      >
        <DialogHeader className="border-b border-line px-5 py-4">
          <DialogTitle className="font-sans text-lg font-semibold text-ink">
            Add a connection
          </DialogTitle>
          <DialogDescription className="font-sans text-[13px] text-ink-2">
            Kassiber is watch-only. Keys never leave your machine.
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[460px] overflow-y-auto px-5 py-4">
          {SECTIONS.map((sec, si) => (
            <div key={sec.label} className={si === 0 ? "" : "mt-4.5"}>
              <div className="mb-2 flex items-center gap-2.5 font-mono text-[10px] uppercase tracking-[0.16em] text-ink-3">
                <span>{sec.label}</span>
                <span className="h-px flex-1 bg-line" />
                <span>{String(sec.items.length).padStart(2, "0")}</span>
              </div>
              <div className="grid grid-cols-2 gap-1.5">
                {sec.items.map((item) => (
                  <button
                    key={item.k}
                    onClick={() => {
                      onPick(item.k);
                      onClose();
                    }}
                    className="grid cursor-pointer grid-cols-[1fr_auto] items-center gap-3 border border-line bg-transparent px-3.5 py-3 text-left transition-colors hover:border-ink hover:bg-paper"
                  >
                    <div className="min-w-0">
                      <div className="font-sans text-sm font-semibold tracking-[-0.005em] text-ink">
                        {item.name}
                      </div>
                      <div className="mt-0.5 font-mono text-[10px] tracking-[0.04em] text-ink-3">
                        {item.desc.toUpperCase()}
                      </div>
                    </div>
                    <ChevronRight className="size-3.5 text-ink-3" />
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>

        <div className="border-t border-line px-5 py-3">
          <div className="flex items-start gap-2.5 border border-line bg-paper-2 px-3 py-3">
            <Lock className="mt-0.5 size-3.5 shrink-0 text-accent" />
            <span className="font-sans text-[11px] leading-[1.55] text-ink-2">
              Watch-only by design. Kassiber imports history via extended
              public keys, descriptors, or read-only API credentials.{" "}
              <b>
                No private keys or withdrawal permissions ever touch this
                machine through Kassiber.
              </b>
            </span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
