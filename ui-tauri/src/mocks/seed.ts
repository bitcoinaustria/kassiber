/**
 * Mock data seeded from claude-design's MOCK constant in components/strings.jsx.
 *
 * These values exist only to drive the UI translation against realistic
 * shapes until the Pydantic→JSON Schema pipeline (Phase 1.2 §2.2) generates
 * fixtures from real `kassiber.core.api.contracts` models. At that point the
 * shapes here become test cases for the schema and these literals get
 * replaced with schema-driven factories.
 */

export type ConnectionStatus = "synced" | "syncing" | "idle" | "error";

export type ConnectionKind =
  | "xpub"
  | "address"
  | "descriptor"
  | "silent-payment"
  | "samourai"
  | "core-ln"
  | "lnd"
  | "nwc"
  | "cashu"
  | "btcpay"
  | "kraken"
  | "bitstamp"
  | "coinbase"
  | "bitpanda"
  | "river"
  | "bullbitcoin"
  | "coinfinity"
  | "strike"
  | "phoenix"
  | "custom"
  | "csv"
  | "bip329"
  | "backend";

export interface Connection {
  id: string;
  kind: ConnectionKind;
  role?: "wallet" | "backend";
  /** Wallet chain ("bitcoin" | "liquid") from the daemon snapshot. */
  chain?: string | null;
  label: string;
  last: string;
  lastSyncAt?: string | null;
  lastTransactionAt?: string | null;
  asset?: string | null;
  network?: string | null;
  policyAsset?: string | null;
  paymentMethodId?: string | null;
  /** balance in BTC (float) */
  balance: number;
  status: ConnectionStatus;
  syncMode?: string;
  syncSource?: string;
  sourceFormat?: string;
  deprecated?: boolean;
  backendId?: string;
  backendKind?: string | null;
  endpoint?: string | null;
  isDefaultBackend?: boolean;
  settingsHash?: string;
  walletRefs?: string[];
  transactionCount?: number;
  addresses?: number;
  gap?: number;
  channels?: number;
  lightningCapabilities?: LightningCapabilities;
  node?: NodeSnapshot;
}

export interface LightningCapabilities {
  nodeSnapshot: boolean;
  routingProfitability: boolean;
  channelBalances: boolean;
  channelLifecycle: boolean;
  forwardEvents: boolean;
  invoiceActivity: boolean;
  paymentActivity: boolean;
  onchainBalance: boolean;
}

export type NodeChannelState =
  | "active"
  | "inactive"
  | "pending_open"
  | "pending_close"
  | "closed"
  | "force_closed";

export interface NodeChannel {
  id: string;
  /** short channel id or temporary id for pending channels */
  shortChannelId?: string | null;
  /** funding outpoint (txid:vout) */
  fundingOutpoint?: string | null;
  peerAlias: string;
  /**
   * Hex node id of the channel peer. `null` for private channels by
   * default (the peer chose private gossip for a reason). Adapters that
   * surface a private-channel peer id must do so on explicit operator
   * opt-in.
   */
  peerPubkey: string | null;
  /** total channel capacity in sats */
  capacitySat: number;
  /** sats currently spendable from this node */
  localBalanceSat: number;
  /** sats currently spendable by the remote */
  remoteBalanceSat: number;
  state: NodeChannelState;
  isPrivate: boolean;
  isInitiator: boolean;
  /** base routing fee in millisat */
  baseFeeMsat?: number;
  /** proportional routing fee in parts-per-million */
  feeRatePpm?: number;
  /** UTC ISO of opening event */
  openedAt?: string | null;
  /** UTC ISO of closing event (closed/force-closed channels) */
  closedAt?: string | null;
  closeKind?: "cooperative" | "force" | "breach" | null;
  /** number of forwards routed through this channel in the snapshot window */
  forwardCount?: number;
  /** sats earned routing through this channel in the snapshot window */
  earnedRoutingSat?: number;
  /** current in-flight HTLC count on this channel */
  htlcCount?: number;
  /** UTC ISO of the last forward/payment that touched this channel */
  lastActivityAt?: string | null;
}

export type NodeForwardStatus = "settled" | "failed" | "offered";

