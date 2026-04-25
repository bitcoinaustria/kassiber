// Settings modal
const DEFAULT_BACKENDS = [
  { id: 'b1', name: 'mempool.space',     url: 'https://mempool.space/api',          net: 'BTC',    health: '#893,014 · 2m',  on: true,  auth: 'none'  },
  { id: 'b2', name: 'local electrs',     url: 'tcp://127.0.0.1:50001',              net: 'BTC',    health: '—',               on: false, auth: 'none'  },
  { id: 'b3', name: 'Blockstream Liquid',url: 'https://blockstream.info/liquid/api',net: 'LIQUID', health: '—',               on: false, auth: 'none'  },
  { id: 'b4', name: 'CoinGecko',         url: 'https://api.coingecko.com/api/v3',   net: 'FX',     health: '€71,420 · 14s',   on: true,  auth: 'none'  },
];

function SettingsModal({ open, onClose, hideSensitive, setHideSensitive, lockSettings, setLockSettings, onLockNow, onImportLabels, onExportLabels, onImportCsv }) {
  const ls = lockSettings || { autoLockEnabled: true, autoLockMinutes: 5, requirePassphrase: true, lockOnClose: true };
  const update = (k, v) => setLockSettings && setLockSettings({ ...ls, [k]: v });
  const [backends, setBackends] = React.useState(DEFAULT_BACKENDS);
  const [addOpen, setAddOpen] = React.useState(false);
  return (
    <Modal open={open} onClose={onClose} title="Settings" width={580}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
        {/* Privacy */}
        <Section title="Privacy">
          <Row label="Hide sensitive data"
            sub="Blur balances, addresses, and amounts throughout the UI."
            control={<Toggle on={hideSensitive} onChange={setHideSensitive} />} />
          <Row label="Clear clipboard after 30s"
            sub="Auto-clear copied addresses and keys."
            control={<Toggle on={true} />} />
        </Section>

        {/* App lock */}
        <Section title="App lock">
          <Row label="Auto-lock when idle"
            sub="Require passphrase to re-enter after a period of inactivity."
            control={<Toggle on={ls.autoLockEnabled} onChange={(v) => update('autoLockEnabled', v)} />} />
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 0 4px 2px', opacity: ls.autoLockEnabled ? 1 : 0.4 }}>
            <span style={{ fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink-2)' }}>Idle timeout</span>
            <div style={{ display: 'flex', gap: 4, marginLeft: 'auto' }}>
              {[1, 5, 15, 30, 60].map(m => (
                <Pill key={m} active={ls.autoLockMinutes === m}
                  onClick={ls.autoLockEnabled ? () => update('autoLockMinutes', m) : undefined}
                  color={ls.autoLockMinutes === m ? 'ink' : 'muted'}>{m}m</Pill>
              ))}
            </div>
          </div>
          <Row label="Require passphrase on launch"
            sub="Prompt for your workspace passphrase every time Kassiber opens."
            control={<Toggle on={ls.requirePassphrase} onChange={(v) => update('requirePassphrase', v)} />} />
          <Row label="Lock on window close"
            sub="Clear in-memory decrypted state when the app window is closed."
            control={<Toggle on={ls.lockOnClose} onChange={(v) => update('lockOnClose', v)} />} />
          <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
            <Button variant="secondary" size="sm" onClick={onLockNow}
              icon={<svg width="11" height="11" viewBox="0 0 14 14"><rect x="3" y="6.2" width="8" height="6.3" stroke="currentColor" strokeWidth="1.2" fill="none"/><path d="M4.8 6.2 V4.3 Q4.8 2 7 2 Q9.2 2 9.2 4.3 V6.2" stroke="currentColor" strokeWidth="1.2" fill="none"/></svg>}
            >Lock now</Button>
            <Button variant="ghost" size="sm">Change passphrase…</Button>
          </div>
        </Section>

        {/* Data */}
        <Section title="Data">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
            <Button variant="secondary" size="md" style={{ justifyContent: 'flex-start' }}
              icon={<span style={{fontFamily:'var(--mono)',fontSize:14}}>⤓</span>}>Backup</Button>
            <Button variant="secondary" size="md" style={{ justifyContent: 'flex-start' }}
              icon={<span style={{fontFamily:'var(--mono)',fontSize:14}}>⤒</span>}>Restore</Button>
            <Button variant="secondary" size="md" style={{ justifyContent: 'flex-start' }}
              icon={<span style={{fontFamily:'var(--mono)',fontSize:14}}>⋯</span>}>Logs</Button>
          </div>

          <div style={{
            marginTop: 10, padding: '10px 12px',
            border: '1px solid var(--line)', background: 'var(--paper)',
          }}>
            <div style={{
              fontFamily: 'var(--mono)', fontSize: 9, fontWeight: 600,
              letterSpacing: '0.14em', textTransform: 'uppercase',
              color: 'var(--ink-3)', marginBottom: 8,
            }}>Labels & imports · workspace-wide</div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
              <Button variant="secondary" size="sm" onClick={onImportLabels}
                style={{ justifyContent: 'flex-start' }}>↓ Import BIP-329</Button>
              <Button variant="secondary" size="sm" onClick={onExportLabels}
                style={{ justifyContent: 'flex-start' }}>↑ Export BIP-329</Button>
              <Button variant="secondary" size="sm" onClick={onImportCsv}
                style={{ justifyContent: 'flex-start' }}>↓ Import CSV</Button>
            </div>
          </div>

          <div style={{ marginTop: 8, fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', lineHeight: 1.6 }}>
            DB  ~/.kassiber/kassiber.db · 2.4 MB<br/>
            Last backup  2026-04-17 23:02 · backup_2026-04-17.tar.zst
          </div>
        </Section>

        {/* Sync */}
        <Section title="Sync backends">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {backends.map((b) => (
              <div key={b.id} style={{
                display: 'grid',
                gridTemplateColumns: '10px 64px 1fr auto auto',
                alignItems: 'center', gap: 12,
                padding: '8px 10px', border: '1px solid var(--line)',
              }}>
                <span style={{ width: 6, height: 6, borderRadius: '50%', background: b.on ? '#3fa66a' : 'var(--ink-3)' }} />
                <NetworkBadge net={b.net} />
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--ink)' }}>{b.name}</div>
                  <div style={{
                    fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)',
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>{b.url}</div>
                </div>
                <span style={{
                  fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.04em',
                  color: b.on ? 'var(--ink-2)' : 'var(--ink-3)',
                  minWidth: 120, textAlign: 'right',
                }}>{b.health}</span>
                <span style={{
                  fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.12em',
                  textTransform: 'uppercase', color: b.on ? '#3fa66a' : 'var(--ink-3)',
                  minWidth: 44, textAlign: 'right',
                }}>{b.on ? 'active' : 'idle'}</span>
              </div>
            ))}

            {/* Add backend row */}
            <button onClick={() => setAddOpen(true)} style={{
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8,
              padding: '9px 10px',
              border: '1px dashed var(--ink-3)',
              background: 'transparent',
              cursor: 'pointer',
              fontFamily: 'var(--mono)', fontSize: 11, letterSpacing: '0.1em',
              textTransform: 'uppercase', color: 'var(--ink-2)',
            }}
              onMouseEnter={e => { e.currentTarget.style.borderColor = 'var(--ink)'; e.currentTarget.style.background = 'var(--paper-2)'; }}
              onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--ink-3)'; e.currentTarget.style.background = 'transparent'; }}
            >
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                <path d="M5 1 V9 M1 5 H9" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
              </svg>
              Add backend
            </button>
          </div>
        </Section>

        {/* Danger */}
        <Section title="Danger zone">
          <Button variant="danger" style={{ alignSelf: 'flex-start' }}>⚠ Reset workspace</Button>
        </Section>
      </div>

      <AddBackendModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onAdd={(b) => { setBackends(prev => [...prev, b]); setAddOpen(false); }}
      />
    </Modal>
  );
}

function NetworkBadge({ net }) {
  const palette = {
    BTC:    { fg: '#b16a12', bg: 'rgba(177,106,18,0.10)', border: 'rgba(177,106,18,0.45)' },
    LIQUID: { fg: '#3e5ea8', bg: 'rgba(62,94,168,0.10)',  border: 'rgba(62,94,168,0.45)'  },
    LN:     { fg: '#7a3fa6', bg: 'rgba(122,63,166,0.10)', border: 'rgba(122,63,166,0.45)' },
    FX:     { fg: 'var(--ink-2)', bg: 'transparent',     border: 'var(--ink-3)'            },
  };
  const p = palette[net] || palette.FX;
  return (
    <span style={{
      display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
      padding: '2px 8px',
      border: '1px solid ' + p.border,
      background: p.bg,
      color: p.fg,
      fontFamily: 'var(--mono)', fontSize: 9, fontWeight: 700,
      letterSpacing: '0.14em',
    }}>{net}</span>
  );
}

function Section({ title, children }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={{
        fontFamily: 'var(--mono)', fontSize: 10, fontWeight: 600,
        letterSpacing: '0.14em', textTransform: 'uppercase',
        color: 'var(--ink-3)',
        borderBottom: '1px solid var(--line)',
        paddingBottom: 6,
      }}>{title}</div>
      {children}
    </div>
  );
}

function Row({ label, sub, control }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '6px 0' }}>
      <div style={{ flex: 1 }}>
        <div style={{ fontFamily: 'var(--sans)', fontSize: 13, color: 'var(--ink)' }}>{label}</div>
        {sub && <div style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)', marginTop: 2 }}>{sub}</div>}
      </div>
      {control}
    </div>
  );
}

