import * as React from "react";
import { Database, FileInput, Server, Wallet, Zap } from "lucide-react";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import bitpandaIcon from "@/assets/integrations/bitpanda.svg";
import btcpayIcon from "@/assets/integrations/btcpay.svg";
import coinfinityIcon from "@/assets/integrations/coinfinity-mark.svg";
import coinbaseIcon from "@/assets/integrations/coinbase.svg";
import coreLightningIcon from "@/assets/integrations/core-lightning.svg";
import krakenIcon from "@/assets/integrations/kraken.svg";
import lightningLabsIcon from "@/assets/integrations/lightning-labs.png";
import liquidIcon from "@/assets/integrations/liquid.svg";
import relaiIcon from "@/assets/integrations/relai.svg";
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
      <DialogContent className="sm:max-w-[920px] lg:max-w-[980px]">
        <DialogHeader>
          <DialogTitle>Add connection</DialogTitle>
          <DialogDescription>
            Choose a watch-only wallet, node, exchange, or local file source.
          </DialogDescription>
        </DialogHeader>

        <div className="grid min-h-[420px] grid-cols-1 overflow-hidden rounded-lg border sm:grid-cols-[220px_minmax(0,1fr)]">
          <div className="border-b bg-muted/30 p-2 sm:border-r sm:border-b-0">
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

          <div className="space-y-3 p-4">
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
