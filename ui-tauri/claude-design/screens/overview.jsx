// Overview screen — empty state + populated dashboard
// Matches reference density: Balance Over Time, Connections, Filters, Fiat, Transactions preview, Balances

function EmptyOverview({ t, onAdd }) {
  return (
    <div style={{
      flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
      padding: 40,
    }}>
      <div style={{
        textAlign: 'center', maxWidth: 520,
        display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 20,
      }}>
        {/* decorative grid of empty tiles */}
        <div style={{
          display: 'grid', gridTemplateColumns: 'repeat(4, 40px)', gap: 6,
          marginBottom: 12, opacity: 0.35,
        }}>
          {Array.from({ length: 12 }).map((_, i) => (
            <div key={i} style={{
              width: 40, height: 28,
              border: '1px dashed var(--line-2)',
            }} />
          ))}
        </div>
        <h2 style={{
          fontFamily: 'var(--sans)', fontSize: 36, fontWeight: 600,
          margin: 0, lineHeight: 1.1, letterSpacing: '-0.01em', color: 'var(--ink)',
        }}>{t.empty.title}</h2>
        <p style={{
          margin: 0, fontFamily: 'var(--sans)', fontSize: 14, lineHeight: 1.55,
          color: 'var(--ink-2)',
        }}>{t.empty.body}</p>
        <Button size="lg" onClick={onAdd}
          icon={<svg width="12" height="12" viewBox="0 0 12 12"><path d="M6 1 V11 M1 6 H11" stroke="currentColor" strokeWidth="1.5"/></svg>}
        >{t.empty.cta}</Button>
      </div>
    </div>
  );
}

// Balance-over-time chart (SVG line + area fill)
// range: 'd' | 'w' | 'm' | 'ytd' | '1y' | '5y' | 'all'
function BalanceChart({ series, w = 520, h: hProp, ccy = 'btc', priceEur = 60000, range = 'ytd' }) {
  const h = hProp || 160;
  // Derive a series for the chosen range. We synthesize with deterministic jitter off the base series
  // so different ranges look plausibly distinct without needing more data.
  const base = series;
  const cfg = {
    d:   { n: 24,  labels: ['00','04','08','12','16','20','24'], start: 0.98, jit: 0.004 },
    w:   { n: 7,   labels: ['M','T','W','T','F','S','S'],        start: 0.93, jit: 0.015 },
    m:   { n: 30,  labels: ['1','5','10','15','20','25','30'],   start: 0.80, jit: 0.02 },
    ytd: { n: 12,  labels: ['J','F','M','A','M','J','J','A','S','O','N','D'], start: 0.22, jit: 0 },
    '1y':{ n: 12,  labels: ['M','A','M','J','J','A','S','O','N','D','J','F'], start: 0.18, jit: 0 },
    '5y':{ n: 20,  labels: ['2021','2022','2023','2024','2025'], start: 0.05, jit: 0.03 },
    all: { n: 24,  labels: ['2019','2020','2021','2022','2023','2024','2025'], start: 0.0, jit: 0.03 },
  }[range] || { n: base.length, labels: ['M','A','M','J','J','A','S','O','N','D','J','F'], start: 0.18, jit: 0 };
  const end = base[base.length - 1];
  const startVal = end * cfg.start;
  const n = cfg.n;
  // deterministic pseudo-random
  const rand = (i) => {
    const x = Math.sin((i + 1) * 9301 + (range.charCodeAt(0) || 0) * 97) * 43758.5453;
    return x - Math.floor(x);
  };
  const arr = Array.from({ length: n }, (_, i) => {
    const t = i / (n - 1);
    const linear = startVal + (end - startVal) * t;
    const jitter = (rand(i) - 0.5) * cfg.jit * end;
    return Math.max(0, linear + jitter);
  });
  // convert to EUR if needed
  const s = ccy === 'eur' ? arr.map(v => v * priceEur) : arr;

  const pad = { t: 14, r: 14, b: 22, l: ccy === 'eur' ? 48 : 36 };
  const min = 0;
  const max = Math.max(...s) * 1.15;
  const stepX = (w - pad.l - pad.r) / (s.length - 1);
  const y = v => pad.t + (1 - (v - min) / (max - min)) * (h - pad.t - pad.b);
  const pts = s.map((v, i) => [pad.l + i * stepX, y(v)]);
  const linePath = pts.map((p, i) => (i === 0 ? 'M' : 'L') + p[0].toFixed(1) + ',' + p[1].toFixed(1)).join(' ');
  const areaPath = linePath + ` L ${pts[pts.length-1][0].toFixed(1)},${(h - pad.b).toFixed(1)} L ${pts[0][0].toFixed(1)},${(h - pad.b).toFixed(1)} Z`;
  const yTicks = [0, 0.25, 0.5, 0.75, 1];
  // choose which points to show dots on (avoid crowding for 24/30/20)
  const showDotEvery = n <= 12 ? 1 : Math.ceil(n / 12);
  const showLabelEvery = Math.ceil(n / cfg.labels.length);
  const fmtY = (v) => ccy === 'eur'
    ? '€' + Math.round(v).toLocaleString('de-AT')
    : v.toFixed(1);
  return (
    <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" style={{ display: 'block' }}>
      {/* y grid */}
      {yTicks.map((tk, i) => {
        const yy = pad.t + tk * (h - pad.t - pad.b);
        return (
          <g key={i}>
            <line x1={pad.l} x2={w - pad.r} y1={yy} y2={yy} stroke="var(--line)" strokeDasharray={i === yTicks.length-1 ? '' : '2 3'} />
            <text x={pad.l - 6} y={yy + 3} textAnchor="end" fontFamily="var(--mono)" fontSize="9" fill="var(--ink-3)">
              {fmtY((1 - tk) * max)}
            </text>
          </g>
        );
      })}
      {/* area */}
      <path d={areaPath} fill="var(--accent)" fillOpacity="0.08"/>
      <path d={linePath} stroke="var(--accent)" strokeWidth="1.5" fill="none"/>
      {/* points */}
      {pts.map((p, i) => (i % showDotEvery === 0 || i === pts.length - 1) && (
        <circle key={i} cx={p[0]} cy={p[1]} r="2" fill="var(--paper-2)" stroke="var(--accent)" strokeWidth="1"/>
      ))}
      {/* x labels — distribute evenly across cfg.labels */}
      {cfg.labels.map((lbl, i) => {
        const frac = cfg.labels.length === 1 ? 0.5 : i / (cfg.labels.length - 1);
        const x = pad.l + frac * (w - pad.l - pad.r);
        return (
          <text key={i} x={x} y={h - 6} textAnchor="middle" fontFamily="var(--mono)" fontSize="9" fill="var(--ink-3)">
            {lbl}
          </text>
        );
      })}
      <text x={pad.l - 28} y={pad.t + 4} fontFamily="var(--mono)" fontSize="9" fill="var(--ink-3)">
        {ccy === 'eur' ? '€' : '₿'}
      </text>
    </svg>
  );
}