function Toggle({ on, onChange }) {
  const [val, setVal] = React.useState(!!on);
  React.useEffect(() => { if (onChange === undefined) setVal(!!on); }, [on]);
  const isOn = onChange ? on : val;
  const handle = () => { if (onChange) onChange(!on); else setVal(v => !v); };
  return (
    <button onClick={handle} style={{
      width: 36, height: 20, background: isOn ? 'var(--ink)' : 'var(--line-2)',
      border: 'none', padding: 0, cursor: 'pointer',
      position: 'relative', transition: 'background 0.15s',
    }}>
      <div style={{
        position: 'absolute', top: 2, left: isOn ? 18 : 2,
        width: 16, height: 16, background: 'var(--paper-2)',
        transition: 'left 0.15s',
      }}/>
    </button>
  );
}

window.SettingsModal = SettingsModal;

// ——— Add Backend modal ——————————————————————————————————————————————
// Generic flow:  pick TYPE (btc/lightning/liquid/fx/other) →
//                pick PRESET or Custom → fill URL + auth → test → add.

const BACKEND_TYPES = [
  {
    id: 'btc', label: 'Bitcoin node',
    net: 'BTC',
    desc: 'Read blocks, addresses and UTXOs from a Bitcoin backend.',
    presets: [
      { id: 'mempool',   name: 'mempool.space',   url: 'https://mempool.space/api',                scheme: 'REST' },
      { id: 'esplora',   name: 'Blockstream Esplora', url: 'https://blockstream.info/api',        scheme: 'REST' },
      { id: 'electrum',  name: 'Electrum server', url: 'tcp://127.0.0.1:50001',                    scheme: 'Electrum' },
      { id: 'core',      name: 'Bitcoin Core RPC', url: 'http://127.0.0.1:8332',                   scheme: 'RPC' },
    ],
  },
  {
    id: 'lightning', label: 'Lightning',
    net: 'LN',
    desc: 'Read channel state, invoices and forwards from an LN node.',
    presets: [
      { id: 'lnd',       name: 'LND',     url: 'https://127.0.0.1:8080',     scheme: 'REST' },
      { id: 'cln',       name: 'Core Lightning', url: 'http://127.0.0.1:3010', scheme: 'CLNREST' },
      { id: 'lnbits',    name: 'LNbits',  url: 'https://your.lnbits.host',    scheme: 'REST' },
      { id: 'nwc',       name: 'Nostr Wallet Connect', url: 'nostr+walletconnect://', scheme: 'NWC' },
    ],
  },
  {
    id: 'liquid', label: 'Liquid / sidechain',
    net: 'LIQUID',
    desc: 'Read Liquid, Rootstock or other sidechain balances.',
    presets: [
      { id: 'blockstream', name: 'Blockstream Liquid', url: 'https://blockstream.info/liquid/api', scheme: 'REST' },
      { id: 'liquidcore',  name: 'Elements RPC',        url: 'http://127.0.0.1:7041',               scheme: 'RPC' },
    ],
  },
  {
    id: 'fx', label: 'Price / FX',
    net: 'FX',
    desc: 'BTC/EUR and other fiat reference rates, spot and historical.',
    presets: [
      { id: 'coingecko', name: 'CoinGecko',   url: 'https://api.coingecko.com/api/v3', scheme: 'REST' },
      { id: 'kraken',    name: 'Kraken',      url: 'https://api.kraken.com/0/public',  scheme: 'REST' },
      { id: 'bitstamp',  name: 'Bitstamp',    url: 'https://www.bitstamp.net/api/v2',  scheme: 'REST' },
      { id: 'ecb',       name: 'ECB reference', url: 'https://data-api.ecb.europa.eu/service/data', scheme: 'REST' },
    ],
  },
  {
    id: 'other', label: 'Other',
    net: 'FX',
    desc: 'A generic HTTP / WebSocket endpoint.',
    presets: [],
  },
];

