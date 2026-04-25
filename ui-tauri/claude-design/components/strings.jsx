// DE/EN strings for Kassiber
const STRINGS = {
  en: {
    locale: 'EN',
    welcome: {
      eyebrow: 'Local · Self-hosted · Bitcoin-only',
      title: 'Welcome.',
      sub: 'Kassiber keeps your books in your own hands. No cloud, no middleman, no breach-in-waiting.',
      label: 'Your name',
      placeholder: 'e.g. Alice',
      workspace: 'Workspace',
      workspacePh: 'My Books',
      taxYear: 'Tax residency',
      go: "Let's go",
      footnote: 'Sensible defaults for your jurisdiction will be applied (FIFO, EUR). Editable later.',
    },
    nav: { overview: 'Overview', transactions: 'Transactions', connections: 'Connections', reports: 'Reports', settings: 'Settings' },
    empty: {
      title: 'No connections yet.',
      body: 'Add a watch-only connection — XPub, descriptor, Lightning node, or CSV — to import transactions.',
      cta: 'Add connection',
    },
    add: {
      title: 'Add a connection',
      sub: 'Kassiber is watch-only. Keys never leave your machine.',
    },
    kind: {
      xpub: { name: 'XPub', desc: 'Single-sig on-chain watch' },
      descriptor: { name: 'Descriptor', desc: 'Multisig wallet descriptor' },
      'core-ln': { name: 'Core Lightning', desc: 'CLN node RPC' },
      lnd: { name: 'LND', desc: 'Lightning Network Daemon' },
      nwc: { name: 'NWC', desc: 'Nostr Wallet Connect' },
      cashu: { name: 'Cashu', desc: 'Ecash mint wallet' },
      btcpay: { name: 'BTCPay Server', desc: 'Merchant API · store read-key' },
      kraken: { name: 'Kraken', desc: 'Read-only API key' },
      bitstamp: { name: 'Bitstamp', desc: 'Read-only API key' },
      coinbase: { name: 'Coinbase', desc: 'Read-only API key' },
      bitpanda: { name: 'Bitpanda', desc: 'Read-only API key' },
      river: { name: 'River', desc: 'Read-only API key' },
      strike: { name: 'Strike', desc: 'Read-only API key' },
      csv: { name: 'CSV import', desc: 'One-shot, from file' },
      bip329: { name: 'BIP-329 labels', desc: 'Import labels · JSONL' },
    },
  },
  de: {
    locale: 'DE',
    welcome: {
      eyebrow: 'Lokal · Selbstgehostet · Nur Bitcoin',
      title: 'Willkommen.',
      sub: 'Kassiber hält Ihre Buchhaltung in Ihren eigenen Händen. Keine Cloud, kein Mittelsmann, kein Datenleck in Wartestellung.',
      label: 'Ihr Name',
      placeholder: 'z. B. Alice',
      workspace: 'Arbeitsbereich',
      workspacePh: 'Meine Bücher',
      taxYear: 'Steuerlicher Wohnsitz',
      go: 'Los geht\u2019s',
      footnote: 'Passende Voreinstellungen für Ihre Jurisdiktion werden übernommen (FIFO, EUR). Später editierbar.',
    },
    nav: { overview: 'Übersicht', transactions: 'Transaktionen', connections: 'Verbindungen', reports: 'Berichte', settings: 'Einstellungen' },
    empty: {
      title: 'Noch keine Verbindungen.',
      body: 'Fügen Sie eine Nur-Lese-Verbindung hinzu — XPub, Descriptor, Lightning-Node oder CSV — um Transaktionen zu importieren.',
      cta: 'Verbindung hinzufügen',
    },
    add: {
      title: 'Verbindung hinzufügen',
      sub: 'Kassiber liest nur. Schlüssel verlassen Ihr Gerät nicht.',
    },
    kind: {
      xpub: { name: 'XPub', desc: 'Single-Sig On-Chain' },
      descriptor: { name: 'Descriptor', desc: 'Multisig-Wallet-Descriptor' },
      'core-ln': { name: 'Core Lightning', desc: 'CLN-Node-RPC' },
      lnd: { name: 'LND', desc: 'Lightning Network Daemon' },
      nwc: { name: 'NWC', desc: 'Nostr Wallet Connect' },
      cashu: { name: 'Cashu', desc: 'E-Cash-Mint-Wallet' },
      btcpay: { name: 'BTCPay Server', desc: 'Merchant-API · Store-Read-Key' },
      kraken: { name: 'Kraken', desc: 'API-Key, nur lesen' },
      bitstamp: { name: 'Bitstamp', desc: 'API-Key, nur lesen' },
      coinbase: { name: 'Coinbase', desc: 'API-Key, nur lesen' },
      bitpanda: { name: 'Bitpanda', desc: 'API-Key, nur lesen' },
      river: { name: 'River', desc: 'API-Key, nur lesen' },
      strike: { name: 'Strike', desc: 'API-Key, nur lesen' },
      csv: { name: 'CSV-Import', desc: 'Einmalig, aus Datei' },
      bip329: { name: 'BIP-329 Labels', desc: 'Labels importieren · JSONL' },
    },
  },
};

