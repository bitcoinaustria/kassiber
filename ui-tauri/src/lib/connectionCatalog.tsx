import type * as React from "react";
import {
  Database,
  FileInput,
  FileSpreadsheet,
  Server,
  Tags,
  Wallet,
  Zap,
} from "lucide-react";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import bitpandaIcon from "@/assets/integrations/bitpanda.svg";
import bitboxIcon from "@/assets/integrations/bitbox.svg";
import bluewalletIcon from "@/assets/integrations/bluewallet.png";
import btcpayIcon from "@/assets/integrations/btcpay.svg";
import bullBitcoinIcon from "@/assets/integrations/bullbitcoin.jpg";
import coldcardIcon from "@/assets/integrations/coldcard.svg";
import coinfinityIcon from "@/assets/integrations/coinfinity-mark.svg";
import coinbaseIcon from "@/assets/integrations/coinbase.svg";
import coreLightningIcon from "@/assets/integrations/core-lightning.svg";
import foundationPassportIcon from "@/assets/integrations/foundation-passport.svg";
import krakenIcon from "@/assets/integrations/kraken.svg";
import ledgerIcon from "@/assets/integrations/ledger.svg";
import lightningIcon from "@/assets/integrations/lightning.svg";
import lightningLabsIcon from "@/assets/integrations/lightning-labs.png";
import lianaIcon from "@/assets/integrations/liana.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";
import mempoolIcon from "@/assets/integrations/mempool-space.svg";
import nunchukIcon from "@/assets/integrations/nunchuk.svg";
import relaiIcon from "@/assets/integrations/relai.svg";
import samouraiIcon from "@/assets/integrations/samourai.png";
import sparrowIcon from "@/assets/integrations/sparrow.png";
import strikeIcon from "@/assets/integrations/strike.jpg";
import trezorIcon from "@/assets/integrations/trezor.svg";
import twentyOneBitcoinIcon from "@/assets/integrations/21bitcoin.png";
import wasabiIcon from "@/assets/integrations/wasabi.png";

export type ConnectionCategory =
  | "wallets"
  | "nodes"
  | "lightning"
  | "merchant"
  | "exchanges"
  | "files";

export type SetupKind =
  | "descriptor"
  | "file-wallet"
  | "file-enrichment"
  | "samourai"
  | "btcpay"
  | "bullbitcoin-wallet"
  | "bip329"
  | "backend-settings"
  | "planned";

export type ConnectionSourceFormat =
  | "csv"
  | "json"
  | "phoenix_csv"
  | "river_csv"
  | "bullbitcoin_csv"
  | "bullbitcoin_wallet_csv"
  | "coinfinity_csv"
  | "21bitcoin_csv"
  | "strike_csv"
  | "wasabi_bundle"
  | "generic_ledger";

export interface ConnectionSource {
  id: string;
  title: string;
  description: string;
  category: ConnectionCategory;
  image?: string;
  icon?: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  imageClassName?: string;
  imageFrameClassName?: string;
  status: "ready" | "planned";
  pathLabel: string;
  formatLabel?: string;
  docsHref?: string;
  setupKind?: SetupKind;
  walletKind?: string;
  sourceFormat?: ConnectionSourceFormat;
  chain?: "bitcoin" | "liquid";
  network?: string;
  details: string[];
}

export interface ConnectionCategoryItem {
  id: ConnectionCategory;
  label: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
}