/**
 * Categorical failure reasons mirroring
 * `kassiber.core.lightning.types.NodeForwardFailureReason`. Kept
 * categorical so adapters cannot smuggle raw node error strings
 * (which may include `failure_source_pubkey`, payment hashes, or
 * route-hint JSON) through what otherwise would look like a free-text
 * field. See `docs/reference/lightning-opsec.md`.
 */
export type NodeForwardFailureReason =
  | "temporary_channel_failure"
  | "unknown_next_peer"
  | "fee_insufficient"
  | "incorrect_payment_details"
  | "expiry_too_soon"
  | "insufficient_balance"
  | "other";

export interface NodeForward {
  id: string;
  /** UTC ISO timestamp */
  occurredAt: string;
  inPeerAlias: string;
  inShortChannelId?: string | null;
  outPeerAlias: string;
  outShortChannelId?: string | null;
  /** incoming amount in millisat */
  amountInMsat: number;
  /** outgoing amount in millisat (amountInMsat - feeMsat for settled forwards) */
  amountOutMsat: number;
  /** earned routing fee in millisat */
  feeMsat: number;
  status: NodeForwardStatus;
  failureReason?: NodeForwardFailureReason | null;
}

export interface NodeRoutingSnapshot {
  /** "Last 30 days" etc. */
  windowLabel: string;
  routingRevenueSat: number;
  paymentCostSat: number;
  rebalanceCostSat: number;
  onchainCostSat: number;
  netProfitSat: number;
  forwardCount: number;
  paymentCount: number;
  rebalanceCount: number;
}

export interface NodeSnapshot {
  alias: string;
  pubkey: string;
  /** mainnet / testnet / signet / regtest */
  network: string;
  /** "v0.18.0" or similar */
  implementationVersion?: string;
  /** number of active peers */
  peerCount: number;
  blockHeight?: number;
  invoiceCount?: number;
  paidInvoiceCount?: number;
  expiredInvoiceCount?: number;
  paymentCount?: number;
  completedPaymentCount?: number;
  failedPaymentCount?: number;
  /** sats sittable on-chain that the node owns */
  onchainBalanceSat: number;
  /** sum of localBalanceSat across active + inactive channels (excludes closed and pending) */
  totalLocalBalanceSat: number;
  /** sum of remoteBalanceSat across active + inactive channels (excludes closed and pending) */
  totalRemoteBalanceSat: number;
  /** sum of capacitySat across active + inactive channels (excludes closed and pending) */
  totalCapacitySat: number;
  channels: NodeChannel[];
  /** appended for collapsed display; may be empty */
  closedChannels?: NodeChannel[];
  routing?: NodeRoutingSnapshot;
  /** recent forwards routed through the node; newest first */
  forwards?: NodeForward[];
}

export type TxType =
  | "Income"
  | "Expense"
  | "Transfer"
  | "Fee"
  | "Swap"
  | "Mint"
  | "Melt"
  | "Consolidation"
  | "Rebalance";

export interface Tx {
  id: string;
  externalId?: string;
  explorerId?: string;
  date: string;
  occurredAt?: string;
  type: TxType;
  asset?: string | null;
  chain?: string | null;
  network?: string | null;
  account: string;
  counter: string;
  amountSat: number;
  feeSat?: number;
  eur: number | null;
  rate: number | null;
  fiatCurrency?: string | null;
  pricingSourceKind?: string | null;
  pricingQuality?: string | null;
  pricingExternalRef?: string | null;
  pricingProvider?: string | null;
  pricingPair?: string | null;
  pricingTimestamp?: string | null;
  pricingFetchedAt?: string | null;
  pricingGranularity?: string | null;
  pricingMethod?: string | null;
  reviewStatus?: string | null;
  taxable?: boolean | null;
  atRegime?: string | null;
  atCategory?: string | null;
  tag: string;
  tags?: string[];
  note?: string;
  excluded?: boolean;
  quarantineReason?: string | null;
  pair?: {
    id: string;
    type: "transfer" | "swap";
    kind?: string | null;
    policy?: string | null;
    outWallet?: string | null;
    outAsset?: string | null;
    outAmountSat?: number;
    inWallet?: string | null;
    inAsset?: string | null;
    inAmountSat?: number;
    feeSat?: number;
    feeKind?: string | null;
  };
  conf: number;
  internal?: boolean;
  balanceBtc?: number;
  costBasisEur?: number;
}

