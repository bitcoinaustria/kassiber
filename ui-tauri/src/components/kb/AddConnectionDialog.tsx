import * as React from "react";
import { Database, FileInput, Server, Wallet, Zap } from "lucide-react";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import bitpandaIcon from "@/assets/integrations/bitpanda.svg";
import bitboxIcon from "@/assets/integrations/bitbox.svg";
import bluewalletIcon from "@/assets/integrations/bluewallet.png";
import btcpayIcon from "@/assets/integrations/btcpay.svg";
import coldcardIcon from "@/assets/integrations/coldcard.svg";
import coinfinityIcon from "@/assets/integrations/coinfinity-mark.svg";
import coinbaseIcon from "@/assets/integrations/coinbase.svg";
import coreLightningIcon from "@/assets/integrations/core-lightning.svg";
import foundationPassportIcon from "@/assets/integrations/foundation-passport.svg";
import krakenIcon from "@/assets/integrations/kraken.svg";
import ledgerIcon from "@/assets/integrations/ledger.svg";
import lightningLabsIcon from "@/assets/integrations/lightning-labs.png";
import lianaIcon from "@/assets/integrations/liana.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";
import nunchukIcon from "@/assets/integrations/nunchuk.svg";
import relaiIcon from "@/assets/integrations/relai.svg";
import sparrowIcon from "@/assets/integrations/sparrow.png";
import trezorIcon from "@/assets/integrations/trezor.svg";
import twentyOneBitcoinIcon from "@/assets/integrations/21bitcoin.png";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";

type ConnectionCategory =
  | "wallets"
  | "lightning"
  | "merchant"
  | "exchanges"
  | "files";

interface ConnectionSource {
  id: string;
  title: string;
  description: string;
  category: ConnectionCategory;
  image?: string;
  icon?: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  imageClassName?: string;
  imageFrameClassName?: string;
  status?: "ready" | "soon";
}

interface CategoryItem {
  id: ConnectionCategory;
  label: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
}

interface AddConnectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const CATEGORIES: CategoryItem[] = [
  { id: "wallets", label: "Wallets", icon: Wallet },
  { id: "lightning", label: "Lightning", icon: Zap },
  { id: "merchant", label: "Merchant", icon: Server },
  { id: "exchanges", label: "Exchanges", icon: Database },
  { id: "files", label: "Files", icon: FileInput },
];

const CONNECTION_SOURCES: ConnectionSource[] = [
  {
    id: "xpub",
    title: "XPub",
    description: "Single-sig on-chain watch-only wallet import.",
    category: "wallets",
    image: bitcoinIcon,
    imageClassName: "size-7",
    status: "ready",
  },
  {
    id: "descriptor",
    title: "Descriptor",
    description: "Multisig or descriptor wallet discovery.",
    category: "wallets",
    image: bitcoinIcon,
    imageClassName: "size-7",
    status: "soon",
  },
  {
    id: "liquid-descriptor",
    title: "Liquid descriptor",
    description: "Liquid watch-only wallet or Elements descriptor.",
    category: "wallets",
    image: liquidIcon,
    imageClassName: "size-8",
    status: "soon",
  },
  {
    id: "sparrow",
    title: "Sparrow",
    description: "Desktop wallet import for PSBT, descriptor, or xpub exports.",
    category: "wallets",
    image: sparrowIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "bluewallet",
    title: "BlueWallet",
    description: "Mobile wallet xpub and transaction export.",
    category: "wallets",
    image: bluewalletIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "liana",
    title: "Liana",
    description: "Timelock multisig descriptor import.",
    category: "wallets",
    image: lianaIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "nunchuk",
    title: "Nunchuk",
    description: "Collaborative multisig wallet export.",
    category: "wallets",
    image: nunchukIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "bitbox",
    title: "BitBox",
    description: "BitBox hardware wallet account export.",
    category: "wallets",
    image: bitboxIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "trezor",
    title: "Trezor",
    description: "Trezor Suite account export.",
    category: "wallets",
    image: trezorIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "coldcard",
    title: "Coldcard",
    description: "Coldcard skeleton wallet or descriptor import.",
    category: "wallets",
    image: coldcardIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "ledger",
    title: "Ledger",
    description: "Ledger Live account export.",
    category: "wallets",
    image: ledgerIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "foundation-passport",
    title: "Foundation Passport",
    description: "Passport wallet export or descriptor import.",
    category: "wallets",
    image: foundationPassportIcon,
    imageClassName: "size-9",
    status: "soon",
  },
  {
    id: "core-ln",
    title: "Core Lightning",
    description: "CLN node history through local RPC.",
    category: "lightning",
    image: coreLightningIcon,
    imageFrameClassName: "bg-[#494120]",
    imageClassName: "size-8",
    status: "soon",
  },
  {
    id: "lnd",
    title: "LND",
    description: "Lightning Network Daemon read-only data.",
    category: "lightning",
    image: lightningLabsIcon,
    imageClassName: "size-8",
    status: "soon",
  },
  {
    id: "btcpay",
    title: "BTCPay Server",
    description: "Store wallet history through a read-only API key.",
    category: "merchant",
    image: btcpayIcon,
    imageClassName: "h-9 w-auto",
    status: "soon",
  },
  {
    id: "bitpanda",
    title: "Bitpanda",
    description: "Read-only broker and exchange import.",
    category: "exchanges",
    image: bitpandaIcon,
    imageFrameClassName: "bg-[#103e36]",
    imageClassName: "h-9 w-auto",
    status: "soon",
  },
  {
    id: "relai",
    title: "Relai",
    description: "Bitcoin-only app activity import.",
    category: "exchanges",
    image: relaiIcon,
    imageClassName: "size-9 rounded-md",
    status: "soon",
  },
  {
    id: "21bitcoin",
    title: "21bitcoin",
    description: "Bitcoin-only app activity import.",
    category: "exchanges",
    image: twentyOneBitcoinIcon,
    imageClassName: "size-8 rounded-md",
    status: "soon",
  },
  {
    id: "coinfinity",
    title: "Coinfinity",
    description: "Bitcoin broker activity import.",
    category: "exchanges",
    image: coinfinityIcon,
    imageClassName: "size-8",
    status: "soon",
  },
  {
    id: "kraken",
    title: "Kraken",
    description: "Read-only exchange API import.",
    category: "exchanges",
    image: krakenIcon,
    imageClassName: "size-8",
    status: "soon",
  },
  {
    id: "coinbase",
    title: "Coinbase",
    description: "Read-only exchange API import.",
    category: "exchanges",
    image: coinbaseIcon,
    imageClassName: "size-8",
    status: "soon",
  },
  {
    id: "csv",
    title: "CSV import",
    description: "One-shot import from a local file.",
    category: "files",
    icon: FileInput,
    status: "soon",
  },
];