export const sourceIcon = (
  label: string,
  background: string,
  foreground: string,
) =>
  `data:image/svg+xml,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" rx="10" fill="${background}"/><text x="20" y="24" text-anchor="middle" font-family="Inter, Arial, sans-serif" font-size="10" font-weight="700" fill="${foreground}">${label}</text></svg>`,
  )}`;

const lightLogoFrame = "bg-white shadow-sm shadow-zinc-950/5 dark:bg-white dark:shadow-black/30";

export const CONNECTION_CATEGORIES: ConnectionCategoryItem[] = [
  { id: "wallets", label: "Wallets", icon: Wallet },
  { id: "nodes", label: "Nodes", icon: Server },
  { id: "lightning", label: "Lightning", icon: Zap },
  { id: "merchant", label: "Merchant", icon: Server },
  { id: "exchanges", label: "Exchanges", icon: Database },
  { id: "files", label: "Files", icon: FileInput },
];

export const connectionCategoryLabel = (category: ConnectionCategory) =>
  CONNECTION_CATEGORIES.find((item) => item.id === category)?.label ?? category;

export const CONNECTION_SOURCES: ConnectionSource[] = [
  {
    id: "xpub",
    title: "Wallet export",
    description: "Single-sig descriptor, xpub-family, or wallet export import.",
    category: "wallets",
    image: bitcoinIcon,
    imageClassName: "size-7",
    status: "ready",
    pathLabel: "Watch-only wallet",
    formatLabel: "descriptor/xpub-family",
    setupKind: "descriptor",
    walletKind: "descriptor",
    chain: "bitcoin",
    details: [
      "Mainnet by default",
      "Uses a configured Bitcoin backend",
      "ypub/zpub/upub/vpub keys are converted to descriptors",
    ],
  },
  {
    id: "descriptor",
    title: "Descriptor",
    description: "Multisig or descriptor wallet discovery.",
    category: "wallets",
    image: bitcoinIcon,
    imageClassName: "size-7",
    status: "ready",
    pathLabel: "Watch-only wallet",
    formatLabel: "output descriptor",
    setupKind: "descriptor",
    walletKind: "descriptor",
    chain: "bitcoin",
    details: [
      "Paste one common wallet export or descriptor",
      "Kassiber stores receive/change branches when present",
    ],
  },
  {
    id: "liquid-descriptor",
    title: "Liquid descriptor",
    description: "Liquid watch-only wallet or Elements descriptor.",
    category: "wallets",
    image: liquidIcon,
    imageClassName: "size-8",
    status: "ready",
    pathLabel: "Watch-only wallet",
    formatLabel: "Liquid descriptor",
    setupKind: "descriptor",
    walletKind: "descriptor",
    chain: "liquid",
    network: "liquidv1",
    details: [
      "Paste one common Liquid wallet export or descriptor",
      "Requires a configured Liquid backend",
    ],
  },
  {
    id: "samourai",
    title: "Samourai Wallet",
    description: "Watch-only Samourai and Whirlpool account descriptor/xpub import.",
    category: "wallets",
    image: samouraiIcon,
    imageClassName: "size-9",
    status: "ready",
    pathLabel: "Watch-only wallet group",
    formatLabel: "Samourai/Whirlpool descriptors",
    setupKind: "samourai",
    walletKind: "samourai",
    chain: "bitcoin",
    network: "main",
    details: [
      "Creates child sources for pasted Deposit, Badbank, Premix, and Postmix accounts",
      "Use Sparrow or another descriptor tool to export public account descriptors/xpubs",
      "Stores only watch-only sync targets",
      "Postmix discovery starts with a widened gap limit",
    ],
  },
  {
    id: "wasabi",
    title: "Wasabi Wallet",
    description: "Paste Wasabi RPC outputs with CoinJoin and anonymity evidence.",
    category: "wallets",
    image: wasabiIcon,
    imageClassName: "size-9",
    status: "ready",
    pathLabel: "Wallet export",
    formatLabel: "wasabi_bundle",
    setupKind: "file-wallet",
    walletKind: "wasabi",
    sourceFormat: "wasabi_bundle",
    chain: "bitcoin",
    docsHref: "https://docs.wasabiwallet.io/using-wasabi/RPC.html",
    details: [
      "Load the wallet in Wasabi, then copy local RPC JSON from port 37128",
      "Use Basic Auth in curl when JsonRpcUser and JsonRpcPassword are set",
      "Paste gethistory plus listcoins/listunspentcoins for Coins anonymity state",
      "CoinJoin evidence becomes review warnings, not fabricated provenance",
    ],
  },
  {
    id: "bitcoin-core",
    title: "Bitcoin Core",
    description: "Your own full node over JSON-RPC. Address-based sync only; no transaction links.",
    category: "nodes",
    image: bitcoinIcon,
    imageClassName: "size-7",
    status: "ready",
    pathLabel: "Node backend",
    formatLabel: "bitcoinrpc",
    setupKind: "backend-settings",
    details: [
      "Configured in Settings as a Bitcoin Core RPC backend",
      "Descriptor-backed RPC scanning is not implemented yet",
    ],
  },
  {
    id: "electrum",
    title: "Electrum server",
    description: "Electrum protocol over TCP/SSL (e.g. Fulcrum, ElectrumX). Sync-only — no transaction links.",
    category: "nodes",
    image: sourceIcon("EL", "#2563eb", "#ffffff"),
    status: "ready",
    pathLabel: "Index backend",
    formatLabel: "electrum",
    setupKind: "backend-settings",
    details: [
      "Configured in Settings with ssl://host:50002 or tcp://host:50001",
      "Works for Bitcoin and Liquid descriptor refresh when the backend matches the chain",
    ],
  },
  {
    id: "esplora",
    title: "Esplora / mempool",
    description: "Esplora / mempool.space-type HTTP REST API. Syncs and provides clickable transaction links.",
    category: "nodes",
    image: mempoolIcon,
    imageClassName: "size-7",
    imageFrameClassName: lightLogoFrame,
    status: "ready",
    pathLabel: "Index backend",
    formatLabel: "esplora",
    setupKind: "backend-settings",
    details: [
      "Kassiber ships with a built-in Esplora/mempool-API Bitcoin backend",
      "Use Settings to add a self-hosted Esplora endpoint",
    ],
  },
  {
    id: "sparrow",
    title: "Sparrow",
    description: "Desktop wallet import for PSBT, descriptor, or xpub exports.",
    category: "wallets",
    image: sparrowIcon,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    formatLabel: "descriptor/xpub",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "specter",
    title: "Specter Desktop",
    description: "Multisig wallet descriptor export.",
    category: "wallets",
    image: sourceIcon("SP", "#7c3aed", "#ffffff"),
    status: "planned",
    pathLabel: "Wallet export",
    formatLabel: "descriptor",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "bluewallet",
    title: "BlueWallet",
    description: "Mobile wallet xpub and transaction export.",
    category: "wallets",
    image: bluewalletIcon,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    details: ["Use generic CSV or address wallets until a dedicated parser lands"],
  },
  {
    id: "blockstream-green",
    title: "Blockstream Green",
    description: "Bitcoin and Liquid wallet export.",
    category: "wallets",
    image: sourceIcon("GR", "#00b45a", "#052e16"),
    status: "planned",
    pathLabel: "Wallet export",
    formatLabel: "descriptor/xpub",
    details: [
      "Use Descriptor or Liquid descriptor when you have an exported descriptor",
    ],
  },
  {
    id: "liana",
    title: "Liana",
    description: "Timelock multisig descriptor import.",
    category: "wallets",
    image: lianaIcon,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    formatLabel: "descriptor",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "nunchuk",
    title: "Nunchuk",
    description: "Collaborative multisig wallet export.",
    category: "wallets",
    image: nunchukIcon,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "bitbox",
    title: "BitBox",
    description: "BitBox hardware wallet account export.",
    category: "wallets",
    image: bitboxIcon,
    imageFrameClassName: lightLogoFrame,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "trezor",
    title: "Trezor",
    description: "Trezor Suite account export.",
    category: "wallets",
    image: trezorIcon,
    imageFrameClassName: lightLogoFrame,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "coldcard",
    title: "Coldcard",
    description: "Coldcard skeleton wallet or descriptor import.",
    category: "wallets",
    image: coldcardIcon,
    imageFrameClassName: lightLogoFrame,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    formatLabel: "skeleton/descriptor",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "ledger",
    title: "Ledger",
    description: "Ledger Live account export.",
    category: "wallets",
    image: ledgerIcon,
    imageFrameClassName: lightLogoFrame,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    details: ["Use generic CSV until a dedicated Ledger Live parser lands"],
  },
  {
    id: "foundation-passport",
    title: "Foundation Passport",
    description: "Passport wallet export or descriptor import.",
    category: "wallets",
    image: foundationPassportIcon,
    imageFrameClassName: lightLogoFrame,
    imageClassName: "size-9",
    status: "planned",
    pathLabel: "Wallet export",
    details: ["Use the Descriptor connection for exported descriptors today"],
  },
  {
    id: "core-ln",
    title: "Core Lightning",
    description: "Read-only CLN bookkeeper and node accounting sync.",
    category: "lightning",
    image: coreLightningIcon,
    imageFrameClassName: "bg-[#494120]",
    imageClassName: "size-8",
    status: "ready",
    pathLabel: "Lightning node",
    formatLabel: "coreln",
    setupKind: "backend-settings",
    walletKind: "coreln",
    details: [
      "Prefer restricted commando runes",
      "Syncs bookkeeper events, forwards, payments, invoices, channels, and node wallet rows",
      "Reports routing profit and per-channel break-even status",
    ],
  },
  {
    id: "lnd",
    title: "LND",
    description: "Lightning Network Daemon read-only profitability data.",
    category: "lightning",
    image: lightningLabsIcon,
    imageFrameClassName: "bg-neutral-950",
    imageClassName: "size-8",
    status: "ready",
    pathLabel: "Lightning node",
    formatLabel: "LND REST",
    setupKind: "backend-settings",
    walletKind: "lnd",
    details: [
      "Stores host, TLS certificate, and read-only macaroon presence",
      "Reads channels, forwards, payments, invoices, and fee snapshots; drops preimages, encoded bolt11 strings, route hops, and route hints at the boundary",
    ],
  },
  {
    id: "zeus",
    title: "ZEUS",
    description: "Lightning wallet and node app export.",
    category: "lightning",
    image: lightningIcon,
    imageClassName: "size-8",
    status: "planned",
    pathLabel: "Lightning wallet",
    details: ["Use Phoenix CSV today when the activity comes from Phoenix"],
  },
  {
    id: "phoenix",
    title: "Phoenix",
    description: "Lightning wallet CSV activity import.",
    category: "lightning",
    image: sourceIcon("PHX", "#6d28d9", "#ffffff"),
    status: "ready",
    pathLabel: "CSV import",
    formatLabel: "phoenix_csv",
    setupKind: "file-wallet",
    walletKind: "phoenix",
    sourceFormat: "phoenix_csv",
    details: [
      "Signed msat amounts drive direction",
      "Phoenix type is preserved as a tag",
    ],
  },
  {
    id: "bullbitcoin-wallet",
    title: "Bull Bitcoin Wallet",
    description: "Unified wallet transaction CSV import.",
    category: "wallets",
    image: bullBitcoinIcon,
    imageClassName: "size-9 rounded-md",
    status: "ready",
    pathLabel: "Wallet CSV import",
    formatLabel: "bullbitcoin_wallet_csv",
    docsHref: "https://github.com/SatoshiPortal/bullbitcoin-mobile/issues/1363",
    setupKind: "bullbitcoin-wallet",
    walletKind: "bullbitcoin",
    sourceFormat: "bullbitcoin_wallet_csv",
    details: [
      "Imports Bitcoin, Liquid, Lightning, payjoin, and swap wallet rows",
      "Use descriptor wallets instead when they are your source of truth",
      "Bull order CSV remains available under Exchanges for exact fiat pricing",
    ],
  },
  {
    id: "btcpay",
    title: "BTCPay Server",
    description: "Store wallet history through a scoped API key.",
    category: "merchant",
    image: btcpayIcon,
    imageClassName: "h-9 w-auto",
    status: "ready",
    pathLabel: "Greenfield API",
    formatLabel: "confirmed wallet history",
    docsHref: "https://docs.btcpayserver.org/Development/GreenFieldExample/",
    setupKind: "btcpay",
    details: [
      "Save or reuse a BTCPay instance",
      "Map payment methods to Kassiber wallets",
    ],
  },
  {
    id: "river",
    title: "River",
    description: "Bitcoin Activity or Account Activity CSV import.",
    category: "exchanges",
    image: sourceIcon("RV", "#1e3a8a", "#ffffff"),
    status: "ready",
    pathLabel: "CSV import",
    formatLabel: "river_csv",
    docsHref:
      "https://support.river.com/hc/en-us/articles/45513824178963-How-do-I-download-my-account-activity",
    setupKind: "file-wallet",
    walletKind: "river",
    sourceFormat: "river_csv",
    details: [
      "Account Activity preserves BTC and cash legs",
      "Buy/sell rows store exact River execution pricing",
    ],
  },
  {
    id: "bullbitcoin",
    title: "Bull Bitcoin",
    description: "Order CSV import for exact buy/sell execution pricing.",
    category: "exchanges",
    image: bullBitcoinIcon,
    imageClassName: "size-9 rounded-md",
    status: "ready",
    pathLabel: "CSV import",
    formatLabel: "bullbitcoin_csv",
    docsHref: "https://www.bullbitcoin.com/",
    setupKind: "file-enrichment",
    walletKind: "bullbitcoin",
    sourceFormat: "bullbitcoin_csv",
    details: [
      "Completed Bitcoin, Lightning, and Liquid orders preserve exact fiat proceeds",
      "Book-wide imports can enrich relevant rows or import the shared export with reconciliation flags",
    ],
  },
  {
    id: "relai",
    title: "Relai",
    description: "Bitcoin-only app activity import.",
    category: "exchanges",
    image: relaiIcon,
    imageClassName: "size-9 rounded-md",
    status: "planned",
    pathLabel: "CSV import",
    docsHref:
      "https://support.relai.app/en/articles/194348-how-do-i-export-my-order-history",
    details: ["Order-history export exists; dedicated parser is not wired yet"],
  },
  {
    id: "pocket-bitcoin",
    title: "Pocket Bitcoin",
    description: "Bitcoin-only broker activity import.",
    category: "exchanges",
    image: sourceIcon("PKT", "#facc15", "#111827"),
    status: "planned",
    pathLabel: "CSV import",
    details: ["Dedicated parser is not wired yet"],
  },
  {
    id: "swan-bitcoin",
    title: "Swan Bitcoin",
    description: "Bitcoin-only savings and broker activity import.",
    category: "exchanges",
    image: sourceIcon("SW", "#111827", "#ffffff"),
    status: "planned",
    pathLabel: "CSV import",
    details: ["Dedicated parser is not wired yet"],
  },
  {
    id: "strike",
    title: "Strike",
    description: "Custodial Bitcoin wallet and exchange import.",
    category: "exchanges",
    image: strikeIcon,
    imageClassName: "size-8 rounded-lg",
    status: "ready",
    pathLabel: "Custodial platform",
    formatLabel: "strike_csv",
    docsHref: "https://strike.me/",
    setupKind: "file-wallet",
    walletKind: "strike",
    sourceFormat: "strike_csv",
    details: [
      "BTC buys, sells, Lightning, and on-chain rows become active platform activity",
      "Fiat-only funding and reversal rows are skipped",
      "Rows use Strike BTC Price as exact CSV pricing when present",
    ],
  },
  {
    id: "21bitcoin",
    title: "21bitcoin",
    description: "Custodial platform ledger import with exact trade pricing.",
    category: "exchanges",
    image: twentyOneBitcoinIcon,
    imageClassName: "size-8 rounded-md",
    status: "ready",
    pathLabel: "CSV import",
    formatLabel: "21bitcoin_csv",
    docsHref: "https://21bitcoin.app/",
    setupKind: "file-wallet",
    walletKind: "21bitcoin",
    sourceFormat: "21bitcoin_csv",
    details: [
      "BTC trade rows become active custodial balance activity",
      "Buy/sell rows store exact 21bitcoin execution pricing from the CSV",
      "L1 withdrawal rows can be paired to your receiving wallet so basis carries out",
    ],
  },
  {
    id: "coinfinity",
    title: "Coinfinity",
    description: "Order CSV import for exact buy/sell execution pricing.",
    category: "exchanges",
    image: coinfinityIcon,
    imageFrameClassName: lightLogoFrame,
    imageClassName: "size-9 rounded-lg",
    status: "ready",
    pathLabel: "CSV import",
    formatLabel: "coinfinity_csv",
    docsHref: "https://coinfinity.co/",
    setupKind: "file-enrichment",
    walletKind: "coinfinity",
    sourceFormat: "coinfinity_csv",
    details: [
      "BTC/EUR order rows preserve exact Coinfinity execution pricing",
      "Book-wide imports can enrich relevant rows or import the shared export with reconciliation flags",
    ],
  },
  {
    id: "bitpanda",
    title: "Bitpanda",
    description: "BTC rows from Bitpanda history exports.",
    category: "exchanges",
    image: bitpandaIcon,
    imageFrameClassName: "bg-[#103e36]",
    imageClassName: "h-9 w-auto",
    status: "planned",
    pathLabel: "CSV/API import",
    docsHref:
      "https://support.bitpanda.com/hc/en-us/articles/360000122759-How-can-I-download-the-history-of-my-Bitpanda-account",
    details: ["History export exists; dedicated BTC parser is not wired yet"],
  },
  {
    id: "kraken",
    title: "Kraken",
    description: "BTC rows from Kraken ledger and trade exports.",
    category: "exchanges",
    image: krakenIcon,
    imageClassName: "size-8",
    status: "planned",
    pathLabel: "Ledger/trade CSV",
    docsHref:
      "https://support.kraken.com/articles/360001169383-how-to-interpret-ledger-history-fields",
    details: ["Needs multi-row trade pairing before cost basis can be trusted"],
  },
  {
    id: "coinbase",
    title: "Coinbase",
    description: "BTC rows from Coinbase account activity exports.",
    category: "exchanges",
    image: coinbaseIcon,
    imageClassName: "size-8",
    status: "planned",
    pathLabel: "CSV/API import",
    details: ["Dedicated BTC parser is not wired yet"],
  },
  {
    id: "generic-ledger",
    title: "Generic ledger",
    description: "Fill-in spreadsheet import for manual transactions.",
    category: "files",
    image: sourceIcon("XLS", "#f2a900", "#1a1a1a"),
    icon: FileSpreadsheet,
    status: "ready",
    pathLabel: "Spreadsheet import",
    formatLabel: "Excel (.xlsx) or CSV",
    setupKind: "file-wallet",
    walletKind: "custom",
    sourceFormat: "generic_ledger",
    details: [
      "Download the template, fill in your transactions, import the file",
      "One Bitcoin leg per row; Buy/Sell/Income/Mining/Gift and more",
      "Amounts in BTC (or SATS); fiat columns in your book's currency",
    ],
  },
  {
    id: "csv",
    title: "CSV import",
    description: "One-shot import from a local file.",
    category: "files",
    image: sourceIcon("CSV", "#64748b", "#ffffff"),
    status: "ready",
    pathLabel: "CSV/JSON import",
    formatLabel: "generic csv/json",
    setupKind: "file-wallet",
    walletKind: "custom",
    sourceFormat: "csv",
    details: [
      "Use Kassiber's generic transaction columns",
      "Specific parsers are preferred when available",
    ],
  },
  {
    id: "bip329",
    title: "BIP329 labels",
    description: "JSONL wallet label import and export.",
    category: "files",
    image: sourceIcon("LBL", "#475569", "#ffffff"),
    icon: Tags,
    status: "ready",
    pathLabel: "Label import",
    formatLabel: "bip329 JSONL",
    docsHref: "https://bips.xyz/329",
    setupKind: "bip329",
    details: ["Labels are stored locally and bridged to matching transactions"],
  },
];