function PopulatedOverview({ t, lang, onSelectConnection, onAddConnection, onOpenTx, hideSensitive, ccy = 'btc' }) {
  const [filters, setFilters] = React.useState({ range: 'ytd', account: 'all', tag: 'all' });
  const [chartFull, setChartFull] = React.useState(false);
  const [chartRange, setChartRange] = React.useState('ytd'); // d|w|m|ytd|1y|5y|all
  const totalBtc = MOCK.connections.reduce((s, c) => s + c.balance, 0);
  const totalEur = totalBtc * MOCK.priceEur;
  const blur = v => <span className={hideSensitive ? 'sensitive' : ''}>{v}</span>;

  return (
    <div style={{
      flex: 1, overflow: 'auto', background: 'var(--paper)',
      padding: 12,
    }}>
      {/* Top row (Option D): Split chart card (KPI gutter + chart) | Connections */}
      <div style={{ display: 'grid', gridTemplateColumns: '2.4fr 1fr', gap: 10, marginBottom: 10, alignItems: 'stretch' }}>
        <Card pad={false}>
          {/* Unified header */}
          <div style={{
            height: 36, padding: '0 14px', flexShrink: 0,
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            borderBottom: '1px solid var(--line)',
          }}>
            <span style={{ fontFamily: 'var(--sans)', fontSize: 13, fontWeight: 600, color: 'var(--ink)', letterSpacing: '0.005em' }}>
              Balance &amp; performance
            </span>
            <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
              <div style={{ display: 'flex', gap: 2 }}>
                {[['d','D'],['w','W'],['m','M'],['ytd','YTD'],['1y','1Y'],['5y','5Y'],['all','ALL']].map(([k, lbl]) => (
                  <button key={k} onClick={() => setChartRange(k)} style={{
                    background: chartRange === k ? 'var(--ink)' : 'transparent',
                    color: chartRange === k ? 'var(--paper)' : 'var(--ink-2)',
                    border: chartRange === k ? '1px solid var(--ink)' : '1px solid var(--line)',
                    padding: '3px 7px',
                    fontFamily: 'var(--mono)', fontSize: 10,
                    letterSpacing: '0.08em', textTransform: 'uppercase',
                    cursor: 'pointer', fontWeight: 600,
                  }}>{lbl}</button>
                ))}
              </div>
              <button onClick={() => setChartFull(true)} title="Expand chart" style={{
                background: 'transparent', border: '1px solid var(--line)',
                width: 20, height: 20, padding: 0, cursor: 'pointer',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                  <path d="M1 4 V1 H4 M6 1 H9 V4 M9 6 V9 H6 M4 9 H1 V6" stroke="var(--ink-2)" strokeWidth="1" strokeLinecap="square"/>
                </svg>
              </button>
            </div>
          </div>

          {/* Split body: KPI gutter + chart */}
          <div style={{ display: 'grid', gridTemplateColumns: '240px 1fr', flex: 1, minHeight: 0 }}>
            {/* KPI gutter */}
            <div style={{
              padding: '14px 16px',
              borderRight: '1px solid var(--line)',
              display: 'flex', flexDirection: 'column', gap: 12,
              background: 'var(--paper-2)',
            }}>
              <div>
                <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>Total</div>
                <div className={hideSensitive?'sensitive':''} style={{ fontFamily: 'var(--sans)', fontSize: 26, fontWeight: 500, letterSpacing: '-0.015em', color: 'var(--ink)', lineHeight: 1.05 }}>
                  ₿ {totalBtc.toFixed(8)}
                </div>
                <div className={hideSensitive?'sensitive':''} style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-2)', marginTop: 2 }}>
                  € {totalEur.toLocaleString('de-AT', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
                </div>
              </div>

              <div style={{ borderTop: '1px solid var(--line)', paddingTop: 10 }}>
                <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
                  {chartRange.toUpperCase()} · change
                </div>
                <RangeDelta range={chartRange} ccy={ccy} priceEur={MOCK.priceEur} hideSensitive={hideSensitive} />
              </div>

              <div style={{ borderTop: '1px solid var(--line)', paddingTop: 10, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <GutterStat label="Cost basis" value={<span className={hideSensitive?'sensitive':''}>€ {Math.round(MOCK.fiat.eurCostBasis/1000).toLocaleString('de-AT')} k</span>} />
                <GutterStat label="Market" value={<span className={hideSensitive?'sensitive':''}>€ {Math.round(MOCK.fiat.eurBalance/1000).toLocaleString('de-AT')} k</span>} />
                <GutterStat label="Unrealized" value={<span className={hideSensitive?'sensitive':''}>+ € {Math.round(MOCK.fiat.eurUnrealized/1000).toLocaleString('de-AT')} k</span>} color="#3fa66a" />
                <GutterStat label="Realized YTD" value={<span className={hideSensitive?'sensitive':''}>+ € {Math.round(MOCK.fiat.eurRealizedYTD/1000).toLocaleString('de-AT')} k</span>} color="#3fa66a" />
              </div>

              <div style={{
                marginTop: 'auto', borderTop: '1px solid var(--line)', paddingTop: 10,
                display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8,
              }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>BTC / EUR · spot</div>
                  <div style={{ fontFamily: 'var(--sans)', fontSize: 14, fontWeight: 500, color: 'var(--ink)', letterSpacing: '-0.005em', lineHeight: 1.1 }}>
                    € {MOCK.priceEur.toLocaleString('de-AT', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
                  </div>
                </div>
                <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: '#3fa66a', whiteSpace: 'nowrap' }}>+ 1.42 % · 24h</span>
              </div>
            </div>

            {/* Chart side */}
            <div style={{ padding: 14, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
              <div style={{ flex: 1, minHeight: 0 }}>
                <BalanceChart series={MOCK.balanceSeries} ccy={ccy} priceEur={MOCK.priceEur} range={chartRange} h={240} />
              </div>
            </div>
          </div>
        </Card>
        {chartFull && <ChartFullscreen onClose={() => setChartFull(false)} hideSensitive={hideSensitive} totalBtc={totalBtc} />}

        <ConnectionsCard
          connections={MOCK.connections}
          totalBtc={totalBtc}
          hideSensitive={hideSensitive}
          onAddConnection={onAddConnection}
          onSelectConnection={onSelectConnection}
        />
      </div>

      {/* Middle row: Transactions preview | Balances */}
      <div style={{ display: 'grid', gridTemplateColumns: '1.6fr 1.5fr', gap: 10, marginBottom: 10, alignItems: 'stretch' }}>
        <Card title="Transactions" action={
          <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
            <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{MOCK.txs.length} entries</span>
            <button onClick={() => onOpenTx()} style={{
              background: 'transparent', border: 'none', cursor: 'pointer',
              fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink)',
              letterSpacing: '0.1em', textTransform: 'uppercase',
            }}>open all →</button>
          </div>
        } pad={false}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: 'var(--mono)', fontSize: 11 }}>
            <thead>
              <tr style={{ borderBottom: '1px solid var(--line)' }}>
                <th style={thStyle}>Date</th>
                <th style={thStyle}>Type</th>
                <th style={thStyle}>Counterparty</th>
                <th style={{...thStyle, textAlign:'right'}}>sats</th>
                <th style={{...thStyle, textAlign:'right'}}>€</th>
              </tr>
            </thead>
            <tbody>
              {MOCK.txs.slice(0, 6).map(tx => (
                <tr key={tx.id} style={{ borderBottom: '1px solid var(--line)' }}>
                  <td style={tdStyle}>{tx.date.slice(5, 10)}</td>
                  <td style={tdStyle}>
                    <span style={{
                      fontSize: 9, letterSpacing: '0.1em', textTransform: 'uppercase',
                      color: (tx.type === 'Income' ? '#3fa66a' : tx.type === 'Expense' ? 'var(--accent)' : tx.type === 'Swap' ? '#8b6f3c' : tx.type === 'Mint' ? '#3f7aa6' : tx.type === 'Melt' ? '#a66a3f' : 'var(--ink-2)'),
                    }}>{tx.type}</span>
                  </td>
                  <td style={{...tdStyle, fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)'}}>{tx.counter}</td>
                  <td style={{...tdStyle, textAlign:'right', color: tx.amountSat>0?'#3fa66a':'var(--ink)'}} className={hideSensitive?'sensitive':''}>
                    {(tx.amountSat > 0 ? '+' : '') + tx.amountSat.toLocaleString('en-US')}
                  </td>
                  <td style={{...tdStyle, textAlign:'right'}} className={hideSensitive?'sensitive':''}>
                    {(tx.eur > 0 ? '+ €' : '− €') + Math.abs(tx.eur).toFixed(2)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>

        <Card title="Balances">
          <BalanceRows hideSensitive={hideSensitive} />
        </Card>
      </div>

      {/* Bottom row: Exports */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 10 }}>
        <ReportTile
          title="Capital gains"
          sub="FIFO · EUR · jurisdiction preset"
          detail="YTD realized: + € 42,118.92"
          icon="↗"
          onClick={()=>{}}
        />
        <ReportTile
          title="Journal entries"
          sub="Debit / credit · double-entry"
          detail={`${MOCK.txs.length * 2} entries · YTD`}
          icon="≡"
          onClick={()=>{}}
        />
        <ReportTile
          title="Balance sheet"
          sub="Assets · Liabilities · Equity"
          detail="As of 2026-04-18"
          icon="▭"
          onClick={()=>{}}
        />
      </div>
    </div>
  );
}

const thStyle = {
  textAlign: 'left',
  padding: '8px 14px',
  fontFamily: 'var(--sans)', fontSize: 9, fontWeight: 600,
  letterSpacing: '0.12em', textTransform: 'uppercase',
  color: 'var(--ink-3)',
};
const tdStyle = {
  padding: '9px 14px',
  color: 'var(--ink-2)',
};

// ——— Connections card ——————————————————————————————————————————
// Designed for glanceability on the overview. The detail screen owns the deep
// info (addresses, derivations, IDs, backend); the overview row should answer
// three questions without a click: (1) what kind of money is this? (2) is it
// in sync? (3) how much of my stack lives here?

function ProtocolChip({ kind }) {
  const label = ({
    'xpub':       'ON-CHAIN',
    'descriptor': 'ON-CHAIN',
    'core-ln':    'LIGHTNING',
    'lnd':        'LIGHTNING',
    'nwc':        'NWC',
    'cashu':      'ECASH',
    'btcpay':     'MERCHANT',
    'kraken':     'EXCHANGE',
    'bitstamp':   'EXCHANGE',
    'coinbase':   'EXCHANGE',
    'bitpanda':   'EXCHANGE',
    'river':      'EXCHANGE',
    'strike':     'EXCHANGE',
    'csv':        'FILE',
  })[kind] || kind.toUpperCase();
  return (
    <span style={{
      fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.14em',
      color: 'var(--ink-3)', fontWeight: 600,
    }}>{label}</span>
  );
}

function SyncBadge({ status }) {
  // Just the state. The timestamp lives on the detail screen where you need precision.
  const cfg = ({
    synced:  { label: 'in sync',  color: 'var(--ink-3)', dot: null,      pulse: false },
    syncing: { label: 'syncing',  color: 'var(--accent)', dot: 'var(--accent)', pulse: true  },
    idle:    { label: 'idle',     color: 'var(--ink-2)',  dot: '#c9a43a',        pulse: false },
    error:   { label: 'error',    color: 'var(--accent)', dot: 'var(--accent)',  pulse: false },
  })[status] || { label: status, color: 'var(--ink-3)', dot: null, pulse: false };
  return (
    <span style={{
      display: 'inline-flex', alignItems: 'center', gap: 5,
      fontFamily: 'var(--mono)', fontSize: 9, color: cfg.color,
      letterSpacing: '0.1em', textTransform: 'uppercase',
      fontWeight: status === 'synced' ? 400 : 600,
    }}>
      {cfg.dot && (
        <span style={{
          width: 6, height: 6, borderRadius: '50%', background: cfg.dot,
          animation: cfg.pulse ? 'kb-fade 0.9s ease-in-out infinite alternate' : 'none',
        }} />
      )}
      {cfg.label}
    </span>
  );
}

// Connections card v2 — share-of-total composition with row-level include/exclude.
// Design moves:
//   • Percentage is the hero — large, right-aligned, tabular.
//   • Drop the redundant "SAT … sat" tail per row; balance is a thin secondary line.
//   • Each row has a checkbox to exclude it from the totals — the chart up top
//     and every other row's % recompute live.
//   • A small dot encodes sync status (color = state); the verbose "in sync /
//     syncing" badge lived twice on the card and is gone from the row.
//   • Header shows "n of N included" with included sat total; sync-all stays.

// Tiny status dot (replaces SyncBadge inside rows).
function SyncDot({ status }) {
  const cfg = ({
    synced:  { color: '#3fa66a', pulse: false, title: 'In sync' },
    syncing: { color: '#c9a43a', pulse: true,  title: 'Syncing' },
    idle:    { color: 'var(--ink-3)', pulse: false, title: 'Idle' },
    error:   { color: 'var(--accent)', pulse: false, title: 'Needs attention' },
  })[status] || { color: 'var(--ink-3)', pulse: false, title: status };
  return (
    <span title={cfg.title} style={{
      width: 6, height: 6, borderRadius: '50%', background: cfg.color, flexShrink: 0,
      animation: cfg.pulse ? 'kb-fade 0.9s ease-in-out infinite alternate' : 'none',
    }} />
  );
}

function ConnectionsCard({ connections, totalBtc, hideSensitive, onAddConnection, onSelectConnection }) {
  const [spin, setSpin] = React.useState(false);
  // Excluded connections are dropped from the total used for percentages.
  // Their balances still exist; they're just hidden from the composition view.
  const [excluded, setExcluded] = React.useState(() => new Set());

  const includedConns = connections.filter(c => !excluded.has(c.id));
  const includedBtc = includedConns.reduce((s, c) => s + c.balance, 0);
  const includedSats = Math.round(includedBtc * 1e8);

  const syncingN = connections.filter(c => c.status === 'syncing' && !excluded.has(c.id)).length;
  const errorN   = connections.filter(c => c.status === 'error'   && !excluded.has(c.id)).length;

  // Worst-severity dot for the header (status of *included* connections).
  const headDot = errorN   ? { color: 'var(--accent)', pulse: false }
                : syncingN ? { color: '#c9a43a',       pulse: true  }
                :            { color: '#3fa66a',       pulse: false };

  const onSyncAll = (e) => { e.stopPropagation(); setSpin(true); setTimeout(() => setSpin(false), 900); };

  const toggleExclude = (e, id) => {
    e.stopPropagation();
    setExcluded(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const includedCount = connections.length - excluded.size;
  const headerAction = (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
      <span style={{
        display: 'inline-flex', alignItems: 'center', gap: 6,
        fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)',
        letterSpacing: '0.04em', fontVariantNumeric: 'tabular-nums',
      }}>
        <span style={{
          width: 6, height: 6, borderRadius: '50%', background: headDot.color,
          animation: headDot.pulse ? 'kb-fade 0.9s ease-in-out infinite alternate' : 'none',
          flexShrink: 0,
        }} />
        {includedCount} of {connections.length}
      </span>
      <button onClick={onSyncAll} title="Sync all connections" style={{
        background: 'transparent', border: '1px solid var(--line)',
        width: 20, height: 20, padding: 0, cursor: 'pointer',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none"
          style={{ transition: 'transform 0.9s ease-in-out', transform: spin ? 'rotate(360deg)' : 'rotate(0deg)' }}>
          <path d="M1.5 5 A3.5 3.5 0 0 1 8.5 5 M8.5 3 L8.5 5 L6.5 5 M8.5 5 A3.5 3.5 0 0 1 1.5 5 M1.5 7 L1.5 5 L3.5 5"
            stroke="var(--ink-2)" strokeWidth="1" strokeLinecap="square" fill="none"/>
        </svg>
      </button>
    </div>
  );

  return (
    <Card title="Connections" action={headerAction} pad={false} style={{ minHeight: 0 }}>
      {/* Subhead: included subtotal + nudge to use checkboxes when something is excluded */}
      <div style={{
        padding: '6px 12px 8px', borderBottom: '1px solid var(--line)',
        display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 8,
        background: 'var(--paper-2)',
      }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
            {excluded.size === 0 ? 'Composition' : 'Composition · filtered'}
          </div>
          <div className={hideSensitive ? 'sensitive' : ''} style={{
            fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--ink)', marginTop: 1,
            fontVariantNumeric: 'tabular-nums',
          }}>
            {includedSats.toLocaleString('en-US')} <span style={{ color: 'var(--ink-3)', fontSize: 9, letterSpacing: '0.1em' }}>SAT</span>
          </div>
        </div>
        {excluded.size > 0 && (
          <button onClick={() => setExcluded(new Set())} style={{
            background: 'transparent', border: 'none', cursor: 'pointer', padding: 0,
            fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-2)',
            letterSpacing: '0.1em', textTransform: 'uppercase',
            textDecoration: 'underline', textUnderlineOffset: 2,
          }}>reset</button>
        )}
      </div>

      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
        {connections.map((c, i) => {
          const sats = Math.round(c.balance * 1e8);
          const isExcluded = excluded.has(c.id);
          const pct = (!isExcluded && includedBtc > 0) ? (c.balance / includedBtc) * 100 : 0;
          return (
            <div key={c.id} style={{
              display: 'grid',
              gridTemplateColumns: '20px 1fr auto',
              columnGap: 10, alignItems: 'center',
              padding: '9px 12px',
              borderTop: i === 0 ? 'none' : '1px solid var(--line)',
              background: isExcluded ? 'var(--paper-2)' : 'transparent',
              transition: 'background 0.12s',
              opacity: isExcluded ? 0.55 : 1,
            }}
              onMouseEnter={e => { if (!isExcluded) e.currentTarget.style.background = 'var(--paper-2)'; }}
              onMouseLeave={e => { if (!isExcluded) e.currentTarget.style.background = 'transparent'; }}
            >
              {/* col 1: include checkbox */}
              <button
                onClick={(e) => toggleExclude(e, c.id)}
                title={isExcluded ? 'Include in totals' : 'Exclude from totals'}
                style={{
                  width: 14, height: 14, padding: 0,
                  background: isExcluded ? 'transparent' : 'var(--ink)',
                  border: '1px solid var(--ink)',
                  cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}
              >
                {!isExcluded && (
                  <svg width="9" height="9" viewBox="0 0 9 9" fill="none">
                    <path d="M1.5 4.5 L3.5 6.5 L7.5 2.5" stroke="var(--paper)" strokeWidth="1.4" strokeLinecap="square"/>
                  </svg>
                )}
              </button>

              {/* col 2: label row + thin balance bar */}
              <button
                onClick={() => onSelectConnection(c.id)}
                style={{
                  display: 'flex', flexDirection: 'column', gap: 4,
                  background: 'transparent', border: 'none', padding: 0, cursor: 'pointer', textAlign: 'left',
                  minWidth: 0,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                  <SyncDot status={c.status} />
                  <span style={{
                    fontFamily: 'var(--sans)', fontSize: 13, fontWeight: 600,
                    color: 'var(--ink)', whiteSpace: 'nowrap', overflow: 'hidden',
                    textOverflow: 'ellipsis', letterSpacing: '-0.005em',
                  }}>{c.label}</span>
                  <ProtocolChip kind={c.kind} />
                </div>
                <div style={{
                  height: 2, background: 'var(--line)', position: 'relative',
                  width: '100%', maxWidth: 180,
                }}>
                  <div className={hideSensitive ? 'sensitive' : ''} style={{
                    position: 'absolute', left: 0, top: 0, bottom: 0,
                    width: isExcluded ? '0%' : `${Math.max(1.5, pct)}%`,
                    background: 'var(--ink)',
                    transition: 'width 0.25s ease-out',
                  }} />
                </div>
              </button>

              {/* col 3: pct hero + sats whisper */}
              <div style={{ textAlign: 'right', minWidth: 56 }}>
                <div className={hideSensitive ? 'sensitive' : ''} style={{
                  fontFamily: 'var(--sans)', fontSize: 16, fontWeight: 500,
                  color: isExcluded ? 'var(--ink-3)' : 'var(--ink)',
                  letterSpacing: '-0.01em', lineHeight: 1.05,
                  fontVariantNumeric: 'tabular-nums',
                }}>
                  {isExcluded ? '—' : (pct < 0.1 ? '<0.1' : pct.toFixed(pct < 10 ? 1 : 0))}
                  <span style={{ fontSize: 10, color: 'var(--ink-3)', marginLeft: 1 }}>%</span>
                </div>
                <div className={hideSensitive ? 'sensitive' : ''} style={{
                  fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)',
                  fontVariantNumeric: 'tabular-nums', marginTop: 1,
                  textDecoration: isExcluded ? 'line-through' : 'none',
                }}>
                  {sats.toLocaleString('en-US')}
                </div>
              </div>
            </div>
          );
        })}
        {/* Add-connection row */}
        <button onClick={onAddConnection} style={{
          display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8, width: '100%',
          padding: '10px 12px',
          background: 'var(--paper)',
          border: 'none', borderTop: '1px solid var(--line)',
          cursor: 'pointer',
          position: 'sticky', bottom: 0,
        }}>
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M5 1 V9 M1 5 H9" stroke="var(--ink-2)" strokeWidth="1.2"/></svg>
          <span style={{ fontFamily: 'var(--sans)', fontSize: 12, fontWeight: 500, color: 'var(--ink)' }}>
            Add connection
          </span>
        </button>
      </div>
    </Card>
  );
}

function BalanceRows({ hideSensitive }) {
  const rows = [
    { k: 'Assets',      sub: 'Resources owned',        sat: 438_007_404,   open: true, children: [
      { k: 'On-chain holdings',     sat: 432_953_372 },
      { k: 'Lightning channels',    sat:   4_821_309 },
      { k: 'Cashu (ecash)',          sat:     19_823 },
      { k: 'NWC balances',          sat:    213_500,  tiny: true },
    ]},
    { k: 'Income',      sub: 'Money earned',           sat: 75_520_000 },
    { k: 'Expenses',    sub: 'Money spent',            sat: -2_812_410 },
    { k: 'Liabilities', sub: 'Debts and obligations',  sat: 0 },
    { k: 'Equity',      sub: 'Owner contributions',    sat: 360_000_000 },
  ];
  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {rows.map((r, i) => (
        <React.Fragment key={r.k}>
          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            padding: '8px 2px',
            borderBottom: i === rows.length - 1 ? 'none' : '1px solid var(--line)',
          }}>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
              <span style={{ fontFamily: 'var(--sans)', fontSize: 14, color: 'var(--ink)' }}>{r.k}</span>
              <span style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)' }}>{r.sub}</span>
            </div>
            <span className={hideSensitive?'sensitive':''} style={{ fontFamily: 'var(--mono)', fontSize: 12, color: r.sat < 0 ? 'var(--accent)' : 'var(--ink)', letterSpacing: '-0.01em' }}>
              ₿ {formatSat(r.sat)} <span style={{color:'var(--ink-3)'}}>sat</span>
            </span>
          </div>
          {r.open && r.children && r.children.map(c => (
            <div key={c.k} style={{ display: 'flex', justifyContent: 'space-between', padding: '4px 2px 4px 18px', borderBottom: '1px dotted var(--line)' }}>
              <span style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-2)' }}>↳ {c.k}</span>
              <span className={hideSensitive?'sensitive':''} style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-2)' }}>
                ₿ {formatSat(c.sat)}
              </span>
            </div>
          ))}
        </React.Fragment>
      ))}
    </div>
  );
}

function GutterStat({ label, value, color }) {
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>{label}</div>
      <div style={{ fontFamily: 'var(--sans)', fontSize: 13, fontWeight: 500, color: color || 'var(--ink)', letterSpacing: '-0.005em', whiteSpace: 'nowrap', lineHeight: 1.15 }}>{value}</div>
    </div>
  );
}

function RangeDelta({ range, ccy, priceEur, hideSensitive }) {
  const deltas = {
    d:   { btc: -0.00184221, pct: -0.04 },
    w:   { btc: +0.02410000, pct: +0.55 },
    m:   { btc: -0.03000000, pct: -0.68 },
    ytd: { btc: +1.88000000, pct: +75.12 },
    '1y':{ btc: +2.14000000, pct: +95.70 },
    '5y':{ btc: +4.15000000, pct: +1810.00 },
    all: { btc: +4.38008004, pct: +999 },
  };
  const d = deltas[range] || deltas.ytd;
  const up = d.btc >= 0;
  const color = up ? '#3fa66a' : 'var(--accent)';
  const sign = up ? '+' : '−';
  const abs = Math.abs(d.btc);
  const absEur = abs * priceEur;
  const blur = hideSensitive ? 'sensitive' : '';
  const fmtEur = (v) => '€ ' + v.toLocaleString('de-AT', { minimumFractionDigits: 0, maximumFractionDigits: 0 });
  return (
    <>
      <div className={blur} style={{
        fontFamily: 'var(--sans)', fontSize: 22, fontWeight: 500, color,
        letterSpacing: '-0.01em', lineHeight: 1.05, marginTop: 2, whiteSpace: 'nowrap',
      }}>
        {sign} {ccy === 'btc' ? abs.toFixed(abs < 0.01 ? 8 : 4) + ' ₿' : fmtEur(absEur)}
      </div>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color, marginTop: 2 }}>
        {sign} {Math.abs(d.pct).toFixed(2)} % {ccy === 'btc' && <span className={blur} style={{ color: 'var(--ink-3)' }}>· ≈ {sign} {fmtEur(absEur)}</span>}
      </div>
    </>
  );
}

function RangeSummary({ range, ccy, totalBtc, totalEur, priceEur, hideSensitive }) {
  // Per-range delta + start value, BTC-denominated. Deterministic demo data.
  const deltas = {
    d:   { btc: -0.00184221, pct: -0.04 },
    w:   { btc: +0.02410000, pct: +0.55 },
    m:   { btc: -0.03000000, pct: -0.68 },
    ytd: { btc: +1.88000000, pct: +75.12 },
    '1y':{ btc: +2.14000000, pct: +95.70 },
    '5y':{ btc: +4.15000000, pct: +1810.00 },
    all: { btc: +4.38008004, pct: +999 },
  };
  const labels = {
    d: 'today', w: 'this week', m: 'this month', ytd: 'year-to-date',
    '1y': 'past 12 months', '5y': 'past 5 years', all: 'since inception',
  };
  const startDates = {
    d: 'Apr 23 · 00:00', w: 'Apr 17', m: 'Mar 23', ytd: 'Jan 1 · 2026',
    '1y': 'Apr 23 · 2025', '5y': 'Apr 23 · 2021', all: 'Jun 14 · 2019',
  };
  const d = deltas[range] || deltas.ytd;
  const btcStart = totalBtc - d.btc;
  const up = d.btc >= 0;
  const color = up ? '#3fa66a' : 'var(--accent)';
  const sign = up ? '+' : '−';
  const abs = Math.abs(d.btc);
  const absEur = abs * priceEur;

  const fmtBtc = (v) => v.toFixed(8);
  const fmtEur = (v) => '€ ' + v.toLocaleString('de-AT', { minimumFractionDigits: 2, maximumFractionDigits: 2 });

  const headlineValue = ccy === 'btc'
    ? `${sign} ${fmtBtc(abs)}`
    : `${sign} ${fmtEur(absEur)}`;
  const headlineUnit = ccy === 'btc' ? '₿' : '';

  const startVal = ccy === 'btc' ? fmtBtc(btcStart) : fmtEur(btcStart * priceEur);
  const endVal   = ccy === 'btc' ? fmtBtc(totalBtc) : fmtEur(totalEur);

  // High / low synthesised from delta for plausibility
  const highBtc = Math.max(totalBtc, btcStart) + Math.abs(d.btc) * 0.35;
  const lowBtc  = Math.max(0, Math.min(totalBtc, btcStart) - Math.abs(d.btc) * 0.22);
  const high = ccy === 'btc' ? fmtBtc(highBtc) : fmtEur(highBtc * priceEur);
  const low  = ccy === 'btc' ? fmtBtc(lowBtc)  : fmtEur(lowBtc * priceEur);

  const blurCls = hideSensitive ? 'sensitive' : '';

  return (
    <div style={{
      marginTop: 12, paddingTop: 12, borderTop: '1px solid var(--line)',
      display: 'grid', gridTemplateColumns: '1.3fr 1fr', gap: 18, alignItems: 'stretch',
    }}>
      {/* Headline delta for the selected range */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 }}>
        <div style={{
          display: 'flex', alignItems: 'center', gap: 6,
          fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em',
          textTransform: 'uppercase', color: 'var(--ink-3)',
        }}>
          <span style={{ color: 'var(--ink-2)', fontWeight: 600 }}>
            {range.toUpperCase()}
          </span>
          <span style={{ width: 2, height: 2, background: 'var(--ink-3)', borderRadius: '50%' }} />
          <span>change · {labels[range] || ''}</span>
        </div>
        <div className={blurCls} style={{
          fontFamily: 'var(--sans)', fontSize: 28, fontWeight: 500,
          letterSpacing: '-0.015em', color, lineHeight: 1.05,
          whiteSpace: 'nowrap',
        }}>
          {headlineValue} {headlineUnit && <span style={{ color: 'var(--ink-3)', fontSize: 20 }}>{headlineUnit}</span>}
        </div>
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)',
        }}>
          <span style={{ color }}>{sign} {Math.abs(d.pct).toFixed(2)} %</span>
          {ccy === 'btc' && (
            <>
              <span style={{ color: 'var(--line-2)' }}>│</span>
              <span className={blurCls}>≈ {sign} {fmtEur(absEur)}</span>
            </>
          )}
        </div>
      </div>

      {/* Start → end ribbon + high/low */}
      <div style={{
        display: 'flex', flexDirection: 'column', gap: 6,
        paddingLeft: 18, borderLeft: '1px solid var(--line)',
        justifyContent: 'center', minWidth: 0,
      }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr auto 1fr', alignItems: 'center', gap: 8 }}>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              {startDates[range] || 'start'}
            </div>
            <div className={blurCls} style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-2)', marginTop: 2, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {startVal}
            </div>
          </div>
          <svg width="22" height="10" viewBox="0 0 22 10" style={{ flexShrink: 0 }}>
            <path d="M0 5 L18 5 M14 1 L18 5 L14 9" stroke={color} strokeWidth="1" fill="none" strokeLinecap="square"/>
          </svg>
          <div style={{ textAlign: 'right', minWidth: 0 }}>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              now
            </div>
            <div className={blurCls} style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink)', marginTop: 2, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {endVal}
            </div>
          </div>
        </div>
        <div style={{
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          borderTop: '1px dotted var(--line)', paddingTop: 6, marginTop: 2,
          fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.08em',
          color: 'var(--ink-3)', textTransform: 'uppercase',
        }}>
          <span>Hi <span className={blurCls} style={{ color: 'var(--ink-2)', textTransform: 'none', letterSpacing: 0, fontSize: 10 }}>{high}</span></span>
          <span>Lo <span className={blurCls} style={{ color: 'var(--ink-2)', textTransform: 'none', letterSpacing: 0, fontSize: 10 }}>{low}</span></span>
        </div>
      </div>
    </div>
  );
}

