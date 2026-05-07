/**
 * Imports route — integrations and connection onboarding as a real screen.
 */

import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import coinbaseIcon from "@/assets/integrations/coinbase.svg";
import lightningIcon from "@/assets/integrations/lightning.svg";
import {
  SettingsIntegrations4,
  type IntegrationItem,
} from "@/components/shadcnblocks/settings-integrations4";
import { Button } from "@/components/ui/button";
import { screenShellClassName } from "@/lib/screen-layout";

const sourceIcon = (label: string, background: string, foreground: string) =>
  `data:image/svg+xml,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" rx="10" fill="${background}"/><text x="20" y="24" text-anchor="middle" font-family="Inter, Arial, sans-serif" font-size="10" font-weight="700" fill="${foreground}">${label}</text></svg>`,
  )}`;

const IMPORT_ITEMS: IntegrationItem[] = [
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
    actionLabel: "Preview",
  },
  {
    id: "liquid-descriptor",
    title: "Liquid descriptor",
    description: "Liquid watch-only wallet or Elements descriptor.",
    category: "wallets",
    categoryLabel: "Wallets",
    image: sourceIcon("LQD", "#38bdf8", "#082f49"),
    actionLabel: "Preview",
  },
  {
    id: "core-ln",
    title: "Core Lightning",
    description: "CLN node history through local RPC.",
    category: "lightning",
    categoryLabel: "Lightning",
    image: lightningIcon,
    actionLabel: "Preview",
  },
  {
    id: "lnd",
    title: "LND",
    description: "Lightning Network Daemon read-only data.",
    category: "lightning",
    categoryLabel: "Lightning",
    image: lightningIcon,
    actionLabel: "Preview",
  },
  {
    id: "nwc",
    title: "NWC",
    description: "Nostr Wallet Connect event history.",
    category: "lightning",
    categoryLabel: "Lightning",
    image: sourceIcon("NWC", "#7c3aed", "#ffffff"),
    actionLabel: "Preview",
  },
  {
    id: "btcpay",
    title: "BTCPay Server",
    description: "Store wallet history through a read-only API key.",
    category: "merchant",
    categoryLabel: "Merchant",
    image: sourceIcon("BTCP", "#111827", "#ffffff"),
    actionLabel: "Preview",
  },
  {
    id: "cashu",
    title: "Cashu",
    description: "Ecash mint wallet activity.",
    category: "merchant",
    categoryLabel: "Merchant",
    image: sourceIcon("EC", "#10b981", "#052e1a"),
    actionLabel: "Preview",
  },
  {
    id: "kraken",
    title: "Kraken",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: sourceIcon("KR", "#4f46e5", "#ffffff"),
    actionLabel: "Preview",
  },
  {
    id: "bitstamp",
    title: "Bitstamp",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: sourceIcon("BS", "#16a34a", "#ffffff"),
    actionLabel: "Preview",
  },
  {
    id: "coinbase",
    title: "Coinbase",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: coinbaseIcon,
    actionLabel: "Preview",
  },
  {
    id: "bitpanda",
    title: "Bitpanda",
    description: "Read-only exchange API import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: sourceIcon("BP", "#111827", "#ffffff"),
    actionLabel: "Preview",
  },
  {
    id: "river",
    title: "River",
    description: "Read-only brokerage import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: sourceIcon("RV", "#0f172a", "#ffffff"),
    actionLabel: "Preview",
  },
  {
    id: "strike",
    title: "Strike",
    description: "Read-only Lightning and fiat activity import.",
    category: "exchanges",
    categoryLabel: "Exchanges",
    image: lightningIcon,
    actionLabel: "Preview",
  },
  {
    id: "csv",
    title: "CSV import",
    description: "One-shot import from a local file.",
    category: "files",
    categoryLabel: "Files",
    image: sourceIcon("CSV", "#64748b", "#ffffff"),
    actionLabel: "Preview",
  },
];

export function Imports() {
  const navigate = useNavigate();
  const [selectedId, setSelectedId] = useState("xpub");

  return (
    <div className={screenShellClassName}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            Connections · imports · integrations
          </p>
          <h2 className="text-2xl font-semibold tracking-tight">
            Add connection
          </h2>
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

      <SettingsIntegrations4
        heading="Connection sources"
        subHeading="Choose the source type. Only watch-only or read-only flows belong here."
        integrations={IMPORT_ITEMS}
        selectedId={selectedId}
        onSelect={(integration) =>
          setSelectedId(integration.id ?? integration.title)
        }
      />
    </div>
  );
}
