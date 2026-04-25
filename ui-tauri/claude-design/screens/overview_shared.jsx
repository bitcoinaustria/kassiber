// Shared bits for overview layout mocks (options A–D)
// Lightweight chart + KPI building blocks, decoupled from the main overview.

function MockChart({ h = 160, w = 600, ccy = 'btc', priceEur = 71420.18, range = 'ytd', color = 'var(--accent)' }) {
  const cfg = {
    d:   { n: 24,  labels: ['00','04','08','12','16','20','24'], start: 0.98, jit: 0.004 },
    w:   { n: 7,   labels: ['M','T','W','T','F','S','S'],        start: 0.93, jit: 0.015 },
    m:   { n: 30,  labels: ['1','5','10','15','20','25','30'],   start: 0.80, jit: 0.02 },
    ytd: { n: 12,  labels: ['J','F','M','A','M','J','J','A','S','O','N','D'], start: 0.22, jit: 0 },
    '1y':{ n: 12,  labels: ['M','A','M','J','J','A','S','O','N','D','J','F'], start: 0.18, jit: 0 },
    '5y':{ n: 20,  labels: ['2021','2022','2023','2024','2025'], start: 0.05, jit: 0.03 },
    all: { n: 24,  labels: ['2019','2020','2021','2022','2023','2024','2025'], start: 0.0, jit: 0.03 },
  }[range] || { n: 12, labels: ['J','F','M','A','M','J','J','A','S','O','N','D'], start: 0.22, jit: 0 };
  const end = 4.38;
  const startVal = end * cfg.start;
  const n = cfg.n;
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
  const s = ccy === 'eur' ? arr.map(v => v * priceEur) : arr;

  const pad = { t: 14, r: 14, b: 22, l: ccy === 'eur' ? 52 : 36 };
  const min = 0;
  const max = Math.max(...s) * 1.15;
  const stepX = (w - pad.l - pad.r) / (s.length - 1);
  const y = v => pad.t + (1 - (v - min) / (max - min)) * (h - pad.t - pad.b);
  const pts = s.map((v, i) => [pad.l + i * stepX, y(v)]);
  const linePath = pts.map((p, i) => (i === 0 ? 'M' : 'L') + p[0].toFixed(1) + ',' + p[1].toFixed(1)).join(' ');
  const areaPath = linePath + ` L ${pts[pts.length-1][0].toFixed(1)},${(h - pad.b).toFixed(1)} L ${pts[0][0].toFixed(1)},${(h - pad.b).toFixed(1)} Z`;
  const yTicks = [0, 0.25, 0.5, 0.75, 1];
  const showDotEvery = n <= 12 ? 1 : Math.ceil(n / 12);
  const fmtY = (v) => ccy === 'eur'
    ? '€' + Math.round(v).toLocaleString('de-AT')
    : v.toFixed(1);
  return (
    <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" style={{ display: 'block' }}>
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
      <path d={areaPath} fill={color} fillOpacity="0.08"/>
      <path d={linePath} stroke={color} strokeWidth="1.5" fill="none"/>
      {pts.map((p, i) => (i % showDotEvery === 0 || i === pts.length - 1) && (
        <circle key={i} cx={p[0]} cy={p[1]} r="2" fill="var(--paper-2)" stroke={color} strokeWidth="1"/>
      ))}
      {cfg.labels.map((lbl, i) => {
        const frac = cfg.labels.length === 1 ? 0.5 : i / (cfg.labels.length - 1);
        const x = pad.l + frac * (w - pad.l - pad.r);
        return (
          <text key={i} x={x} y={h - 6} textAnchor="middle" fontFamily="var(--mono)" fontSize="9" fill="var(--ink-3)">
            {lbl}
          </text>
        );
      })}
    </svg>
  );
}

// Small range pill row, reused
function RangeTabs({ value, onChange, compact = false }) {
  const opts = [['d','D'],['w','W'],['m','M'],['ytd','YTD'],['1y','1Y'],['5y','5Y'],['all','ALL']];
  return (
    <div style={{ display: 'flex', gap: 2 }}>
      {opts.map(([k, lbl]) => (
        <button key={k} onClick={() => onChange(k)} style={{
          background: value === k ? 'var(--ink)' : 'transparent',
          color: value === k ? 'var(--paper)' : 'var(--ink-2)',
          border: value === k ? '1px solid var(--ink)' : '1px solid var(--line)',
          padding: compact ? '3px 7px' : '4px 10px',
          fontFamily: 'var(--mono)', fontSize: 10,
          letterSpacing: '0.08em', textTransform: 'uppercase',
          cursor: 'pointer', fontWeight: 600,
        }}>{lbl}</button>
      ))}
    </div>
  );
}