function formatSat(n) {
  const sign = n < 0 ? '− ' : '';
  const abs = Math.abs(n).toString().padStart(9, '0');
  // split as BTC integer part + sats (8 digits)
  const btc = abs.slice(0, -8) || '0';
  const rest = abs.slice(-8);
  return sign + btc + '.' + rest.slice(0, 2) + ' ' + rest.slice(2, 5) + ' ' + rest.slice(5);
}

// ——— Fullscreen chart modal ——————————————————————————————————————
// Surfaced when the expand icon on the Balance & performance card is clicked.
// Reuses the existing range buttons + delta logic but goes deeper:
//   • A much larger chart with a hover scrubber: vertical guide + value
//     readout following the cursor.
//   • A "By connection" mode that paints a stacked composition area chart,
//     so the user can see where the balance is concentrated over time.
//   • Range-aware KPIs (high, low, change, % change) and a simple legend.
// Lives over the MacFrame inner content as a true fullscreen take-over.

function ChartFullscreen({ onClose, hideSensitive, totalBtc }) {
  const [range, setRange] = React.useState('ytd');
  const [stacked, setStacked] = React.useState(false);
  const [ccy, setCcy] = React.useState('btc');
  const [hover, setHover] = React.useState(null); // {i, x, y, val, date}

  React.useEffect(() => {
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Replicate the per-range synthesis used by BalanceChart so deltas line up.
  const cfg = ({
    d:   { n: 24,  start: 0.98, jit: 0.004, tickLabels: ['00:00','04:00','08:00','12:00','16:00','20:00','24:00'], scrubFmt: (i, n) => `${String(Math.round(i/(n-1)*24)).padStart(2,'0')}:00` },
    w:   { n: 7,   start: 0.93, jit: 0.015, tickLabels: ['Mon','Tue','Wed','Thu','Fri','Sat','Sun'], scrubFmt: (i) => ['Mon','Tue','Wed','Thu','Fri','Sat','Sun'][i] },
    m:   { n: 30,  start: 0.80, jit: 0.02,  tickLabels: ['1','5','10','15','20','25','30'], scrubFmt: (i) => `Apr ${i+1}` },
    ytd: { n: 12,  start: 0.22, jit: 0,     tickLabels: ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'], scrubFmt: (i) => ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'][i] + ' 2026' },
    '1y':{ n: 12,  start: 0.18, jit: 0,     tickLabels: ['M','A','M','J','J','A','S','O','N','D','J','F'], scrubFmt: (i) => ['May 25','Jun 25','Jul 25','Aug 25','Sep 25','Oct 25','Nov 25','Dec 25','Jan 26','Feb 26','Mar 26','Apr 26'][i] },
    '5y':{ n: 20,  start: 0.05, jit: 0.03,  tickLabels: ['2021','2022','2023','2024','2025','2026'], scrubFmt: (i, n) => `Q${(i % 4) + 1} · ${2021 + Math.floor(i / 4)}` },
    all: { n: 24,  start: 0.0,  jit: 0.03,  tickLabels: ['2019','2020','2021','2022','2023','2024','2025','2026'], scrubFmt: (i, n) => `${2019 + Math.floor(i / (n-1) * 7)}` },
  })[range];

  const end = totalBtc;
  const startVal = end * cfg.start;
  const n = cfg.n;
  const rand = (i, salt = 0) => {
    const x = Math.sin((i + 1) * 9301 + (range.charCodeAt(0) || 0) * 97 + salt * 13) * 43758.5453;
    return x - Math.floor(x);
  };
  const totalSeries = Array.from({ length: n }, (_, i) => {
    const t = i / (n - 1);
    const linear = startVal + (end - startVal) * t;
    const jitter = (rand(i) - 0.5) * cfg.jit * end;
    return Math.max(0, linear + jitter);
  });

  // Per-connection composition. Use current % of total as the steady-state mix
  // and back-fade newer accounts so the stacked story has movement.
  const conns = MOCK.connections;
  const mix = conns.map(c => c.balance / totalBtc);
  const stack = conns.map((c, ci) => {
    // some accounts grow late: c5 (cashu) appears late, c4 (NWC) mid-history
    const lateness = ['cashu','nwc'].includes(c.kind) ? 0.8 : (c.kind === 'core-ln' || c.kind === 'lnd') ? 0.4 : 0.0;
    return totalSeries.map((tot, i) => {
      const t = i / (n - 1);
      const ramp = lateness === 0 ? 1 : Math.max(0, (t - lateness) / Math.max(0.0001, 1 - lateness));
      // jitter the share a touch, but keep last index exactly = current mix
      const j = (rand(i, ci + 1) - 0.5) * 0.06 * mix[ci];
      const share = Math.max(0, mix[ci] * ramp + j);
      return tot * share;
    });
  });
  // normalize each timestep so stacked sums match totalSeries
  for (let i = 0; i < n; i++) {
    const sum = stack.reduce((s, layer) => s + layer[i], 0);
    if (sum > 0) {
      const k = totalSeries[i] / sum;
      for (let ci = 0; ci < stack.length; ci++) stack[ci][i] *= k;
    }
  }

  // Range KPIs
  const series = ccy === 'eur' ? totalSeries.map(v => v * MOCK.priceEur) : totalSeries;
  const max = Math.max(...series);
  const min = Math.min(...series);
  const startV = series[0];
  const endV = series[series.length - 1];
  const delta = endV - startV;
  const pct = startV > 0 ? (delta / startV) * 100 : 0;
  const up = delta >= 0;

  const fmt = (v) => ccy === 'eur'
    ? '€ ' + v.toLocaleString('de-AT', { minimumFractionDigits: 0, maximumFractionDigits: 0 })
    : '₿ ' + v.toFixed(v < 0.01 ? 8 : 4);

  const W = 1100, H = 440;
  const pad = { t: 28, r: 24, b: 36, l: 80 };
  const yMax = max * 1.08;
  const yMin = 0;
  const stepX = (W - pad.l - pad.r) / (series.length - 1);
  const yToPx = v => pad.t + (1 - (v - yMin) / (yMax - yMin)) * (H - pad.t - pad.b);
  const pts = series.map((v, i) => [pad.l + i * stepX, yToPx(v)]);
  const linePath = pts.map((p, i) => (i === 0 ? 'M' : 'L') + p[0].toFixed(1) + ',' + p[1].toFixed(1)).join(' ');
  const areaPath = linePath + ` L ${pts[pts.length-1][0].toFixed(1)},${(H - pad.b).toFixed(1)} L ${pts[0][0].toFixed(1)},${(H - pad.b).toFixed(1)} Z`;

  // Stacked layers (cumulative from bottom)
  const stackLayers = (() => {
    const cum = Array.from({ length: n }, () => 0);
    return stack.map((layer, ci) => {
      const top = layer.map((v, i) => {
        cum[i] += v;
        return cum[i];
      });
      const bot = top.map((v, i) => v - layer[i]);
      const layerSeries = ccy === 'eur' ? { top: top.map(v => v * MOCK.priceEur), bot: bot.map(v => v * MOCK.priceEur) } : { top, bot };
      const topPath = layerSeries.top.map((v, i) => (i === 0 ? 'M' : 'L') + (pad.l + i * stepX).toFixed(1) + ',' + yToPx(v).toFixed(1)).join(' ');
      const botRev = layerSeries.bot.slice().reverse();
      const botPath = botRev.map((v, j) => 'L' + (pad.l + (n - 1 - j) * stepX).toFixed(1) + ',' + yToPx(v).toFixed(1)).join(' ');
      return { ci, conn: conns[ci], d: topPath + ' ' + botPath + ' Z' };
    });
  })();

  // Connection layer colors — single accent + tonal grays so it stays calm.
  const layerFill = (ci, alpha = 0.85) => {
    const palette = [
      `rgba(34,34,34,${alpha})`,            // ink
      `rgba(74,74,74,${alpha})`,            // ink-2
      `rgba(138,138,138,${alpha})`,         // ink-3
      `rgba(212,212,212,${alpha})`,         // line-2
      `rgba(227,0,15,${Math.min(1, alpha)})`, // accent for last
    ];
    return palette[ci % palette.length];
  };

  const yTicks = [0, 0.25, 0.5, 0.75, 1];

  // Mouse handler converts client x → series index, snaps to nearest point.
  const onMouseMove = (e) => {
    const svg = e.currentTarget;
    const rect = svg.getBoundingClientRect();
    const xRatio = (e.clientX - rect.left) / rect.width;
    const xVb = xRatio * W;
    const i = Math.max(0, Math.min(n - 1, Math.round((xVb - pad.l) / stepX)));
    setHover({ i, x: pts[i][0], y: pts[i][1], val: series[i], date: cfg.scrubFmt(i, n) });
  };

  const blurCls = hideSensitive ? 'sensitive' : '';

  return (
    <div
      onClick={onClose}
      style={{
        position: 'absolute', inset: 0,
        background: 'rgba(26,22,19,0.45)',
        zIndex: 80, padding: 28,
        display: 'flex', alignItems: 'stretch', justifyContent: 'stretch',
        animation: 'kb-fade 0.15s ease-out',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          flex: 1, background: 'var(--paper)', border: '1px solid var(--ink)',
          boxShadow: '0 30px 80px -10px rgba(40,20,20,0.4)',
          display: 'flex', flexDirection: 'column', minHeight: 0,
          animation: 'kb-rise 0.2s ease-out',
        }}
      >
        {/* Header */}
        <div style={{
          height: 52, padding: '0 18px',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          borderBottom: '1px solid var(--line)',
        }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, minWidth: 0 }}>
            <span style={{ fontFamily: 'var(--sans)', fontSize: 17, fontWeight: 600, letterSpacing: '-0.005em', color: 'var(--ink)' }}>
              Balance &amp; performance
            </span>
            <span style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.16em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              fullscreen
            </span>
          </div>
          <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
            {/* range chips */}
            <div style={{ display: 'flex', gap: 2 }}>
              {[['d','D'],['w','W'],['m','M'],['ytd','YTD'],['1y','1Y'],['5y','5Y'],['all','ALL']].map(([k, lbl]) => (
                <button key={k} onClick={() => { setRange(k); setHover(null); }} style={{
                  background: range === k ? 'var(--ink)' : 'transparent',
                  color: range === k ? 'var(--paper)' : 'var(--ink-2)',
                  border: range === k ? '1px solid var(--ink)' : '1px solid var(--line)',
                  padding: '4px 9px',
                  fontFamily: 'var(--mono)', fontSize: 10,
                  letterSpacing: '0.08em', textTransform: 'uppercase',
                  cursor: 'pointer', fontWeight: 600,
                }}>{lbl}</button>
              ))}
            </div>
            {/* currency toggle */}
            <div style={{ display: 'flex', gap: 2 }}>
              {[['btc','₿'],['eur','€']].map(([k, lbl]) => (
                <button key={k} onClick={() => setCcy(k)} style={{
                  background: ccy === k ? 'var(--ink)' : 'transparent',
                  color: ccy === k ? 'var(--paper)' : 'var(--ink-2)',
                  border: ccy === k ? '1px solid var(--ink)' : '1px solid var(--line)',
                  padding: '4px 9px',
                  fontFamily: 'var(--mono)', fontSize: 11,
                  cursor: 'pointer', fontWeight: 600,
                }}>{lbl}</button>
              ))}
            </div>
            {/* mode toggle */}
            <button onClick={() => setStacked(s => !s)} style={{
              background: stacked ? 'var(--ink)' : 'transparent',
              color: stacked ? 'var(--paper)' : 'var(--ink-2)',
              border: stacked ? '1px solid var(--ink)' : '1px solid var(--line)',
              padding: '4px 10px',
              fontFamily: 'var(--mono)', fontSize: 10,
              letterSpacing: '0.08em', textTransform: 'uppercase',
              cursor: 'pointer', fontWeight: 600,
            }}>
              {stacked ? 'By connection' : 'Total'}
            </button>
            {/* close */}
            <button onClick={onClose} title="Close (Esc)" style={{
              background: 'transparent', border: '1px solid var(--line)',
              width: 28, height: 28, padding: 0, cursor: 'pointer', marginLeft: 4,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                <path d="M2 2 L10 10 M10 2 L2 10" stroke="var(--ink)" strokeWidth="1.4" strokeLinecap="square"/>
              </svg>
            </button>
          </div>
        </div>

        {/* KPI strip */}
        <div style={{
          padding: '14px 18px',
          display: 'grid',
          gridTemplateColumns: '1.4fr 1fr 1fr 1fr 1fr',
          gap: 18,
          borderBottom: '1px solid var(--line)',
          background: 'var(--paper-2)',
        }}>
          <div>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              {range.toUpperCase()} · current
            </div>
            <div className={blurCls} style={{
              fontFamily: 'var(--sans)', fontSize: 30, fontWeight: 500,
              letterSpacing: '-0.015em', color: 'var(--ink)', lineHeight: 1.05, marginTop: 2,
            }}>
              {fmt(endV)}
            </div>
          </div>
          <KpiBlock label="Change" value={
            <span className={blurCls} style={{ color: up ? '#3fa66a' : 'var(--accent)' }}>
              {up ? '+ ' : '− '}{fmt(Math.abs(delta))}
            </span>
          } sub={<span style={{ color: up ? '#3fa66a' : 'var(--accent)' }}>{up ? '+' : '−'} {Math.abs(pct).toFixed(2)} %</span>} />
          <KpiBlock label="High" value={<span className={blurCls}>{fmt(max)}</span>} sub={cfg.scrubFmt(series.indexOf(max), n)} />
          <KpiBlock label="Low" value={<span className={blurCls}>{fmt(min)}</span>} sub={cfg.scrubFmt(series.indexOf(min), n)} />
          <KpiBlock label="Spot · BTC/EUR" value={`€ ${MOCK.priceEur.toLocaleString('de-AT', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`} sub={<span style={{ color: '#3fa66a' }}>+ 1.42 % · 24h</span>} />
        </div>

        {/* Chart */}
        <div style={{ flex: 1, minHeight: 0, padding: '12px 18px 0', display: 'flex', flexDirection: 'column' }}>
          <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
            <svg
              viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none"
              style={{ width: '100%', height: '100%', display: 'block', cursor: 'crosshair' }}
              onMouseMove={onMouseMove}
              onMouseLeave={() => setHover(null)}
            >
              {/* y grid + labels */}
              {yTicks.map((tk, i) => {
                const yy = pad.t + tk * (H - pad.t - pad.b);
                const v = (1 - tk) * yMax;
                return (
                  <g key={i}>
                    <line x1={pad.l} x2={W - pad.r} y1={yy} y2={yy} stroke="var(--line)" strokeDasharray={i === yTicks.length - 1 ? '' : '2 3'} />
                    <text x={pad.l - 10} y={yy + 4} textAnchor="end" fontFamily="var(--mono)" fontSize="10" fill="var(--ink-3)">
                      {ccy === 'eur' ? '€ ' + Math.round(v).toLocaleString('de-AT') : v.toFixed(v < 0.1 ? 4 : 2)}
                    </text>
                  </g>
                );
              })}
              {/* x labels */}
              {cfg.tickLabels.map((lbl, i) => {
                const frac = cfg.tickLabels.length === 1 ? 0.5 : i / (cfg.tickLabels.length - 1);
                const x = pad.l + frac * (W - pad.l - pad.r);
                return (
                  <g key={i}>
                    <line x1={x} x2={x} y1={H - pad.b} y2={H - pad.b + 4} stroke="var(--line-2)" />
                    <text x={x} y={H - pad.b + 18} textAnchor="middle" fontFamily="var(--mono)" fontSize="10" fill="var(--ink-3)">
                      {lbl}
                    </text>
                  </g>
                );
              })}

              {/* series */}
              {!stacked && (
                <>
                  <path d={areaPath} fill="var(--accent)" fillOpacity="0.08" />
                  <path d={linePath} stroke="var(--accent)" strokeWidth="1.5" fill="none" />
                  {pts.map((p, i) => (
                    <circle key={i} cx={p[0]} cy={p[1]} r={n <= 12 ? 2.5 : 1.5} fill="var(--paper)" stroke="var(--accent)" strokeWidth="1" />
                  ))}
                </>
              )}
              {stacked && (
                <g className={blurCls}>
                  {stackLayers.map((l, idx) => (
                    <path key={l.ci} d={l.d} fill={layerFill(idx, 0.78)} stroke="var(--paper)" strokeWidth="0.6" />
                  ))}
                  {/* keep total line on top for reference */}
                  <path d={linePath} stroke="var(--ink)" strokeWidth="1" fill="none" strokeOpacity="0.25" />
                </g>
              )}

              {/* Hover crosshair */}
              {hover && (
                <g>
                  <line x1={hover.x} x2={hover.x} y1={pad.t} y2={H - pad.b} stroke="var(--ink)" strokeDasharray="2 3" strokeWidth="1" />
                  <circle cx={hover.x} cy={hover.y} r="4" fill="var(--paper)" stroke="var(--ink)" strokeWidth="1.4" />
                </g>
              )}
            </svg>

            {/* Hover readout — absolute, snaps near the cursor */}
            {hover && (
              <div style={{
                position: 'absolute',
                left: `${(hover.x / W) * 100}%`,
                top: 8,
                transform: hover.x > W * 0.7 ? 'translateX(-100%) translateX(-12px)' : 'translateX(12px)',
                background: 'var(--ink)', color: 'var(--paper)',
                padding: '6px 10px',
                fontFamily: 'var(--mono)', fontSize: 10,
                pointerEvents: 'none', whiteSpace: 'nowrap',
                letterSpacing: '0.04em',
              }}>
                <div style={{ color: 'var(--paper)', opacity: 0.7, fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase' }}>{hover.date}</div>
                <div className={blurCls} style={{ fontSize: 13, fontFamily: 'var(--sans)', letterSpacing: '-0.005em', marginTop: 1 }}>
                  {fmt(hover.val)}
                </div>
              </div>
            )}
          </div>

          {/* Legend / footnote */}
          <div style={{
            padding: '10px 0 14px',
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            borderTop: '1px solid var(--line)', marginTop: 8,
          }}>
            {stacked ? (
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 14 }}>
                {conns.map((c, ci) => (
                  <div key={c.id} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <span style={{ width: 10, height: 10, background: layerFill(ci, 0.85), display: 'inline-block' }} />
                    <span style={{ fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)' }}>{c.label}</span>
                    <span className={blurCls} style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>
                      {((c.balance / totalBtc) * 100).toFixed(1)}%
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ width: 14, height: 2, background: 'var(--accent)', display: 'inline-block' }} />
                <span style={{ fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)' }}>Total balance</span>
                <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', marginLeft: 8 }}>
                  {n} points · synthesized for demo
                </span>
              </div>
            )}
            <span style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              hover for detail · esc to close
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

function KpiBlock({ label, value, sub }) {
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>{label}</div>
      <div style={{ fontFamily: 'var(--sans)', fontSize: 18, fontWeight: 500, color: 'var(--ink)', letterSpacing: '-0.01em', marginTop: 2, lineHeight: 1.1, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {value}
      </div>
      {sub && (
        <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', marginTop: 2 }}>{sub}</div>
      )}
    </div>
  );
}

function ReportTile({ title, sub, detail, icon, onClick }) {
  return (
    <button onClick={onClick} style={{
      textAlign: 'left',
      background: 'var(--paper-2)',
      border: '1px solid var(--line)',
      padding: 16,
      cursor: 'pointer',
      display: 'flex', gap: 14, alignItems: 'flex-start',
    }}>
      <div style={{
        width: 34, height: 34, flexShrink: 0,
        border: '1px solid var(--ink)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        fontFamily: 'var(--sans)', fontSize: 18, color: 'var(--ink)',
      }}>{icon}</div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontFamily: 'var(--sans)', fontSize: 16, color: 'var(--ink)' }}>{title}</div>
        <div style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)', marginTop: 2 }}>{sub}</div>
        <div style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink)', marginTop: 10 }}>{detail}</div>
      </div>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--ink-3)' }}>↗</span>
    </button>
  );
}

window.EmptyOverview = EmptyOverview;
window.PopulatedOverview = PopulatedOverview;