export interface FiatSnapshot {
  fiatCurrency?: string | null;
  eurBalance: number;
  eurCostBasis: number;
  eurUnrealized: number;
  eurRealizedYTD: number;
}

export interface PortfolioPoint {
  date: string;
  label: string;
  balanceBtc: number;
  valueEur: number;
  costBasisEur: number;
  priceEur?: number;
  priceTimestamp?: string | null;
  priceSource?: string | null;
}

export interface MarketRateSnapshot {
  asset: "BTC";
  fiatCurrency: string;
  pair: string | null;
  rate: number | null;
  timestamp: string | null;
  source: string | null;
  fetchedAt: string | null;
  granularity: string | null;
  method: string | null;
}

export interface OverviewSnapshot {
  priceEur: number;
  priceUsd: number;
  marketRate?: MarketRateSnapshot;
  connections: Connection[];
  activityTxs?: Tx[];
  txs: Tx[];
  /** monthly-ish BTC totals across the span */
  balanceSeries: number[];
  /** dated portfolio points from the daemon, using real source dates/rates */
  portfolioSeries?: PortfolioPoint[];
  fiat: FiatSnapshot;
  status?: {
    workspace: string | null;
    profile: string | null;
    transactionCount?: number;
    needsJournals: boolean;
    quarantines: number;
  };
}

const minutesAgoIso = (minutes: number) =>
  new Date(Date.now() - minutes * 60_000).toISOString();

const MOCK_MARKET_RATE_FETCHED_AT = minutesAgoIso(2);