function CcyToggle({ value, onChange }) {
  return (
    <div style={{
      display: 'inline-flex', border: '1px solid var(--line)',
      padding: 1, background: 'var(--paper)',
    }}>
      {['btc', 'eur'].map(k => (
        <button key={k} onClick={() => onChange(k)} style={{
          background: value === k ? 'var(--ink)' : 'transparent',
          color: value === k ? 'var(--paper)' : 'var(--ink-2)',
          border: 'none',
          padding: '3px 8px',
          fontFamily: 'var(--mono)', fontSize: 10,
          letterSpacing: '0.1em', textTransform: 'uppercase',
          cursor: 'pointer', fontWeight: 600,
        }}>{k}</button>
      ))}
    </div>
  );
}

// Compact KPI cell — label over big value over delta
function Kpi({ label, value, delta, deltaColor = '#3fa66a', width, align = 'left' }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0, width, textAlign: align }}>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>{label}</div>
      <div style={{ fontFamily: 'var(--sans)', fontSize: 20, fontWeight: 500, color: 'var(--ink)', letterSpacing: '-0.01em', lineHeight: 1.1, whiteSpace: 'nowrap' }}>{value}</div>
      {delta && <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: deltaColor }}>{delta}</div>}
    </div>
  );
}

function TinyKpi({ label, value, delta, deltaColor = '#3fa66a' }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>{label}</div>
      <div style={{ fontFamily: 'var(--sans)', fontSize: 14, fontWeight: 500, color: 'var(--ink)', letterSpacing: '-0.005em', lineHeight: 1.1, whiteSpace: 'nowrap' }}>{value}</div>
      {delta && <div style={{ fontFamily: 'var(--mono)', fontSize: 9, color: deltaColor }}>{delta}</div>}
    </div>
  );
}

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
  })[kind] || (kind || '').toUpperCase();
  return (
    <span style={{
      fontFamily: 'var(--mono)', fontSize: 8, letterSpacing: '0.14em',
      color: 'var(--ink-3)', fontWeight: 600,
    }}>{label}</span>
  );
}

function MiniConnectionsList({ dense = false, onlyTop = null }) {
  const conns = onlyTop ? MOCK.connections.slice(0, onlyTop) : MOCK.connections;
  const total = MOCK.connections.reduce((s, c) => s + c.balance, 0);
  return (
    <div>
      {conns.map((c, i) => {
        const sats = Math.round(c.balance * 1e8);
        const pct = total > 0 ? (c.balance / total) * 100 : 0;
        return (
          <div key={c.id} style={{
            display: 'grid', gridTemplateColumns: '1fr auto',
            columnGap: 10, rowGap: 2,
            padding: dense ? '6px 12px' : '9px 12px',
            borderTop: i === 0 ? 'none' : '1px solid var(--line)',
          }}>
            <div style={{ minWidth: 0, display: 'flex', alignItems: 'baseline', gap: 8 }}>
              <span style={{ fontFamily: 'var(--sans)', fontSize: dense ? 12 : 13, fontWeight: 600, color: 'var(--ink)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{c.label}</span>
              <ProtocolChip kind={c.kind} />
            </div>
            <div style={{ textAlign: 'right', fontFamily: 'var(--mono)', fontSize: dense ? 11 : 12, color: 'var(--ink)', fontVariantNumeric: 'tabular-nums' }}>
              {sats.toLocaleString('en-US')} <span style={{ color: 'var(--ink-3)', fontSize: 9 }}>SAT</span>
            </div>
            {!dense && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                <div style={{ flex: 1, height: 2, background: 'var(--line)', position: 'relative', maxWidth: 140 }}>
                  <div style={{ position: 'absolute', left: 0, top: 0, bottom: 0, width: `${Math.max(1.5, pct)}%`, background: 'var(--ink)' }} />
                </div>
                <span style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-2)', minWidth: 30, textAlign: 'right' }}>
                  {pct < 1 ? '<1' : pct.toFixed(1)}%
                </span>
              </div>
            )}
            <div style={{ textAlign: 'right', fontFamily: 'var(--mono)', fontSize: 9, color: c.status === 'synced' ? 'var(--ink-3)' : 'var(--accent)', letterSpacing: '0.04em', textTransform: c.status !== 'synced' ? 'uppercase' : 'none', fontWeight: c.status !== 'synced' ? 600 : 400 }}>
              {c.status === 'synced' ? c.last : `${c.status} · ${c.last}`}
            </div>
          </div>
        );
      })}
    </div>
  );
}

Object.assign(window, { MockChart, RangeTabs, CcyToggle, Kpi, TinyKpi, ProtocolChip, MiniConnectionsList });
