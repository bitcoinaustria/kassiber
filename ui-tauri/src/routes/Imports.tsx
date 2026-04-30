/**
 * Imports route — integrations and connection onboarding as a real screen.
 */

import { useMemo, useState } from "react";
import { ArrowLeft, Check } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import coinbaseIcon from "@/assets/integrations/coinbase.svg";
import lightningIcon from "@/assets/integrations/lightning.svg";
import {
  SettingsIntegrations8,
  type SettingsIntegration8Item,
} from "@/components/shadcnblocks/settings-integrations8";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";

type AddrType = "p2pkh" | "p2sh" | "p2wpkh" | "p2tr";

const ADDRESS_TYPES: Array<[AddrType, string, string]> = [
  ["p2pkh", "Pay to Public Key Hash", "1A1zP1..."],
  ["p2sh", "Pay to Script Hash", "3J98t1..."],
  ["p2wpkh", "Pay to Witness Pub Hash", "bc1qar..."],
  ["p2tr", "Pay to Taproot", "bc1p5c..."],
];

const IMPORT_ITEMS: SettingsIntegration8Item[] = [
  {
    id: "xpub",
    title: "XPub",
    description: "Single-sig on-chain watch-only wallet import.",
    category: "wallets",
    categoryLabel: "Wallets",
    image: bitcoinIcon,
    actionLabel: "Configure",
  },
  {
    id: "descriptor",
    title: "Descriptor",
    description: "Multisig or descriptor wallet discovery.",
    category: "wallets",
    categoryLabel: "Wallets",
    image: bitcoinIcon,
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "liquid-descriptor",
    title: "Liquid descriptor",
    description: "Liquid watch-only wallet or Elements descriptor.",
    category: "wallets",
    categoryLabel: "Wallets",
    initials: "LQD",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "core-ln",
    title: "Core Lightning",
    description: "CLN node history through local RPC.",
    category: "lightning",
    categoryLabel: "Lightning",
    image: lightningIcon,
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "lnd",
    title: "LND",
    description: "Lightning Network Daemon read-only data.",
    category: "lightning",
    categoryLabel: "Lightning",
    image: lightningIcon,
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "nwc",
    title: "NWC",
    description: "Nostr Wallet Connect event history.",
    category: "lightning",
    categoryLabel: "Lightning",
    initials: "NWC",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "btcpay",
    title: "BTCPay Server",
    description: "Store wallet history through a read-only API key.",
    category: "merchant",
    categoryLabel: "Merchant",
    initials: "BTCP",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "cashu",
    title: "Cashu",
    description: "Ecash mint wallet activity.",
    category: "merchant",
    categoryLabel: "Merchant",
    initials: "EC",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "kraken",
    title: "Kraken",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    initials: "KR",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "bitstamp",
    title: "Bitstamp",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    initials: "BS",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "coinbase",
    title: "Coinbase",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: coinbaseIcon,
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "bitpanda",
    title: "Bitpanda",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    initials: "BP",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "river",
    title: "River",
    description: "Read-only brokerage import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    initials: "RV",
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "strike",
    title: "Strike",
    description: "Read-only Lightning and fiat activity import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: lightningIcon,
    disabled: true,
    actionLabel: "Preview",
  },
  {
    id: "csv",
    title: "CSV import",
    description: "One-shot import from a local file.",
    category: "files",
    categoryLabel: "Files",
    initials: "CSV",
    disabled: true,
    actionLabel: "Preview",
  },
];

export function Imports() {
  const navigate = useNavigate();
  const addNotification = useUiStore((state) => state.addNotification);
  const [selectedId, setSelectedId] = useState("xpub");
  const selected = useMemo(
    () => IMPORT_ITEMS.find((item) => item.id === selectedId) ?? IMPORT_ITEMS[0],
    [selectedId],
  );

  return (
    <div className="w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            Connections · imports · integrations
          </p>
          <h2 className="text-2xl font-semibold tracking-tight">Imports</h2>
          <p className="text-sm text-muted-foreground">
            Add watch-only wallet sources, node integrations, exchange imports,
            and local files.
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void navigate({ to: "/connections" })}
        >
          <ArrowLeft className="size-4" aria-hidden="true" />
          Connections
        </Button>
      </div>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,0.55fr)]">
        <SettingsIntegrations8
          heading="Connection sources"
          subHeading="Choose the source type. Only watch-only or read-only flows belong here."
          integrations={IMPORT_ITEMS}
          selectedId={selected.id}
          onSelect={(integration) => setSelectedId(integration.id)}
        />
        <IntegrationDetail
          item={selected}
          onSaved={(label) => {
            addNotification({
              title: "Connection staged",
              body: `${label} is ready for the daemon-backed save flow.`,
              tone: "success",
            });
            void navigate({ to: "/connections" });
          }}
        />
      </div>
    </div>
  );
}