export function AddConnectionDialog({
  open,
  onOpenChange,
}: AddConnectionDialogProps) {
  const addNotification = useUiStore((state) => state.addNotification);
  const [activeCategory, setActiveCategory] =
    React.useState<ConnectionCategory>("wallets");
  const [selectedId, setSelectedId] = React.useState("xpub");

  const visibleSources = React.useMemo(
    () =>
      CONNECTION_SOURCES.filter((source) => source.category === activeCategory),
    [activeCategory],
  );
  const selected =
    CONNECTION_SOURCES.find((source) => source.id === selectedId) ??
    CONNECTION_SOURCES[0];

  const selectCategory = (category: ConnectionCategory) => {
    setActiveCategory(category);
    const firstSource = CONNECTION_SOURCES.find(
      (source) => source.category === category,
    );
    if (firstSource) {
      setSelectedId(firstSource.id);
    }
  };

  const stageConnection = () => {
    addNotification({
      title: "Connection setup selected",
      body: `${selected.title} is selected. The daemon-backed setup form can be wired next.`,
      tone: selected.status === "ready" ? "success" : "warning",
    });
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="grid h-[calc(100dvh-2rem)] max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:h-[720px] sm:max-w-[920px] lg:max-w-[980px]">
        <DialogHeader>
          <DialogTitle>Add connection</DialogTitle>
          <DialogDescription>
            Choose a watch-only wallet, node, exchange, or local file source.
          </DialogDescription>
        </DialogHeader>

        <div className="grid min-h-0 grid-cols-1 overflow-hidden rounded-lg border sm:grid-cols-[220px_minmax(0,1fr)]">
          <div className="overflow-y-auto border-b bg-muted/30 p-2 sm:border-r sm:border-b-0">
            {CATEGORIES.map((category) => {
              const Icon = category.icon;
              const active = activeCategory === category.id;
              return (
                <button
                  key={category.id}
                  type="button"
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors",
                    active
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
                  )}
                  onClick={() => selectCategory(category.id)}
                >
                  <Icon className="size-4" aria-hidden="true" />
                  {category.label}
                </button>
              );
            })}
          </div>

          <div className="min-h-0 space-y-3 overflow-y-auto p-4">
            {visibleSources.map((source) => {
              const selectedSource = selectedId === source.id;
              return (
                <button
                  key={source.id}
                  type="button"
                  className={cn(
                    "flex w-full items-start gap-4 rounded-lg border p-4 text-left transition-colors hover:bg-muted/40",
                    selectedSource && "border-primary bg-primary/5",
                  )}
                  onClick={() => setSelectedId(source.id)}
                >
                  <span
                    className={cn(
                      "flex size-12 shrink-0 items-center justify-center rounded-lg border bg-background p-1.5",
                      source.imageFrameClassName,
                    )}
                    aria-hidden="true"
                  >
                    {source.image ? (
                      <img
                        src={source.image}
                        alt=""
                        className={cn(
                          "max-h-full max-w-full object-contain",
                          source.imageClassName,
                        )}
                      />
                    ) : source.icon ? (
                      <source.icon className="size-6 text-muted-foreground" />
                    ) : null}
                  </span>
                  <span className="min-w-0 flex-1 space-y-1">
                    <span className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{source.title}</span>
                      {source.status === "soon" ? (
                        <Badge variant="outline">Soon</Badge>
                      ) : null}
                    </span>
                    <span className="block text-sm text-muted-foreground">
                      {source.description}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button type="button" onClick={stageConnection}>
            Continue
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