const AUTH_MODES = [
  { id: 'none',    label: 'None' },
  { id: 'apikey',  label: 'API key' },
  { id: 'basic',   label: 'User + pass' },
  { id: 'bearer',  label: 'Bearer token' },
];

function AddBackendModal({ open, onClose, onAdd }) {
  const [typeId, setTypeId] = React.useState('btc');
  const [presetId, setPresetId] = React.useState('mempool');
  const [name, setName] = React.useState('');
  const [url, setUrl] = React.useState('https://mempool.space/api');
  const [auth, setAuth] = React.useState('none');
  const [authVal, setAuthVal] = React.useState('');
  const [authVal2, setAuthVal2] = React.useState('');
  const [testState, setTestState] = React.useState('idle'); // idle | testing | ok | fail

  const type = BACKEND_TYPES.find(t => t.id === typeId) || BACKEND_TYPES[0];
  const preset = presetId === 'custom' ? null : type.presets.find(p => p.id === presetId);

  // Reset state when modal opens
  React.useEffect(() => {
    if (!open) return;
    setTypeId('btc');
    setPresetId('mempool');
    setName('');
    setUrl('https://mempool.space/api');
    setAuth('none');
    setAuthVal(''); setAuthVal2('');
    setTestState('idle');
  }, [open]);

  // When type or preset changes, fill url + suggested name
  React.useEffect(() => {
    if (!open) return;
    if (preset) {
      setUrl(preset.url);
      setName(preset.name);
    } else if (presetId === 'custom') {
      setUrl('');
      setName('');
    }
    setTestState('idle');
  }, [typeId, presetId]);

  const onPickType = (id) => {
    setTypeId(id);
    const t = BACKEND_TYPES.find(x => x.id === id);
    setPresetId(t.presets[0]?.id || 'custom');
  };

  const testConnection = () => {
    if (!url.trim()) return;
    setTestState('testing');
    setTimeout(() => {
      // deterministic-looking pseudo test — succeeds for https/http/tcp URLs with a host
      const ok = /^(https?|tcp|wss?|nostr\+walletconnect):\/\/[\w.\-:\/]+/i.test(url.trim());
      setTestState(ok ? 'ok' : 'fail');
    }, 900);
  };

  const canAdd = name.trim() && url.trim();
  const add = () => {
    if (!canAdd) return;
    onAdd({
      id: 'b' + Date.now(),
      name: name.trim(),
      url: url.trim(),
      net: type.net,
      health: testState === 'ok' ? 'just added · ok' : '—',
      on: testState === 'ok',
      auth,
    });
  };

  return (
    <Modal open={open} onClose={onClose} title="Add backend" width={620}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
        {/* 1 — Type selector */}
        <div>
          <SectionLabel step="01" label="Backend type" />
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 6, marginTop: 8 }}>
            {BACKEND_TYPES.map(t => {
              const active = t.id === typeId;
              return (
                <button key={t.id} onClick={() => onPickType(t.id)} style={{
                  textAlign: 'left',
                  padding: '10px 10px',
                  border: '1px solid ' + (active ? 'var(--ink)' : 'var(--line)'),
                  background: active ? 'var(--paper-2)' : 'transparent',
                  boxShadow: active ? '3px 3px 0 var(--ink)' : 'none',
                  cursor: 'pointer',
                  display: 'flex', flexDirection: 'column', gap: 6,
                  minHeight: 72,
                }}>
                  <NetworkBadge net={t.net} />
                  <span style={{ fontFamily: 'var(--sans)', fontSize: 12, fontWeight: 500, color: 'var(--ink)', lineHeight: 1.2 }}>{t.label}</span>
                </button>
              );
            })}
          </div>
          <div style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)', marginTop: 8 }}>
            {type.desc}
          </div>
        </div>

        {/* 2 — Preset */}
        {(type.presets.length > 0) && (
          <div>
            <SectionLabel step="02" label="Preset" />
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 8 }}>
              {type.presets.map(p => (
                <Pill key={p.id} active={presetId === p.id} onClick={() => setPresetId(p.id)}
                  color={presetId === p.id ? 'ink' : 'muted'}>
                  {p.name}
                  <span style={{ opacity: 0.55, marginLeft: 6, fontSize: 9 }}>{p.scheme}</span>
                </Pill>
              ))}
              <Pill active={presetId === 'custom'} onClick={() => setPresetId('custom')}
                color={presetId === 'custom' ? 'ink' : 'muted'}>+ Custom</Pill>
            </div>
          </div>
        )}

        {/* 3 — Connection details */}
        <div>
          <SectionLabel step={type.presets.length > 0 ? '03' : '02'} label="Connection" />
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginTop: 8 }}>
            <Input label="Display name" value={name} onChange={e => setName(e.target.value)}
              placeholder="e.g. My home node" />
            <Input label="Endpoint URL" value={url} onChange={e => { setUrl(e.target.value); setTestState('idle'); }}
              placeholder="https://…" mono />

            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <span style={{
                fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600,
                letterSpacing: '0.12em', textTransform: 'uppercase',
                color: 'var(--ink-2)',
              }}>Authentication</span>
              <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                {AUTH_MODES.map(m => (
                  <Pill key={m.id} active={auth === m.id} onClick={() => setAuth(m.id)}
                    color={auth === m.id ? 'ink' : 'muted'}>{m.label}</Pill>
                ))}
              </div>
              {auth === 'apikey' && (
                <Input label="API key" value={authVal} onChange={e => setAuthVal(e.target.value)}
                  placeholder="sk_live_…" type="password" mono />
              )}
              {auth === 'bearer' && (
                <Input label="Bearer token" value={authVal} onChange={e => setAuthVal(e.target.value)}
                  placeholder="eyJ…" type="password" mono />
              )}
              {auth === 'basic' && (
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                  <Input label="Username" value={authVal} onChange={e => setAuthVal(e.target.value)} mono />
                  <Input label="Password" value={authVal2} onChange={e => setAuthVal2(e.target.value)}
                    type="password" mono />
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Test + footer */}
        <div style={{
          borderTop: '1px solid var(--ink)', paddingTop: 14,
          display: 'flex', alignItems: 'center', gap: 10,
        }}>
          <Button variant="secondary" size="sm" onClick={testConnection}
            disabled={!url.trim() || testState === 'testing'}
            icon={<svg width="10" height="10" viewBox="0 0 10 10" fill="none"
              style={{ animation: testState === 'testing' ? 'kb-spin 0.7s linear infinite' : 'none' }}>
              <path d="M1.6 5 A3.4 3.4 0 1 1 5 8.4" stroke="currentColor" strokeWidth="1.1" fill="none" strokeLinecap="round"/>
              <path d="M1.6 5 L1.6 2.3 L4.3 2.3" stroke="currentColor" strokeWidth="1.1" fill="none" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>}
          >{testState === 'testing' ? 'Testing…' : 'Test connection'}</Button>

          {testState === 'ok' && (
            <span style={{
              fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.1em',
              textTransform: 'uppercase', color: '#3fa66a',
              display: 'inline-flex', alignItems: 'center', gap: 6,
            }}>
              <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#3fa66a' }} />
              Connected · 142 ms
            </span>
          )}
          {testState === 'fail' && (
            <span style={{
              fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.1em',
              textTransform: 'uppercase', color: 'var(--accent)',
            }}>⚠ Could not reach endpoint</span>
          )}

          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <Button variant="ghost" size="md" onClick={onClose}>Cancel</Button>
            <Button size="md" onClick={add} disabled={!canAdd}>Add backend</Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}

function SectionLabel({ step, label }) {
  return (
    <div style={{
      display: 'flex', alignItems: 'baseline', gap: 8,
      borderBottom: '1px solid var(--line)', paddingBottom: 6,
    }}>
      <span style={{
        fontFamily: 'var(--mono)', fontSize: 9, fontWeight: 700,
        letterSpacing: '0.14em', color: 'var(--accent)',
      }}>{step}</span>
      <span style={{
        fontFamily: 'var(--mono)', fontSize: 10, fontWeight: 600,
        letterSpacing: '0.14em', textTransform: 'uppercase',
        color: 'var(--ink-2)',
      }}>{label}</span>
    </div>
  );
}

// spin keyframes (shared with footer refresh icon)
if (typeof document !== 'undefined' && !document.getElementById('kb-spin-kf')) {
  const s = document.createElement('style');
  s.id = 'kb-spin-kf';
  s.textContent = '@keyframes kb-spin { to { transform: rotate(360deg); } }';
  document.head.appendChild(s);
}

window.AddBackendModal = AddBackendModal;
