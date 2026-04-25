/**
 * Add-connection picker modal.
 *
 * Adapted from shadcnblocks/settings-integrations4: tabbed integration
 * categories with card rows and primary connect actions. It stays wrapped
 * in Kassiber's existing modal flow so selecting XPub can still continue to
 * the dedicated XPub form.
 */

import { useMemo, useState } from "react";
import {
  Bitcoin,
  Bolt,
  ChevronRight,
  FileText,
  Landmark,
  Lock,
  Server,
  Store,
  Wallet,
} from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
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

interface ConnectionItem {
  k: ConnectionKindKey;
  name: string;
  desc: string;
  status?: "available" | "soon";
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
}

interface SectionDef {
  id: string;
  label: string;
  items: ConnectionItem[];
}

const SECTIONS: SectionDef[] = [
  {
    id: "self-custody",
    label: "Self-custody",
    items: [
      {
        k: "xpub",
        name: "XPub",
        desc: "Single-sig on-chain watch-only import.",
        status: "available",
        icon: Bitcoin,
      },
      {
        k: "descriptor",
        name: "Descriptor",
        desc: "Multisig or descriptor wallet discovery.",
        status: "soon",
        icon: Wallet,
      },
    ],
  },
  {
    id: "lightning",
    label: "Lightning",
    items: [
      {
        k: "core-ln",
        name: "Core Lightning",
        desc: "CLN node history through local RPC.",
        status: "soon",
        icon: Bolt,
      },
      {
        k: "lnd",
        name: "LND",
        desc: "Lightning Network Daemon read-only data.",
        status: "soon",
        icon: Server,
      },
      {
        k: "nwc",
        name: "NWC",
        desc: "Nostr Wallet Connect event history.",
        status: "soon",
        icon: Bolt,
      },
    ],
  },
  {
    id: "merchant",
    label: "Merchant",
    items: [
      {
        k: "btcpay",
        name: "BTCPay Server",
        desc: "Store wallet history through a read key.",
        status: "soon",
        icon: Store,
      },
      {
        k: "cashu",
        name: "Cashu",
        desc: "Ecash mint wallet activity.",
        status: "soon",
        icon: Wallet,
      },
    ],
  },
  {
    id: "exchanges",
    label: "Exchanges",
    items: [
      {
        k: "kraken",
        name: "Kraken",
        desc: "Read-only API import.",
        status: "soon",
        icon: Landmark,
      },
      {
        k: "bitstamp",
        name: "Bitstamp",
        desc: "Read-only API import.",
        status: "soon",
        icon: Landmark,
      },
      {
        k: "coinbase",
        name: "Coinbase",
        desc: "Read-only API import.",
        status: "soon",
        icon: Landmark,
      },
      {
        k: "bitpanda",
        name: "Bitpanda",
        desc: "Read-only API import.",
        status: "soon",
        icon: Landmark,
      },
      {
        k: "river",
        name: "River",
        desc: "Read-only API import.",
        status: "soon",
        icon: Landmark,
      },
      {
        k: "strike",
        name: "Strike",
        desc: "Read-only API import.",
        status: "soon",
        icon: Landmark,
      },
    ],
  },
  {
    id: "file",
    label: "File",
    items: [
      {
        k: "csv",
        name: "CSV import",
        desc: "One-shot import from a local file.",
        status: "soon",
        icon: FileText,
      },
    ],
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
  const [activeCategory, setActiveCategory] = useState(SECTIONS[0].id);
  const activeSection = useMemo(
    () => SECTIONS.find((section) => section.id === activeCategory) ?? SECTIONS[0],
    [activeCategory],
  );

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        showCloseButton={true}
        className={cn(
          "max-w-[820px] gap-0 overflow-hidden rounded-lg border bg-background p-0 shadow-xl",
          "[&>button[data-slot=dialog-close]]:right-4 [&>button[data-slot=dialog-close]]:top-4",
        )}
      >
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle className="text-2xl font-semibold tracking-tight">
            Add a connection
          </DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground">
            Connect watch-only wallets, nodes, services, exchanges, or local
            files. Kassiber never asks for private keys.
          </DialogDescription>
        </DialogHeader>

        <div className="border-b px-6 pt-4">
          <div className="flex gap-1 overflow-x-auto pb-3">
            {SECTIONS.map((section) => {
              const active = activeSection.id === section.id;
              return (
                <button
                  key={section.id}
                  type="button"
                  onClick={() => setActiveCategory(section.id)}
                  className={cn(
                    "h-9 shrink-0 rounded-md px-3 text-sm font-medium transition-colors",
                    active
                      ? "bg-muted text-foreground"
                      : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                  )}
                >
                  {section.label}
                </button>
              );
            })}
          </div>
        </div>

        <div className="max-h-[520px] overflow-y-auto px-6 py-5">
          <div className="space-y-3">
            {activeSection.items.map((item) => (
              <ConnectionCard key={item.k} item={item} onPick={onPick} />
            ))}
          </div>
        </div>

        <div className="border-t bg-muted/30 px-6 py-4">
          <div className="flex items-start gap-3 rounded-lg border bg-background px-4 py-3">
            <Lock className="mt-0.5 size-4 shrink-0 text-primary" />
            <span className="text-sm leading-6 text-muted-foreground">
              Watch-only by design. Use extended public keys, descriptors, local
              files, or read-only credentials. Withdrawal permissions and
              private keys stay outside Kassiber.
            </span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ConnectionCard({
  item,
  onPick,
}: {
  item: ConnectionItem;
  onPick: (kind: ConnectionKindKey) => void;
}) {
  const Icon = item.icon;
  const available = item.status === "available";

  return (
    <div className="flex items-start gap-4 rounded-lg border p-4 transition-colors hover:bg-muted/30">
      <div className="flex size-10 shrink-0 items-center justify-center rounded-md border bg-background">
        <Icon className="size-5 text-muted-foreground" aria-hidden="true" />
      </div>
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="font-medium">{item.name}</p>
          {available ? (
            <span className="inline-flex rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-800">
              Available
            </span>
          ) : (
            <span className="inline-flex rounded-full bg-secondary px-2 py-0.5 text-xs font-medium text-secondary-foreground">
              Soon
            </span>
          )}
        </div>
        <p className="text-sm text-muted-foreground">{item.desc}</p>
      </div>
      <Button
        variant={available ? "default" : "outline"}
        size="sm"
        onClick={() => onPick(item.k)}
      >
        {available ? "Connect" : "Preview"}
        <ChevronRight className="size-4" aria-hidden="true" />
      </Button>
    </div>
  );
}