export const MOCK_OVERVIEW: OverviewSnapshot = {
  priceEur: 71_420.18,
  priceUsd: 76_597.49,
  marketRate: {
    asset: "BTC",
    fiatCurrency: "EUR",
    pair: "BTC-EUR",
    rate: 71_420.18,
    timestamp: MOCK_MARKET_RATE_FETCHED_AT,
    source: "coinbase-exchange",
    fetchedAt: MOCK_MARKET_RATE_FETCHED_AT,
    granularity: "60",
    method: "close",
  },
  connections: [
    {
      id: "c1",
      kind: "xpub",
      label: "Cold Storage",
      last: "2m ago",
      lastSyncAt: "2026-06-26T11:58:03Z",
      chain: "bitcoin",
      network: "main",
      balance: 1.24810472,
      status: "synced",
      addresses: 142,
      gap: 40,
    },
    {
      id: "c2",
      kind: "descriptor",
      label: "Multisig 2/3 Vault",
      last: "2m ago",
      lastSyncAt: "2026-06-26T11:57:57Z",
      chain: "bitcoin",
      network: "main",
      balance: 3.0814290,
      status: "synced",
      addresses: 86,
      gap: 40,
    },
    {
      id: "c3",
      kind: "core-ln",
      label: "Home Node (CLN)",
      last: "18s ago",
      lastSyncAt: "2026-06-26T11:59:42Z",
      balance: 0.04821309,
      status: "synced",
      channels: 12,
      lightningCapabilities: {
        nodeSnapshot: true,
        routingProfitability: true,
        channelBalances: true,
        channelLifecycle: true,
        forwardEvents: true,
        invoiceActivity: true,
        paymentActivity: true,
        onchainBalance: true,
      },
      node: {
        alias: "kassiber-home",
        pubkey:
          "03f3c108ccd536b8526841f0a5c58212bb9e6584a1eb493080e7c1cc34f82dad71",
        network: "mainnet",
        implementationVersion: "Core Lightning v24.11",
        peerCount: 18,
        blockHeight: 884_212,
        onchainBalanceSat: 1_205_000,
        totalLocalBalanceSat: 2_812_309,
        totalRemoteBalanceSat: 2_010_000,
        totalCapacitySat: 4_822_309,
        channels: [
          {
            id: "ch1",
            shortChannelId: "884011x412x0",
            fundingOutpoint:
              "8a9c4e7b6f5a3d2c1e0f9988776655443322110099aabbccddeeff0011223344:0",
            peerAlias: "ACINQ",
            peerPubkey:
              "03864ef025fde8fb587d989186ce6a4a186895ee44a926bfc370e2c366597a3f8f",
            capacitySat: 2_000_000,
            localBalanceSat: 1_312_500,
            remoteBalanceSat: 687_500,
            state: "active",
            isPrivate: false,
            isInitiator: true,
            baseFeeMsat: 1_000,
            feeRatePpm: 250,
            openedAt: "2025-09-21T14:22:11Z",
            forwardCount: 142,
            earnedRoutingSat: 11_240,
            htlcCount: 2,
            lastActivityAt: "2026-05-18T07:42:18Z",
          },
          {
            id: "ch2",
            shortChannelId: "884088x77x1",
            fundingOutpoint:
              "7f6e5d4c3b2a1908ffeeddccbbaa9988776655443322110099aabbccddeeff00:1",
            peerAlias: "Boltz",
            peerPubkey:
              "026165850492521f4ac8abd9bd8088123446d126f648ca35e60f88177dc149ceb2",
            capacitySat: 1_500_000,
            localBalanceSat: 925_000,
            remoteBalanceSat: 575_000,
            state: "active",
            isPrivate: false,
            isInitiator: true,
            baseFeeMsat: 0,
            feeRatePpm: 100,
            openedAt: "2025-10-04T09:14:00Z",
            forwardCount: 96,
            earnedRoutingSat: 4_812,
            htlcCount: 0,
            lastActivityAt: "2026-05-18T06:11:02Z",
          },
          {
            id: "ch3",
            shortChannelId: "884150x18x0",
            fundingOutpoint:
              "6e5d4c3b2a190877665544332211ffeeddccbbaa00998877665544332211aabb:0",
            peerAlias: "WalletOfSatoshi.com",
            peerPubkey:
              "035e4ff418fc8b5554c5d9eea66396c227bd429a3251c8cbc711002ba215bfc226",
            capacitySat: 800_000,
            localBalanceSat: 312_000,
            remoteBalanceSat: 488_000,
            state: "active",
            isPrivate: false,
            isInitiator: false,
            baseFeeMsat: 1_000,
            feeRatePpm: 200,
            openedAt: "2025-11-12T18:42:30Z",
            forwardCount: 38,
            earnedRoutingSat: 1_204,
            htlcCount: 1,
            lastActivityAt: "2026-05-17T22:30:00Z",
          },
          {
            id: "ch4",
            shortChannelId: "884188x42x2",
            fundingOutpoint:
              "5d4c3b2a190877665544332211ffeeddccbbaa00998877665544332211aabbcc:2",
            peerAlias: "LNBig.com [lnd-22]",
            peerPubkey:
              "0298f6074a454a1f5345cb2a7c6f9fce206cd0bf675d177cdbf0ca7508dd28852d",
            capacitySat: 522_309,
            localBalanceSat: 262_809,
            remoteBalanceSat: 259_500,
            state: "inactive",
            isPrivate: false,
            isInitiator: true,
            baseFeeMsat: 1_000,
            feeRatePpm: 300,
            openedAt: "2025-12-02T11:09:00Z",
            forwardCount: 12,
            earnedRoutingSat: 420,
            htlcCount: 0,
            lastActivityAt: "2026-05-11T14:08:42Z",
          },
          {
            id: "ch5",
            shortChannelId: null,
            fundingOutpoint:
              "4c3b2a190877665544332211ffeeddccbbaa00998877665544332211aabbccdd:0",
            peerAlias: "Bitrefill",
            // Private channel — adapter would not expose the peer pubkey
            // by default (the peer chose private gossip).
            peerPubkey: null,
            capacitySat: 500_000,
            localBalanceSat: 500_000,
            remoteBalanceSat: 0,
            state: "pending_open",
            isPrivate: true,
            isInitiator: true,
            openedAt: "2026-04-18T12:14:00Z",
            forwardCount: 0,
            earnedRoutingSat: 0,
          },
        ],
        closedChannels: [
          {
            id: "ch_closed_1",
            shortChannelId: "883102x88x0",
            fundingOutpoint:
              "3b2a190877665544332211ffeeddccbbaa00998877665544332211aabbccddee:0",
            peerAlias: "old peer (alias unknown)",
            peerPubkey:
              "02d8f3b6d3a4be2bdc6e0a7c45ed6cc8c39ce14b5c14ba38eb1f0ad0a2b3c4d5e6",
            capacitySat: 1_000_000,
            localBalanceSat: 0,
            remoteBalanceSat: 0,
            state: "closed",
            isPrivate: false,
            isInitiator: true,
            openedAt: "2025-04-09T08:00:00Z",
            closedAt: "2026-01-21T11:08:00Z",
            closeKind: "cooperative",
            forwardCount: 0,
            earnedRoutingSat: 0,
          },
          {
            id: "ch_closed_2",
            shortChannelId: "882011x12x1",
            fundingOutpoint:
              "2a190877665544332211ffeeddccbbaa00998877665544332211aabbccddeeff:1",
            peerAlias: "neighbor.node",
            peerPubkey:
              "021a2b3c4d5e6f70819293a4b5c6d7e8f90011223344556677889900aabbccdd1e",
            capacitySat: 750_000,
            localBalanceSat: 0,
            remoteBalanceSat: 0,
            state: "force_closed",
            isPrivate: false,
            isInitiator: false,
            openedAt: "2025-03-14T07:00:00Z",
            closedAt: "2025-12-04T22:42:00Z",
            closeKind: "force",
            forwardCount: 0,
            earnedRoutingSat: 0,
          },
        ],
        routing: {
          windowLabel: "Last 30 days",
          routingRevenueSat: 17_676,
          paymentCostSat: 1_412,
          rebalanceCostSat: 880,
          onchainCostSat: 4_210,
          netProfitSat: 11_174,
          forwardCount: 288,
          paymentCount: 41,
          rebalanceCount: 4,
        },
        forwards: [
          {
            id: "fw_cln_1",
            occurredAt: "2026-05-18T07:42:18Z",
            inPeerAlias: "ACINQ",
            inShortChannelId: "884011x412x0",
            outPeerAlias: "Boltz",
            outShortChannelId: "884088x77x1",
            amountInMsat: 240_120_000,
            amountOutMsat: 240_000_000,
            feeMsat: 120_000,
            status: "settled",
          },
          {
            id: "fw_cln_2",
            occurredAt: "2026-05-18T06:11:02Z",
            inPeerAlias: "WalletOfSatoshi.com",
            inShortChannelId: "884150x18x0",
            outPeerAlias: "ACINQ",
            outShortChannelId: "884011x412x0",
            amountInMsat: 18_540_000,
            amountOutMsat: 18_500_000,
            feeMsat: 40_000,
            status: "settled",
          },
          {
            id: "fw_cln_3",
            occurredAt: "2026-05-17T22:30:00Z",
            inPeerAlias: "Boltz",
            inShortChannelId: "884088x77x1",
            outPeerAlias: "WalletOfSatoshi.com",
            outShortChannelId: "884150x18x0",
            amountInMsat: 95_220_000,
            amountOutMsat: 95_200_000,
            feeMsat: 20_000,
            status: "settled",
          },
          {
            id: "fw_cln_4",
            occurredAt: "2026-05-17T19:08:44Z",
            inPeerAlias: "ACINQ",
            inShortChannelId: "884011x412x0",
            outPeerAlias: "LNBig.com [lnd-22]",
            outShortChannelId: "884188x42x2",
            amountInMsat: 412_000_000,
            amountOutMsat: 0,
            feeMsat: 0,
            status: "failed",
            failureReason: "temporary_channel_failure",
          },
          {
            id: "fw_cln_5",
            occurredAt: "2026-05-17T15:22:12Z",
            inPeerAlias: "Boltz",
            inShortChannelId: "884088x77x1",
            outPeerAlias: "ACINQ",
            outShortChannelId: "884011x412x0",
            amountInMsat: 1_204_500_000,
            amountOutMsat: 1_204_000_000,
            feeMsat: 500_000,
            status: "settled",
          },
          {
            id: "fw_cln_6",
            occurredAt: "2026-05-17T11:04:18Z",
            inPeerAlias: "WalletOfSatoshi.com",
            inShortChannelId: "884150x18x0",
            outPeerAlias: "Boltz",
            outShortChannelId: "884088x77x1",
            amountInMsat: 6_180_000,
            amountOutMsat: 6_180_000,
            feeMsat: 0,
            status: "offered",
          },
          {
            id: "fw_cln_7",
            occurredAt: "2026-05-16T20:48:00Z",
            inPeerAlias: "ACINQ",
            inShortChannelId: "884011x412x0",
            outPeerAlias: "Boltz",
            outShortChannelId: "884088x77x1",
            amountInMsat: 82_330_000,
            amountOutMsat: 82_300_000,
            feeMsat: 30_000,
            status: "settled",
          },
        ],
      },
    },
    {
      id: "c4",
      kind: "nwc",
      label: "Alby Hub",
      last: "1h ago",
      lastSyncAt: "2026-06-26T11:00:00Z",
      balance: 0.00213500,
      status: "idle",
    },
    {
      id: "c5",
      kind: "cashu",
      label: "minibits.cash",
      last: "3h ago",
      lastSyncAt: "2026-06-26T09:00:00Z",
      balance: 0.00019823,
      status: "synced",
    },
    {
      id: "c6",
      kind: "lnd",
      label: "lnd_merchant_backup",
      last: "44s ago",
      lastSyncAt: "2026-06-26T11:59:16Z",
      balance: 0.02914872,
      status: "syncing",
      channels: 7,
      lightningCapabilities: {
        nodeSnapshot: true,
        routingProfitability: true,
        channelBalances: true,
        channelLifecycle: true,
        forwardEvents: true,
        invoiceActivity: true,
        paymentActivity: true,
        onchainBalance: true,
      },
      node: {
        alias: "kassiber-routing",
        pubkey:
          "02a14b7c5d9e0f1234567890abcdef00112233445566778899aabbccddeeff0011",
        network: "mainnet",
        implementationVersion: "lnd 0.18.4-beta",
        peerCount: 11,
        blockHeight: 884_209,
        onchainBalanceSat: 612_500,
        totalLocalBalanceSat: 1_840_000,
        totalRemoteBalanceSat: 1_460_372,
        totalCapacitySat: 3_300_372,
        channels: [
          {
            id: "lch1",
            shortChannelId: "884099x501x0",
            fundingOutpoint:
              "1f2e3d4c5b6a798877665544332211ffeeddccbbaa00998877665544332211aa:0",
            peerAlias: "deezy.io",
            peerPubkey:
              "024bfaf0cabe7f874fd33ebf7c6f4e5d3c2b1a09081726354455667788990011aa",
            capacitySat: 1_500_000,
            localBalanceSat: 940_000,
            remoteBalanceSat: 560_000,
            state: "active",
            isPrivate: false,
            isInitiator: true,
            baseFeeMsat: 1_000,
            feeRatePpm: 150,
            openedAt: "2025-08-30T13:14:00Z",
            forwardCount: 204,
            earnedRoutingSat: 8_122,
            htlcCount: 3,
            lastActivityAt: "2026-05-18T08:01:09Z",
          },
          {
            id: "lch2",
            shortChannelId: "884121x18x2",
            fundingOutpoint:
              "2e3d4c5b6a798877665544332211ffeeddccbbaa00998877665544332211aabb:2",
            peerAlias: "Voltage Cloud",
            peerPubkey:
              "030115273849af5d6c7e8f90112233445566778899aabbccddeeff0011223344cc",
            capacitySat: 1_000_000,
            localBalanceSat: 612_000,
            remoteBalanceSat: 388_000,
            state: "active",
            isPrivate: false,
            isInitiator: true,
            baseFeeMsat: 1_000,
            feeRatePpm: 200,
            openedAt: "2025-09-18T08:00:00Z",
            forwardCount: 118,
            earnedRoutingSat: 3_902,
            htlcCount: 1,
            lastActivityAt: "2026-05-18T07:11:42Z",
          },
          {
            id: "lch3",
            shortChannelId: "884170x77x1",
            fundingOutpoint:
              "3d4c5b6a798877665544332211ffeeddccbbaa00998877665544332211aabbcc:1",
            peerAlias: "Olympus by ZEUS",
            // Private channel — pubkey withheld by default per opsec policy.
            peerPubkey: null,
            capacitySat: 800_372,
            localBalanceSat: 288_000,
            remoteBalanceSat: 512_372,
            state: "active",
            isPrivate: true,
            isInitiator: false,
            baseFeeMsat: 1_000,
            feeRatePpm: 350,
            openedAt: "2025-11-04T21:30:00Z",
            forwardCount: 51,
            earnedRoutingSat: 1_840,
            htlcCount: 0,
            lastActivityAt: "2026-05-17T18:50:21Z",
          },
        ],
        closedChannels: [],
        routing: {
          windowLabel: "Last 30 days",
          routingRevenueSat: 13_864,
          paymentCostSat: 942,
          rebalanceCostSat: 1_204,
          onchainCostSat: 2_180,
          netProfitSat: 9_538,
          forwardCount: 372,
          paymentCount: 28,
          rebalanceCount: 6,
        },
        forwards: [
          {
            id: "fw_lnd_1",
            occurredAt: "2026-05-18T08:01:09Z",
            inPeerAlias: "deezy.io",
            inShortChannelId: "884099x501x0",
            outPeerAlias: "Voltage Cloud",
            outShortChannelId: "884121x18x2",
            amountInMsat: 320_080_000,
            amountOutMsat: 320_000_000,
            feeMsat: 80_000,
            status: "settled",
          },
          {
            id: "fw_lnd_2",
            occurredAt: "2026-05-18T07:11:42Z",
            inPeerAlias: "Voltage Cloud",
            inShortChannelId: "884121x18x2",
            outPeerAlias: "Olympus by ZEUS",
            outShortChannelId: "884170x77x1",
            amountInMsat: 142_350_000,
            amountOutMsat: 142_300_000,
            feeMsat: 50_000,
            status: "settled",
          },
          {
            id: "fw_lnd_3",
            occurredAt: "2026-05-17T18:50:21Z",
            inPeerAlias: "deezy.io",
            inShortChannelId: "884099x501x0",
            outPeerAlias: "Olympus by ZEUS",
            outShortChannelId: "884170x77x1",
            amountInMsat: 50_280_000,
            amountOutMsat: 50_250_000,
            feeMsat: 30_000,
            status: "settled",
          },
          {
            id: "fw_lnd_4",
            occurredAt: "2026-05-17T14:08:00Z",
            inPeerAlias: "Olympus by ZEUS",
            inShortChannelId: "884170x77x1",
            outPeerAlias: "deezy.io",
            outShortChannelId: "884099x501x0",
            amountInMsat: 9_180_000,
            amountOutMsat: 0,
            feeMsat: 0,
            status: "failed",
            failureReason: "insufficient_balance",
          },
          {
            id: "fw_lnd_5",
            occurredAt: "2026-05-16T22:30:12Z",
            inPeerAlias: "Voltage Cloud",
            inShortChannelId: "884121x18x2",
            outPeerAlias: "deezy.io",
            outShortChannelId: "884099x501x0",
            amountInMsat: 612_400_000,
            amountOutMsat: 612_300_000,
            feeMsat: 100_000,
            status: "settled",
          },
        ],
      },
    },
  ],
  // Multi-year activity: on-chain and Lightning receipts, spends, channel
  // lifecycle and internal transfers spread across 2019→2026 so the ledger
  // reads like real history instead of a single burst. Newest first — the
  // overview surfaces slice the leading rows as "recent activity".
  txs: [
    { id: "tx1", externalId: "tx1", explorerId: "0000000000000000000000000000000000000000000000000000000000000001", date: "2026-04-18 14:22", type: "Income", account: "Cold Storage", counter: "Invoice · ACME GmbH", amountSat: 2_450_000, eur: 1749.79, rate: 71420.18, tag: "Revenue", conf: 41 },
    { id: "tx2", date: "2025-10-17 09:08", type: "Expense", account: "Home Node (CLN)", counter: "Server rental · Hetzner", amountSat: -120_431, eur: -105.98, rate: 88000.0, tag: "Hosting", conf: 140 },
    { id: "tx3", externalId: "tx3", explorerId: "0000000000000000000000000000000000000000000000000000000000000003", date: "2025-06-16 16:51", type: "Transfer", account: "Cold Storage → Vault", counter: "Internal transfer", amountSat: -50_000_000, eur: -41000.0, rate: 82000.0, tag: "Transfer", conf: 220, internal: true },
    { id: "tx4", date: "2024-12-15 11:14", type: "Income", account: "NWC · Alby", counter: "Client payment · LN", amountSat: 92_808, eur: 55.68, rate: 60000.0, tag: "Revenue", conf: 1 },
    { id: "tx5", date: "2024-05-14 22:02", type: "Expense", account: "Multisig Vault", counter: "Equipment · BitcoinStore", amountSat: -890_210, eur: -462.91, rate: 52000.0, tag: "Capex", conf: 420 },
    { id: "tx6", date: "2023-10-12 08:30", type: "Income", account: "Cold Storage", counter: "Sale · Consulting", amountSat: 3_800_000, eur: 1140.0, rate: 30000.0, tag: "Revenue", conf: 612 },
    { id: "tx7", date: "2023-04-11 19:45", type: "Expense", account: "Cashu · minibits", counter: "Coffee", amountSat: -8_400, eur: -2.18, rate: 26000.0, tag: "Meals", conf: 1 },
    { id: "tx8", date: "2022-09-09 10:00", type: "Fee", account: "Home Node (CLN)", counter: "Channel open", amountSat: -18_210, eur: -3.46, rate: 19000.0, tag: "Bank fees", conf: 380 },
    { id: "tx9", date: "2022-03-07 13:12", type: "Income", account: "Multisig Vault", counter: "Invoice · Globex AG", amountSat: 1_210_000, eur: 459.8, rate: 38000.0, tag: "Revenue", conf: 820 },
    { id: "tx10", date: "2021-06-06 15:30", type: "Swap", account: "NWC · Alby → Cashu · minibits", counter: "LN → ecash swap", amountSat: 500_000, eur: 210.0, rate: 42000.0, tag: "Swap", conf: 1 },
    { id: "tx11", date: "2020-11-05 11:08", type: "Swap", account: "Multisig Vault → Home Node (CLN)", counter: "Submarine swap · on-chain → LN", amountSat: 2_000_000, eur: 260.0, rate: 13000.0, tag: "Swap", conf: 12 },
    { id: "tx12", date: "2019-06-03 09:22", type: "Consolidation", account: "Cold Storage", counter: "12 UTXOs → 1", amountSat: 0, feeSat: 42_180, eur: -2.95, rate: 7000.0, tag: "Consolidation fee", conf: 210 },
  ],
  balanceSeries: [0.6, 0.9, 1.2, 1.6, 2.0, 2.5, 3.1, 3.5, 3.8, 4.1, 4.25, 4.38],
  portfolioSeries: [
    { date: "2019-06-30", label: "2019-06-30", balanceBtc: 0.6, valueEur: 4_200.0, costBasisEur: 3_000 },
    { date: "2019-12-31", label: "2019-12-31", balanceBtc: 0.9, valueEur: 5_760.0, costBasisEur: 4_600 },
    { date: "2020-12-31", label: "2020-12-31", balanceBtc: 1.2, valueEur: 28_800.0, costBasisEur: 12_000 },
    { date: "2021-12-31", label: "2021-12-31", balanceBtc: 1.6, valueEur: 73_600.0, costBasisEur: 30_000 },
    { date: "2022-12-31", label: "2022-12-31", balanceBtc: 2.0, valueEur: 33_000.0, costBasisEur: 45_000 },
    { date: "2023-12-31", label: "2023-12-31", balanceBtc: 2.5, valueEur: 95_000.0, costBasisEur: 68_000 },
    { date: "2024-12-31", label: "2024-12-31", balanceBtc: 3.1, valueEur: 170_500.0, costBasisEur: 95_000 },
    { date: "2025-06-30", label: "2025-06-30", balanceBtc: 3.5, valueEur: 287_000.0, costBasisEur: 130_000 },
    { date: "2025-09-30", label: "2025-09-30", balanceBtc: 3.8, valueEur: 326_800.0, costBasisEur: 152_000 },
    { date: "2025-12-31", label: "2025-12-31", balanceBtc: 4.1, valueEur: 369_000.0, costBasisEur: 175_000 },
    { date: "2026-02-28", label: "2026-02-28", balanceBtc: 4.25, valueEur: 331_500.0, costBasisEur: 190_000 },
    { date: "2026-04-30", label: "2026-04-30", balanceBtc: 4.38, valueEur: 312_842.77, costBasisEur: 198_502.40 },
  ],
  fiat: {
    fiatCurrency: "EUR",
    eurBalance: 312_842.77,
    eurCostBasis: 198_502.40,
    eurUnrealized: 114_340.37,
    eurRealizedYTD: 42_118.92,
  },
};