// Mock data for populated states
const MOCK = {
  priceEur: 71_420.18,
  priceUsd: 76_597.49,
  connections: [
    { id: 'c1', kind: 'xpub', label: 'Cold Storage', last: '2m ago', balance: 1.24810472, status: 'synced', addresses: 142, gap: 10 },
    { id: 'c2', kind: 'descriptor', label: 'Multisig 2/3 Vault', last: '2m ago', balance: 3.08142900, status: 'synced', addresses: 86, gap: 10 },
    { id: 'c3', kind: 'core-ln', label: 'Home Node (CLN)', last: '18s ago', balance: 0.04821309, status: 'syncing', channels: 12 },
    { id: 'c4', kind: 'nwc', label: 'Alby Hub', last: '1h ago', balance: 0.00213500, status: 'idle' },
    { id: 'c5', kind: 'cashu', label: 'minibits.cash', last: '3h ago', balance: 0.00019823, status: 'synced' },
  ],
  txs: [
    { id: 'tx1', date: '2026-04-18 14:22', type: 'Income',   account: 'Cold Storage',        counter: 'Invoice · ACME GmbH',        amountSat: 2_450_000,   eur:  1749.79, rate: 71420.18, tag: 'Revenue',      conf: 41 },
    { id: 'tx2', date: '2026-04-17 09:08', type: 'Expense',  account: 'Home Node (CLN)',     counter: 'Server rental · Hetzner',    amountSat: -120_431,    eur:   -86.00, rate: 71432.10, tag: 'Hosting',      conf: 140 },
    { id: 'tx3', date: '2026-04-16 16:51', type: 'Transfer', account: 'Cold Storage → Vault',counter: 'Internal transfer',           amountSat: -50_000_000, eur:-35710.09, rate: 71420.18, tag: 'Transfer',     conf: 220, internal: true },
    { id: 'tx4', date: '2026-04-15 11:14', type: 'Income',   account: 'NWC · Alby',          counter: 'Client payment · LN',        amountSat:    92_808,   eur:    66.27, rate: 71398.42, tag: 'Revenue',      conf: 1 },
    { id: 'tx5', date: '2026-04-14 22:02', type: 'Expense',  account: 'Multisig Vault',     counter: 'Equipment · BitcoinStore',    amountSat:   -890_210,  eur:  -635.71, rate: 71412.00, tag: 'Capex',        conf: 420 },
    { id: 'tx6', date: '2026-04-12 08:30', type: 'Income',   account: 'Cold Storage',        counter: 'Sale · Consulting',          amountSat: 3_800_000,   eur:  2713.97, rate: 71420.18, tag: 'Revenue',      conf: 612 },
    { id: 'tx7', date: '2026-04-11 19:45', type: 'Expense',  account: 'Cashu · minibits',    counter: 'Coffee',                      amountSat:     -8_400,  eur:    -6.00, rate: 71428.57, tag: 'Meals',        conf: 1 },
    { id: 'tx8', date: '2026-04-09 10:00', type: 'Fee',      account: 'Home Node (CLN)',     counter: 'Channel open',                amountSat:    -18_210,  eur:   -13.01, rate: 71445.91, tag: 'Bank fees',    conf: 380 },
    { id: 'tx9', date: '2026-04-07 13:12', type: 'Income',   account: 'Multisig Vault',     counter: 'Invoice · Globex AG',         amountSat: 1_210_000,   eur:   864.18, rate: 71420.00, tag: 'Revenue',      conf: 820 },
    { id: 'tx10', date: '2026-04-06 15:30', type: 'Swap',       account: 'NWC · Alby → Cashu · minibits', counter: 'LN → ecash swap',      amountSat:    500_000,  eur:   357.10, rate: 71420.00, tag: 'Swap',         conf: 1 },
    { id: 'tx11', date: '2026-04-05 11:08', type: 'Swap',       account: 'Multisig Vault → Home Node (CLN)', counter: 'Submarine swap · on-chain → LN', amountSat: 2_000_000, eur: 1428.40, rate: 71420.00, tag: 'Swap',         conf: 12 },
    { id: 'tx12', date: '2026-04-03 09:22', type: 'Consolidation', account: 'Cold Storage',    counter: '12 UTXOs → 1',               amountSat:    -42_180,  eur:   -30.13, rate: 71432.00, tag: 'Consolidation',conf: 210 },
    { id: 'tx13', date: '2026-03-30 18:44', type: 'Consolidation', account: 'Multisig Vault',  counter: '8 UTXOs → 1',                amountSat:    -58_900,  eur:   -42.08, rate: 71432.00, tag: 'Consolidation',conf: 980 },
    { id: 'tx14', date: '2026-03-28 12:10', type: 'Rebalance',  account: 'Home Node (CLN)',   counter: 'Circular rebalance · LN',     amountSat:     -2_140,  eur:    -1.53, rate: 71432.00, tag: 'Rebalance',    conf: 1 },
    { id: 'tx15', date: '2026-03-25 20:15', type: 'Mint',       account: 'Cashu · minibits',  counter: 'Mint ecash from LN',          amountSat:    100_000,  eur:    71.42, rate: 71420.00, tag: 'Mint',         conf: 1 },
    { id: 'tx16', date: '2026-03-24 17:40', type: 'Melt',       account: 'Cashu · minibits',  counter: 'Melt ecash to LN',            amountSat:    -50_000,  eur:   -35.71, rate: 71420.00, tag: 'Melt',         conf: 1 },
  ],
  balanceSeries: [
    // monthly-ish, sats→btc total across 12 points
    0.8, 1.1, 1.6, 1.55, 2.2, 2.4, 2.8, 3.1, 3.6, 4.0, 4.3, 4.38,
  ],
  fiat: {
    eurBalance: 312_842.77,
    eurCostBasis: 198_502.40,
    eurUnrealized: 114_340.37,
    eurRealizedYTD: 42_118.92,
  },
  workspaces: [
    {
      id: 'w1', name: 'My Books', kind: 'Personal', currency: 'EUR', jurisdiction: 'Austria',
      created: '2024-03-12', profiles: [
        { id: 'p1', name: 'Alice', role: 'Owner', taxPolicy: 'Private · 1 year holding spec.', accounts: 4, wallets: 5, lastOpened: 'Just now', active: true },
        { id: 'p2', name: 'Alice · Self-employed', role: 'Owner', taxPolicy: 'Self-employed · FIFO · full income tax', accounts: 3, wallets: 2, lastOpened: '3 days ago' },
      ],
    },
    {
      id: 'w2', name: 'Hyperion GmbH', kind: 'Business', currency: 'EUR', jurisdiction: 'Germany',
      created: '2024-09-01', profiles: [
        { id: 'p3', name: 'Hyperion GmbH · Operating', role: 'Treasurer', taxPolicy: 'Business · FIFO · corporate income tax', accounts: 6, wallets: 8, lastOpened: 'Yesterday' },
        { id: 'p4', name: 'Hyperion GmbH · Treasury', role: 'Treasurer', taxPolicy: 'Business · FIFO · long-term hold', accounts: 2, wallets: 3, lastOpened: '1 week ago' },
      ],
    },
    {
      id: 'w3', name: 'Family', kind: 'Household', currency: 'CHF', jurisdiction: 'Switzerland',
      created: '2025-02-18', profiles: [
        { id: 'p5', name: 'Household', role: 'Owner', taxPolicy: 'Private · shared · long-term hold', accounts: 2, wallets: 3, lastOpened: '2 weeks ago' },
      ],
    },
  ],
};

window.STRINGS = STRINGS;
window.MOCK = MOCK;
