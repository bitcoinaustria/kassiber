// Connection flows: type picker modal, XPub form, descriptor form, connection detail page

function ConnectionTypePicker({ open, onClose, onPick, t }) {
  const sections = [
    {
      label: 'Self-custody · On-chain',
      items: ['xpub', 'descriptor'],
    },
    {
      label: 'Lightning',
      items: ['core-ln', 'lnd', 'nwc'],
    },
    {
      label: 'Services · Merchant',
      items: ['btcpay', 'cashu'],
    },
    {
      label: 'Exchanges · Read-only API',
      items: ['kraken', 'bitstamp', 'coinbase', 'bitpanda', 'river', 'strike'],
    },
    {
      label: 'File',
      items: ['csv'],
    },
  ];
  return (
    <Modal open={open} onClose={onClose} title={t.add.title} width={720}>
      {/* Sticky intro */}
      <div style={{
        position: 'sticky', top: 0, zIndex: 2,
        background: 'var(--paper)',
        margin: '-4px -4px 0',
        padding: '4px 4px 14px',
      }}>
        <p style={{ margin: 0, fontFamily: 'var(--sans)', fontSize: 13, color: 'var(--ink-2)' }}>
          {t.add.sub}
        </p>
      </div>

      {/* Scrollable list */}
      <div style={{
        maxHeight: 440,
        overflowY: 'auto',
        margin: '0 -4px',
        padding: '4px',
        borderTop: '1px solid var(--line)',
        borderBottom: '1px solid var(--line)',
      }}>
        {sections.map((sec, si) => (
          <div key={sec.label} style={{ marginTop: si === 0 ? 10 : 18 }}>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 10,
              fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.16em',
              textTransform: 'uppercase', color: 'var(--ink-3)',
              marginBottom: 8,
            }}>
              <span>{sec.label}</span>
              <span style={{ flex: 1, height: 1, background: 'var(--line)' }} />
              <span>{String(sec.items.length).padStart(2, '0')}</span>
            </div>
            <div style={{
              display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 6,
            }}>
              {sec.items.map(k => {
                const info = t.kind[k];
                if (!info) return null;
                return (
                  <button key={k} onClick={() => onPick(k)} style={{
                    display: 'grid',
                    gridTemplateColumns: '1fr auto',
                    alignItems: 'center',
                    gap: 12,
                    padding: '12px 14px',
                    background: 'transparent',
                    border: '1px solid var(--line)',
                    cursor: 'pointer', textAlign: 'left',
                    transition: 'background 0.12s, border-color 0.12s, transform 0.12s',
                  }}
                    onMouseEnter={e => { e.currentTarget.style.background = 'var(--paper)'; e.currentTarget.style.borderColor = 'var(--ink)'; }}
                    onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = 'var(--line)'; }}
                  >
                    <div style={{ minWidth: 0 }}>
                      <div style={{
                        fontFamily: 'var(--sans)', fontSize: 14, fontWeight: 600,
                        color: 'var(--ink)', letterSpacing: '-0.005em',
                      }}>{info.name}</div>
                      <div style={{
                        fontFamily: 'var(--mono)', fontSize: 10,
                        color: 'var(--ink-3)', marginTop: 2, letterSpacing: '0.04em',
                      }}>
                        {info.desc.toUpperCase()}
                      </div>
                    </div>
                    <span style={{
                      fontFamily: 'var(--mono)', fontSize: 13, color: 'var(--ink-3)',
                    }}>→</span>
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </div>

      {/* Sticky footer */}
      <div style={{
        position: 'sticky', bottom: 0, zIndex: 2,
        background: 'var(--paper)',
        margin: '14px -4px -4px',
        padding: '4px',
      }}>
        <div style={{
          padding: 12, background: 'var(--paper-2)', border: '1px solid var(--line)',
          display: 'flex', gap: 10, alignItems: 'flex-start',
        }}>
          <svg width="14" height="14" viewBox="0 0 14 14" style={{ flexShrink: 0, marginTop: 2 }}>
            <rect x="3" y="6" width="8" height="6" stroke="var(--accent)" strokeWidth="1.2" fill="none"/>
            <path d="M5 6 V4.5 Q5 2.5 7 2.5 Q9 2.5 9 4.5 V6" stroke="var(--accent)" strokeWidth="1.2" fill="none"/>
          </svg>
          <span style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-2)', lineHeight: 1.55 }}>
            Watch-only by design. Kassiber imports history via extended public keys, descriptors, or read-only API credentials. <b>No private keys or withdrawal permissions ever touch this machine through Kassiber.</b>
          </span>
        </div>
      </div>
    </Modal>
  );
}

function XpubForm({ open, onClose, onBack, onSave }) {
  const [label, setLabel] = React.useState('Cold Storage');
  const [xpub, setXpub] = React.useState('');
  const [addrTypes, setAddrTypes] = React.useState({ p2wpkh: true, p2tr: false, p2pkh: false, p2sh: false });
  const [gap, setGap] = React.useState(10);

  const EXAMPLE = 'xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j…';

  return (
    <Modal open={open} onClose={onClose} title="XPub" back={onBack} width={620}>
      <p style={{ margin: '0 0 20px', fontFamily: 'var(--sans)', fontSize: 13, color: 'var(--ink-2)' }}>
        Enter your extended public key. Kassiber will derive addresses and sync on-chain history.
      </p>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <Input label="Connection label" value={label} onChange={e => setLabel(e.target.value)} placeholder="e.g. Cold Storage" />

        <div>
          <Input label="xpub / ypub / zpub" value={xpub} onChange={e => setXpub(e.target.value)} placeholder={EXAMPLE} mono rightAdornment="paste" />
          <div style={{ display: 'flex', gap: 16, marginTop: 6, fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>
            <span>Detected: <span style={{color: 'var(--ink-2)'}}>{xpub ? (xpub.startsWith('zpub') ? 'BIP84 · native segwit' : xpub.startsWith('ypub') ? 'BIP49 · nested' : 'BIP44') : '—'}</span></span>
            <span>Fingerprint: <span style={{color: 'var(--ink-2)'}}>{xpub ? '5f3a · 8c0e' : '—'}</span></span>
          </div>
        </div>

        <div>
          <div style={{ fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-2)', marginBottom: 8 }}>
            Address types to derive
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
            {[
              ['p2pkh', 'Pay to Public Key Hash',  '1A1zP1…'],
              ['p2sh',  'Pay to Script Hash',       '3J98t1…'],
              ['p2wpkh','Pay to Witness Pub Hash',  'bc1qar…'],
              ['p2tr',  'Pay to Taproot',           'bc1p5c…'],
            ].map(([k, label, prefix]) => (
              <label key={k} style={{
                display: 'flex', alignItems: 'center', gap: 10,
                padding: '8px 10px',
                border: '1px solid var(--line)',
                cursor: 'pointer',
                background: addrTypes[k] ? 'var(--paper)' : 'transparent',
              }}>
                <input type="checkbox" checked={addrTypes[k]}
                  onChange={() => setAddrTypes(a => ({...a, [k]: !a[k]}))}
                  style={{ accentColor: 'var(--accent)' }} />
                <span style={{ flex: 1, fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)' }}>{label}</span>
                <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{prefix}</span>
              </label>
            ))}
          </div>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <Input label="Gap limit" value={gap} onChange={e => setGap(+e.target.value || 0)} type="number" mono />
          <div>
            <div style={{ fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-2)', marginBottom: 6 }}>
              Sync backend
            </div>
            <div style={{ border: '1px solid var(--line)', padding: '8px 10px', fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)', background: 'var(--paper-2)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              Mempool.space (default)
              <svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 4 L5 7 L8 4" stroke="var(--ink-3)" strokeWidth="1.2" fill="none"/></svg>
            </div>
          </div>
        </div>

        <Rule />

        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button onClick={() => onSave({ kind: 'xpub', label, balance: 0, last: 'never', status: 'syncing', addresses: 0, gap })}
            icon={<svg width="12" height="12" viewBox="0 0 12 12"><path d="M3 6 L5 8 L9 4" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>}
          >Save and sync</Button>
        </div>
      </div>
    </Modal>
  );
}

// Connection detail page — accessed from Connections list or overview tile
function ConnectionDetail({ conn, onBack, hideSensitive, onImportLabels, onExportLabels }) {
  if (!conn) return null;
  const txs = MOCK.txs.filter(t => t.account.toLowerCase().includes(conn.label.toLowerCase().split(' ')[0].toLowerCase())).slice(0, 5);
  return (
    <div style={{ flex: 1, overflow: 'auto', background: 'var(--paper)', padding: 18 }}>
      {/* header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 20 }}>
        <button onClick={onBack} style={{
          background: 'transparent', border: '1px solid var(--line)', width: 32, height: 32, cursor: 'pointer',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <svg width="12" height="12" viewBox="0 0 12 12"><path d="M8 2 L3 6 L8 10" stroke="var(--ink)" strokeWidth="1.5" fill="none" strokeLinecap="round" strokeLinejoin="round"/></svg>
        </button>
        <ProtocolIcon kind={conn.kind} size={40} />
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.12em', textTransform: 'uppercase' }}>
            {conn.kind} · Connection
          </div>
          <h2 style={{ margin: 0, fontFamily: 'var(--sans)', fontSize: 30, fontWeight: 600, letterSpacing: '-0.01em', color: 'var(--ink)' }}>
            {conn.label}
          </h2>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <Button variant="secondary" size="sm">⟳ Sync</Button>
          <Button variant="secondary" size="sm" onClick={onImportLabels}>↓ Import labels</Button>
          <Button variant="secondary" size="sm" onClick={onExportLabels}>↑ Export labels</Button>
          <div style={{ width: 1, height: 22, background: 'var(--line)', margin: '0 2px', alignSelf: 'center' }} />
          <Button variant="ghost" size="sm">Edit</Button>
          <Button variant="danger" size="sm">Remove</Button>
        </div>
      </div>

      {/* top stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10, marginBottom: 18 }}>
        <StatTile label="Balance" value={<span className={hideSensitive?'sensitive':''}>{conn.balance.toFixed(8)} ₿</span>} sub={`€ ${(conn.balance * MOCK.priceEur).toLocaleString('de-AT', {minimumFractionDigits:2, maximumFractionDigits:2})}`}/>
        <StatTile label="Addresses" value={conn.addresses || conn.channels || '—'} sub={conn.kind === 'core-ln' ? 'channels' : 'derived'} />
        <StatTile label="Last sync" value={conn.last} sub={conn.status} />
        <StatTile label="Gap limit" value={conn.gap || '—'} sub="unused window" />
      </div>

      {/* two columns: transactions | addresses */}
      <div style={{ display: 'grid', gridTemplateColumns: '1.4fr 1fr', gap: 10 }}>
        <Card title="Recent transactions" pad={false}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: 'var(--mono)', fontSize: 11 }}>
            <thead><tr style={{ borderBottom: '1px solid var(--line)' }}>
              <th style={thStyle}>Date</th><th style={thStyle}>Type</th><th style={{...thStyle,textAlign:'right'}}>sats</th><th style={{...thStyle,textAlign:'right'}}>€</th><th style={{...thStyle,textAlign:'right'}}>conf</th>
            </tr></thead>
            <tbody>
              {(txs.length ? txs : MOCK.txs.slice(0, 5)).map(tx => (
                <tr key={tx.id} style={{ borderBottom: '1px solid var(--line)' }}>
                  <td style={tdStyle}>{tx.date.slice(5)}</td>
                  <td style={tdStyle}>{tx.type}</td>
                  <td style={{...tdStyle, textAlign:'right', color: tx.amountSat>0?'#3fa66a':'var(--ink)'}} className={hideSensitive?'sensitive':''}>
                    {(tx.amountSat > 0 ? '+' : '') + tx.amountSat.toLocaleString('en-US')}
                  </td>
                  <td style={{...tdStyle, textAlign:'right'}} className={hideSensitive?'sensitive':''}>
                    {(tx.eur > 0 ? '+ €' : '− €') + Math.abs(tx.eur).toFixed(2)}
                  </td>
                  <td style={{...tdStyle, textAlign:'right', color: 'var(--ink-3)'}}>{tx.conf}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <Card title="Connection details" pad>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <KV k="Label" v={conn.label} mono={false} />
              <KV k="Type" v={conn.kind.toUpperCase()} />
              <KV k="Derivation path" v={conn.kind === 'xpub' ? "m / 84' / 0' / 0'" : '—'} />
              {(conn.kind === 'xpub' || conn.kind === 'descriptor') && (
                <>
                  <KV k="Fingerprint" v="5f3a8c0e" copy />
                  <KVReveal k="Account xpub" full="xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j" short="xpub6C…aQL2j" hideSensitive={hideSensitive} />
                </>
              )}
              <KV k="Backend" v="mempool.space" mono={false} />
              <KV k="Created" v="2026-03-02 10:14" />
              <KV k="Kassiber ID" v="conn_01HX2..3f7k" />
            </div>
          </Card>
          <Card title="Derived addresses" pad={false}>
            <div style={{ maxHeight: 180, overflow: 'auto' }}>
              {['bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq', 'bc1q9d4ywgfnd8h43da5tpcxcn6ajv590cg6d3tg6a', 'bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh'].map((a, i) => (
                <div key={a} style={{ display: 'flex', justifyContent: 'space-between', padding: '7px 14px', borderTop: i === 0 ? 'none' : '1px solid var(--line)', fontFamily: 'var(--mono)', fontSize: 10 }}>
                  <span className={hideSensitive?'sensitive':''} style={{ color: 'var(--ink)' }}>{a.slice(0, 28)}…</span>
                  <span style={{ color: 'var(--ink-3)' }}>m/84'/0'/0'/0/{i}</span>
                </div>
              ))}
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}

function KVReveal({ k, full, short, hideSensitive }) {
  const [revealed, setRevealed] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const masked = !revealed || hideSensitive;
  const onCopy = async (e) => {
    e.stopPropagation();
    try { await navigator.clipboard.writeText(full); } catch {}
    setCopied(true);
    setTimeout(() => setCopied(false), 1100);
  };
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
      <span style={{
        fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 500,
        letterSpacing: '0.12em', textTransform: 'uppercase',
        color: 'var(--ink-3)',
      }}>{k}</span>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
        <span className={masked ? 'sensitive' : ''} style={{
          flex: 1, minWidth: 0,
          fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--ink)',
          letterSpacing: '-0.01em',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>{short}</span>
        <button onClick={() => setRevealed(r => !r)} title={revealed ? 'Hide' : 'Reveal'} style={iconBtnStyle}>
          {revealed ? (
            <svg width="10" height="10" viewBox="0 0 14 14" fill="none">
              <path d="M1.5 7 Q7 2.5 12.5 7 Q11 8.8 9 9.8" stroke="var(--ink-2)" strokeWidth="1" fill="none" strokeLinecap="round"/>
              <path d="M5 9.5 Q4 8.5 4 7 Q4 5.5 5.5 4.8" stroke="var(--ink-2)" strokeWidth="1" fill="none" strokeLinecap="round"/>
              <path d="M2 2 L12 12" stroke="var(--ink-2)" strokeWidth="1" strokeLinecap="round"/>
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 14 14" fill="none">
              <path d="M1.5 7 Q7 2 12.5 7 Q7 12 1.5 7 Z" stroke="var(--ink-2)" strokeWidth="1" fill="none"/>
              <circle cx="7" cy="7" r="1.7" stroke="var(--ink-2)" strokeWidth="1" fill="none"/>
            </svg>
          )}
        </button>
        <button onClick={onCopy} title={copied ? 'Copied' : 'Copy'} style={iconBtnStyle}>
          {copied ? (
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <path d="M2 5 L4 7 L8 3" stroke="#3fa66a" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" fill="none"/>
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <rect x="2.5" y="2.5" width="5" height="5.5" stroke="var(--ink-2)" strokeWidth="0.9" fill="none"/>
              <path d="M4 2.5 V1.5 H8.5 V6.5 H7.5" stroke="var(--ink-2)" strokeWidth="0.9" fill="none"/>
            </svg>
          )}
        </button>
      </div>
    </div>
  );
}

const iconBtnStyle = {
  flexShrink: 0,
  background: 'transparent', border: '1px solid var(--line)',
  width: 20, height: 20, padding: 0, cursor: 'pointer',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
};

function StatTile({ label, value, sub }) {
  return (
    <div style={{
      background: 'var(--paper-2)',
      border: '1px solid var(--line)',
      padding: 14,
    }}>
      <div style={{ fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
        {label}
      </div>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 18, color: 'var(--ink)', marginTop: 6, letterSpacing: '-0.01em' }}>
        {value}
      </div>
      {sub && <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', marginTop: 3, letterSpacing: '0.05em' }}>{sub}</div>}
    </div>
  );
}

window.ConnectionTypePicker = ConnectionTypePicker;
window.XpubForm = XpubForm;
window.ConnectionDetail = ConnectionDetail;
window.Bip329Form = Bip329Form;

// ——— BIP-329 Labels import ——————————————————————————————————————————
// BIP-329 is JSON Lines, one record per line:
//   {"type":"tx","ref":"<txid>","label":"rent payment"}
//   {"type":"addr","ref":"bc1q…","label":"donations","spendable":true}
//   {"type":"output","ref":"<txid>:vout","label":"change"}
//   {"type":"input","ref":"<txid>:vin","label":"exchange deposit"}
//   {"type":"xpub","ref":"xpub…","label":"Cold Storage"}
//
// The mock below simulates parsing a dropped/loaded .jsonl file.

const BIP329_SAMPLE = [
  { type: 'tx',     ref: '4a3f…e1c9', label: 'Rent · March',              ok: true  },
  { type: 'tx',     ref: 'b7d2…0f84', label: 'Salary · Acme GmbH',        ok: true  },
  { type: 'addr',   ref: 'bc1qar0srrr7xfkvy5l…', label: 'Donations',      ok: true, spendable: true },
  { type: 'addr',   ref: 'bc1q9d4ywgfnd8h43da…', label: 'Cold receive',   ok: true, spendable: true },
  { type: 'output', ref: '4a3f…e1c9:1',  label: 'Change',                 ok: true  },
  { type: 'output', ref: 'c09e…ab2d:0',  label: 'Merchant payout',        ok: true  },
  { type: 'input',  ref: 'b7d2…0f84:0',  label: 'Kraken withdrawal',      ok: true  },
  { type: 'xpub',   ref: 'xpub6CUGRU…',  label: 'Cold Storage',           ok: true  },
  { type: 'tx',     ref: '00ff…dead',    label: '(not in wallet)',        ok: false },
];

function Bip329Form({ open, onClose, onBack, onImport, connections, scopedConnId }) {
  const [file, setFile] = React.useState(null);       // { name, size }
  const [scope, setScope] = React.useState(scopedConnId || 'all');
  const [strategy, setStrategy] = React.useState('keep'); // 'keep' | 'overwrite'
  const [onlySpendable, setOnlySpendable] = React.useState(false);
  const fileInputRef = React.useRef(null);

  React.useEffect(() => { if (open) setScope(scopedConnId || 'all'); }, [open, scopedConnId]);

  const counts = React.useMemo(() => {
    if (!file) return null;
    const c = { tx: 0, addr: 0, output: 0, input: 0, xpub: 0, unmatched: 0 };
    BIP329_SAMPLE.forEach(r => {
      if (!r.ok) c.unmatched++;
      else c[r.type]++;
    });
    return c;
  }, [file]);

  const onCanvasDrop = (e) => {
    e.preventDefault();
    const f = e.dataTransfer?.files?.[0];
    if (f) setFile({ name: f.name, size: f.size });
  };
  const onPick = (e) => {
    const f = e.target.files?.[0];
    if (f) setFile({ name: f.name, size: f.size });
  };
  const mockPick = () => setFile({ name: 'labels.jsonl', size: 2714 });

  const totalValid = counts ? (counts.tx + counts.addr + counts.output + counts.input + counts.xpub) : 0;

  return (
    <Modal open={open} onClose={onClose} title="Import BIP-329 labels" back={onBack} width={680}>
      <p style={{ margin: '0 0 16px', fontFamily: 'var(--sans)', fontSize: 13, color: 'var(--ink-2)', lineHeight: 1.55 }}>
        BIP-329 is a wallet-agnostic format for <b>labels</b> — human-readable notes attached to transactions, addresses, outputs, inputs, or xpubs. Kassiber matches each record against your existing connections; nothing is imported for entries it cannot match.
      </p>

      {/* Drop zone / file info */}
      {!file ? (
        <div
          onDragOver={e => e.preventDefault()}
          onDrop={onCanvasDrop}
          style={{
            border: '1px dashed var(--ink-3)',
            padding: '32px 20px',
            display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 10,
            background: 'var(--paper-2)',
            textAlign: 'center',
          }}
        >
          <div style={{
            fontFamily: 'var(--sans)', fontSize: 14, color: 'var(--ink)', fontWeight: 600,
          }}>Drop a <span style={{ fontFamily: 'var(--mono)' }}>.jsonl</span> file here</div>
          <div style={{
            fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.12em',
            textTransform: 'uppercase', color: 'var(--ink-3)',
          }}>or</div>
          <div style={{ display: 'flex', gap: 8 }}>
            <Button size="sm" variant="secondary" onClick={() => fileInputRef.current?.click()}>Choose file…</Button>
            <Button size="sm" variant="ghost" onClick={mockPick}>Use sample</Button>
          </div>
          <input ref={fileInputRef} type="file" accept=".jsonl,application/jsonl,application/json" onChange={onPick} style={{ display: 'none' }} />
          <div style={{ marginTop: 6, fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.04em' }}>
            Spec: github.com/bitcoin/bips/blob/master/bip-0329.mediawiki
          </div>
        </div>
      ) : (
        <div style={{ border: '1px solid var(--ink)', background: 'var(--paper-2)', padding: 14, display: 'flex', alignItems: 'center', gap: 14 }}>
          <svg width="28" height="28" viewBox="0 0 28 28" fill="none" aria-hidden="true">
            <rect x="5" y="3" width="16" height="22" stroke="var(--ink)" strokeWidth="1.4" fill="none"/>
            <path d="M15 3 L15 8 L21 8" stroke="var(--ink)" strokeWidth="1.4" fill="none"/>
            <text x="8" y="20" fontFamily="monospace" fontSize="7" fill="var(--ink-2)">{`{ }`}</text>
          </svg>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--ink)' }}>{file.name}</div>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.04em', marginTop: 2 }}>
              {(file.size/1024).toFixed(1)} kB · {BIP329_SAMPLE.length} records parsed
            </div>
          </div>
          <button onClick={() => setFile(null)} style={{
            background: 'transparent', border: '1px solid var(--line)', padding: '4px 10px', cursor: 'pointer',
            fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.1em', textTransform: 'uppercase', color: 'var(--ink-2)',
          }}>Replace</button>
        </div>
      )}

      {file && counts && (
        <>
          {/* Record type counts */}
          <div style={{
            display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 6, marginTop: 14,
          }}>
            {[
              ['TX',     counts.tx],
              ['ADDR',   counts.addr],
              ['OUTPUT', counts.output],
              ['INPUT',  counts.input],
              ['XPUB',   counts.xpub],
              ['SKIP',   counts.unmatched, true],
            ].map(([k, v, warn]) => (
              <div key={k} style={{
                border: '1px solid ' + (warn && v > 0 ? 'var(--accent)' : 'var(--line)'),
                padding: '8px 10px',
                background: warn && v > 0 ? 'rgba(166,47,47,0.05)' : 'transparent',
              }}>
                <div style={{
                  fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em',
                  color: warn && v > 0 ? 'var(--accent)' : 'var(--ink-3)', fontWeight: 600,
                }}>{k}</div>
                <div style={{
                  fontFamily: 'var(--mono)', fontSize: 18, color: 'var(--ink)', marginTop: 2,
                }}>{v}</div>
              </div>
            ))}
          </div>

          {/* Preview table */}
          <div style={{
            marginTop: 14,
            border: '1px solid var(--line)',
            maxHeight: 200, overflowY: 'auto',
          }}>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ borderBottom: '1px solid var(--line)', background: 'var(--paper-2)', position: 'sticky', top: 0 }}>
                  <th style={bip329Th}>Type</th>
                  <th style={bip329Th}>Ref</th>
                  <th style={bip329Th}>Label</th>
                  <th style={{...bip329Th, textAlign: 'right'}}>Match</th>
                </tr>
              </thead>
              <tbody>
                {BIP329_SAMPLE.map((r, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid var(--line)', opacity: r.ok ? 1 : 0.55 }}>
                    <td style={bip329Td}>
                      <span style={{
                        display: 'inline-block',
                        padding: '2px 6px',
                        fontSize: 9, letterSpacing: '0.12em', fontWeight: 700,
                        border: '1px solid var(--line)',
                        color: 'var(--ink-2)',
                        background: 'var(--paper-2)',
                      }}>{r.type.toUpperCase()}</span>
                    </td>
                    <td style={{...bip329Td, color: 'var(--ink-2)'}}>{r.ref}</td>
                    <td style={{...bip329Td, fontFamily: 'var(--sans)', color: 'var(--ink)'}}>{r.label}</td>
                    <td style={{...bip329Td, textAlign: 'right'}}>
                      {r.ok
                        ? <span style={{ color: '#3fa66a' }}>✓</span>
                        : <span style={{ color: 'var(--accent)' }}>— not in wallet</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Scope + strategy */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginTop: 14 }}>
            <div>
              <div style={{
                fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600, letterSpacing: '0.12em',
                textTransform: 'uppercase', color: 'var(--ink-2)', marginBottom: 6,
              }}>Apply to</div>
              <div style={{ border: '1px solid var(--line)', padding: '0', background: 'var(--paper-2)' }}>
                <select value={scope} onChange={e => setScope(e.target.value)} style={{
                  width: '100%',
                  padding: '9px 10px',
                  background: 'transparent',
                  border: 'none',
                  fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)',
                  outline: 'none', cursor: 'pointer',
                }}>
                  <option value="all">All matching connections</option>
                  {(connections || []).filter(c => c.kind === 'xpub' || c.kind === 'descriptor').map(c => (
                    <option key={c.id} value={c.id}>{c.label}</option>
                  ))}
                </select>
              </div>
            </div>
            <div>
              <div style={{
                fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600, letterSpacing: '0.12em',
                textTransform: 'uppercase', color: 'var(--ink-2)', marginBottom: 6,
              }}>On conflict</div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
                {[
                  ['keep',      'Keep existing',  'skip rows where a label is already set'],
                  ['overwrite', 'Overwrite',      'replace existing labels'],
                ].map(([k, label, desc]) => {
                  const active = strategy === k;
                  return (
                    <button key={k} onClick={() => setStrategy(k)} style={{
                      textAlign: 'left',
                      border: '1px solid ' + (active ? 'var(--ink)' : 'var(--line)'),
                      background: active ? 'var(--paper-2)' : 'transparent',
                      padding: '8px 10px',
                      cursor: 'pointer',
                    }} title={desc}>
                      <div style={{ fontFamily: 'var(--sans)', fontSize: 12, fontWeight: 600, color: 'var(--ink)' }}>{label}</div>
                      <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.08em', color: 'var(--ink-3)', marginTop: 2 }}>{desc}</div>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>

          <label style={{
            marginTop: 12, display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer',
            fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink-2)',
          }}>
            <input type="checkbox" checked={onlySpendable} onChange={e => setOnlySpendable(e.target.checked)} style={{ accentColor: 'var(--accent)' }} />
            Only import records marked <span style={{ fontFamily: 'var(--mono)', color: 'var(--ink)' }}>spendable: true</span>
          </label>
        </>
      )}

      <Rule />

      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 10 }}>
        <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.08em' }}>
          {file
            ? <>Will import <b style={{ color: 'var(--ink-2)' }}>{totalValid}</b> · skip <b style={{ color: counts?.unmatched ? 'var(--accent)' : 'var(--ink-2)' }}>{counts?.unmatched || 0}</b></>
            : 'No file selected'}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button
            disabled={!file}
            onClick={() => onImport?.({ file, scope, strategy, onlySpendable, counts })}
            icon={<svg width="12" height="12" viewBox="0 0 12 12"><path d="M3 6 L5 8 L9 4" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>}
          >Import labels</Button>
        </div>
      </div>
    </Modal>
  );
}

const bip329Th = {
  textAlign: 'left', padding: '8px 10px',
  fontFamily: 'var(--mono)', fontSize: 9, fontWeight: 600,
  letterSpacing: '0.12em', textTransform: 'uppercase',
  color: 'var(--ink-3)',
};
const bip329Td = {
  padding: '7px 10px',
  fontFamily: 'var(--mono)', fontSize: 11,
  color: 'var(--ink)',
};