function IntegrationDetail({
  item,
  onSaved,
}: {
  item: SettingsIntegration8Item;
  onSaved: (label: string) => void;
}) {
  if (item.id === "xpub") {
    return <XpubInlineForm onSaved={onSaved} />;
  }

  return (
    <Card className="min-w-0">
      <CardHeader>
        <CardTitle>{item.title}</CardTitle>
        <CardDescription>{item.description}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Badge variant="outline">Not wired yet</Badge>
        <p className="text-sm leading-6 text-muted-foreground">
          This source type is shown so the import catalog is complete, but the
          daemon-backed form still needs to land before it can save data.
        </p>
      </CardContent>
    </Card>
  );
}

function XpubInlineForm({ onSaved }: { onSaved: (label: string) => void }) {
  const [label, setLabel] = useState("Cold Storage");
  const [xpub, setXpub] = useState("");
  const [addrTypes, setAddrTypes] = useState<Record<AddrType, boolean>>({
    p2pkh: false,
    p2sh: false,
    p2wpkh: true,
    p2tr: false,
  });
  const [gap, setGap] = useState(10);
  const detected = useMemo(() => {
    if (!xpub) return "-";
    if (xpub.startsWith("zpub")) return "BIP84 · native segwit";
    if (xpub.startsWith("ypub")) return "BIP49 · nested";
    return "BIP44";
  }, [xpub]);

  return (
    <Card className="min-w-0">
      <CardHeader>
        <CardTitle>XPub connection</CardTitle>
        <CardDescription>
          Enter an extended public key. Kassiber derives addresses and syncs
          on-chain history without private keys.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <div className="grid gap-2">
          <Label htmlFor="import-xpub-label">Connection label</Label>
          <Input
            id="import-xpub-label"
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            placeholder="e.g. Cold Storage"
          />
        </div>

        <div className="grid gap-2">
          <Label htmlFor="import-xpub-key">XPUB / YPUB / ZPUB</Label>
          <Input
            id="import-xpub-key"
            value={xpub}
            onChange={(event) => setXpub(event.target.value)}
            placeholder="xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j..."
            className="font-mono text-xs"
          />
          <div className="flex flex-wrap gap-2 pt-1">
            <Badge variant="secondary">Detected: {detected}</Badge>
            <Badge variant="outline">Fingerprint: {xpub ? "5f3a · 8c0e" : "-"}</Badge>
          </div>
        </div>

        <section className="grid gap-3">
          <Label>Address types to derive</Label>
          <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
            {ADDRESS_TYPES.map(([key, name, prefix]) => (
              <div
                key={key}
                className={cn(
                  "cursor-pointer rounded-lg border p-3 transition-colors hover:bg-muted/40",
                  addrTypes[key] && "border-primary bg-primary/5",
                )}
                onClick={() =>
                  setAddrTypes((current) => ({
                    ...current,
                    [key]: !current[key],
                  }))
                }
              >
                <div className="flex items-center gap-3">
                  <Checkbox
                    checked={addrTypes[key]}
                    onCheckedChange={(checked) =>
                      setAddrTypes((current) => ({
                        ...current,
                        [key]: checked === true,
                      }))
                    }
                    onClick={(event) => event.stopPropagation()}
                    aria-label={name}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">{name}</span>
                    <span className="block font-mono text-xs text-muted-foreground">
                      {prefix}
                    </span>
                  </span>
                </div>
              </div>
            ))}
          </div>
        </section>

        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
          <div className="grid gap-2">
            <Label htmlFor="import-xpub-gap">Gap limit</Label>
            <Input
              id="import-xpub-gap"
              value={String(gap)}
              onChange={(event) =>
                setGap(Number.parseInt(event.target.value, 10) || 0)
              }
              type="number"
              className="font-mono"
            />
          </div>
          <div className="grid gap-2">
            <Label>Sync backend</Label>
            <Select value="mempool-space">
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="mempool-space">
                  Mempool.space (default)
                </SelectItem>
                <SelectItem value="electrum" disabled>
                  Electrum (soon)
                </SelectItem>
                <SelectItem value="bitcoin-core" disabled>
                  Bitcoin Core RPC (soon)
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="flex justify-end border-t pt-4">
          <Button
            type="button"
            disabled={!xpub.trim()}
            onClick={() => onSaved(label.trim() || "Cold Storage")}
          >
            <Check className="size-4" aria-hidden="true" />
            Save and sync
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
