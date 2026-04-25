// Shared UI primitives for Kassiber
// Designed to map 1:1 onto Qt/QML widgets: solid fills, single shadow, simple borders

function Button({ children, variant = 'primary', size = 'md', onClick, disabled, icon, style = {} }) {
  const sizes = {
    sm: { h: 28, px: 12, fs: 12 },
    md: { h: 36, px: 16, fs: 13 },
    lg: { h: 44, px: 20, fs: 14 },
  }[size];
  const variants = {
    primary: { bg: 'var(--accent)', fg: '#fff6ef', border: 'var(--accent)' },
    secondary: { bg: 'transparent', fg: 'var(--ink)', border: 'var(--ink)' },
    ghost: { bg: 'transparent', fg: 'var(--ink-2)', border: 'transparent' },
    danger: { bg: 'transparent', fg: 'var(--accent)', border: 'var(--accent)' },
  }[variant];
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      style={{
        height: sizes.h,
        padding: `0 ${sizes.px}px`,
        fontSize: sizes.fs,
        fontFamily: 'var(--sans)',
        fontWeight: 500,
        letterSpacing: '0.01em',
        background: variants.bg,
        color: variants.fg,
        border: `1px solid ${variants.border}`,
        borderRadius: 2,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.45 : 1,
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: 8,
        transition: 'opacity 0.15s, background 0.15s',
        whiteSpace: 'nowrap',
        ...style,
      }}
    >
      {icon}
      {children}
    </button>
  );
}

function Input({ label, value, onChange, placeholder, mono, type = 'text', style = {}, rightAdornment }) {
  const id = React.useId();
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, ...style }}>
      {label && (
        <label htmlFor={id} style={{
          fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600,
          letterSpacing: '0.12em', textTransform: 'uppercase',
          color: 'var(--ink-2)',
        }}>{label}</label>
      )}
      <div style={{
        display: 'flex', alignItems: 'center',
        border: '1px solid var(--line)',
        background: 'var(--paper-2)',
        height: 36,
      }}>
        <input
          id={id}
          type={type}
          value={value ?? ''}
          onChange={onChange}
          placeholder={placeholder}
          style={{
            flex: 1,
            height: '100%',
            padding: '0 10px',
            border: 'none',
            background: 'transparent',
            outline: 'none',
            fontFamily: mono ? 'var(--mono)' : 'var(--sans)',
            fontSize: mono ? 12 : 13,
            color: 'var(--ink)',
          }}
        />
        {rightAdornment && (
          <div style={{ padding: '0 10px', fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-3)' }}>
            {rightAdornment}
          </div>
        )}
      </div>
    </div>
  );
}

// Parse/format an ISO yyyy-mm-dd (local, no timezone drift).
function parseISODate(s) {
  if (!s || !/^\d{4}-\d{2}-\d{2}/.test(s)) return null;
  const [y, m, d] = s.slice(0, 10).split('-').map(Number);
  return new Date(y, m - 1, d);
}
function fmtISODate(d) {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${dd}`;
}

// DateInput: clickable field that opens a monochrome calendar flyout.
function DateInput({ label, value, onChange, placeholder = 'yyyy-mm-dd', style = {} }) {
  const id = React.useId();
  const [open, setOpen] = React.useState(false);
  const rootRef = React.useRef(null);
  const initial = parseISODate(value) || new Date();
  const [viewY, setViewY] = React.useState(initial.getFullYear());
  const [viewM, setViewM] = React.useState(initial.getMonth()); // 0-11

  React.useEffect(() => {
    if (!open) return;
    const onDoc = (e) => { if (rootRef.current && !rootRef.current.contains(e.target)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);

  React.useEffect(() => {
    const d = parseISODate(value);
    if (d) { setViewY(d.getFullYear()); setViewM(d.getMonth()); }
  }, [value]);

  const selected = parseISODate(value);
  const today = new Date();
  const monthName = new Date(viewY, viewM, 1).toLocaleString('en-US', { month: 'long' });

  // Build 6×7 day grid starting on Monday
  const first = new Date(viewY, viewM, 1);
  const startDow = (first.getDay() + 6) % 7; // Mon=0
  const daysInMonth = new Date(viewY, viewM + 1, 0).getDate();
  const cells = [];
  for (let i = 0; i < 42; i++) {
    const dayNum = i - startDow + 1;
    const d = new Date(viewY, viewM, dayNum);
    cells.push(d);
  }
  const shift = (delta) => {
    const nm = viewM + delta;
    const ny = viewY + Math.floor(nm / 12);
    const nmo = ((nm % 12) + 12) % 12;
    setViewY(ny); setViewM(nmo);
  };
  const pick = (d) => {
    onChange && onChange({ target: { value: fmtISODate(d) } });
    setOpen(false);
  };
  const sameYmd = (a, b) => a && b && a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

  return (
    <div ref={rootRef} style={{ display: 'flex', flexDirection: 'column', gap: 6, position: 'relative', ...style }}>
      {label && (
        <label htmlFor={id} style={{
          fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600,
          letterSpacing: '0.12em', textTransform: 'uppercase',
          color: 'var(--ink-2)',
        }}>{label}</label>
      )}
      <button
        id={id}
        type="button"
        onClick={() => setOpen(o => !o)}
        style={{
          display: 'flex', alignItems: 'center', width: '100%',
          border: '1px solid ' + (open ? 'var(--ink)' : 'var(--line)'),
          background: 'var(--paper-2)',
          height: 36,
          padding: '0 10px',
          fontFamily: 'var(--mono)', fontSize: 12, color: value ? 'var(--ink)' : 'var(--ink-3)',
          cursor: 'pointer', textAlign: 'left',
          justifyContent: 'space-between', gap: 8,
        }}>
        <span>{value || placeholder}</span>
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" style={{ flexShrink: 0 }}>
          <rect x="1.5" y="2.5" width="9" height="8" stroke="var(--ink-2)" strokeWidth="1" fill="none"/>
          <line x1="1.5" y1="5" x2="10.5" y2="5" stroke="var(--ink-2)" strokeWidth="1"/>
          <line x1="4" y1="1.5" x2="4" y2="3.5" stroke="var(--ink-2)" strokeWidth="1"/>
          <line x1="8" y1="1.5" x2="8" y2="3.5" stroke="var(--ink-2)" strokeWidth="1"/>
        </svg>
      </button>
      {open && (
        <div style={{
          position: 'absolute', top: '100%', left: 0, marginTop: 4,
          zIndex: 50,
          background: 'var(--paper)',
          border: '1px solid var(--ink)',
          boxShadow: '4px 4px 0 var(--ink)',
          padding: 10,
          width: 252,
          fontFamily: 'var(--sans)',
        }}>
          {/* header */}
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
            <button type="button" onClick={() => shift(-1)} style={calBtnStyle}>‹</button>
            <div style={{ display: 'flex', gap: 6, alignItems: 'baseline' }}>
              <span style={{ fontFamily: 'var(--sans)', fontSize: 13, fontWeight: 600, color: 'var(--ink)' }}>{monthName}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-3)' }}>{viewY}</span>
            </div>
            <button type="button" onClick={() => shift(1)} style={calBtnStyle}>›</button>
          </div>
          {/* dow row */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 1fr)', gap: 1 }}>
            {['M','T','W','T','F','S','S'].map((d, i) => (
              <div key={i} style={{
                textAlign: 'center', fontFamily: 'var(--mono)', fontSize: 9,
                letterSpacing: '0.08em', color: 'var(--ink-3)', padding: '4px 0',
              }}>{d}</div>
            ))}
          </div>
          {/* days grid */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 1fr)', gap: 1 }}>
            {cells.map((d, i) => {
              const inMonth = d.getMonth() === viewM;
              const isSel = sameYmd(d, selected);
              const isToday = sameYmd(d, today);
              return (
                <button
                  key={i}
                  type="button"
                  onClick={() => pick(d)}
                  style={{
                    height: 28,
                    border: 'none',
                    background: isSel ? 'var(--ink)' : 'transparent',
                    color: isSel ? 'var(--paper)' : (inMonth ? 'var(--ink)' : 'var(--ink-3)'),
                    fontFamily: 'var(--mono)', fontSize: 11,
                    cursor: 'pointer',
                    outline: isToday && !isSel ? '1px solid var(--accent)' : 'none',
                    outlineOffset: -2,
                    fontWeight: isToday ? 700 : 400,
                  }}
                  onMouseEnter={(e) => { if (!isSel) e.currentTarget.style.background = 'var(--line)'; }}
                  onMouseLeave={(e) => { if (!isSel) e.currentTarget.style.background = 'transparent'; }}
                >{d.getDate()}</button>
              );
            })}
          </div>
          {/* footer */}
          <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 8, paddingTop: 8, borderTop: '1px solid var(--line)' }}>
            <button type="button" onClick={() => pick(new Date())} style={calLinkStyle}>Today</button>
            <button type="button" onClick={() => { onChange && onChange({ target: { value: '' } }); setOpen(false); }} style={calLinkStyle}>Clear</button>
          </div>
        </div>
      )}
    </div>
  );
}
const calBtnStyle = {
  background: 'transparent', border: '1px solid var(--line)',
  width: 22, height: 22, padding: 0, cursor: 'pointer',
  fontFamily: 'var(--sans)', fontSize: 13, color: 'var(--ink-2)',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
};
const calLinkStyle = {
  background: 'transparent', border: 'none', cursor: 'pointer',
  fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-2)',
  letterSpacing: '0.1em', textTransform: 'uppercase',
  padding: '2px 4px',
};

function Card({ title, action, children, style = {}, pad = true, resizable = true }) {
  return (
    <div style={{
      background: 'var(--paper-2)',
      border: '1px solid var(--line)',
      display: 'flex', flexDirection: 'column',
      minHeight: 0,
      ...style,
    }}>
      {title && (
        <div style={{
          height: 36,
          padding: '0 14px',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          borderBottom: '1px solid var(--line)',
          flexShrink: 0,
        }}>
          <span style={{
            fontFamily: 'var(--sans)', fontSize: 13, fontWeight: 600,
            color: 'var(--ink)', letterSpacing: '0.005em',
          }}>{title}</span>
          {action}
        </div>
      )}
      <div style={{ flex: 1, padding: pad ? 14 : 0, minHeight: 0, overflow: 'auto' }}>
        {children}
      </div>
    </div>
  );
}

function Pill({ children, active, onClick, color = 'ink' }) {
  const colors = {
    ink: { border: 'var(--ink)', fg: 'var(--ink)' },
    accent: { border: 'var(--accent)', fg: 'var(--accent)' },
    muted: { border: 'var(--line-2)', fg: 'var(--ink-2)' },
  }[color];
  return (
    <button
      onClick={onClick}
      style={{
        height: 26, padding: '0 12px',
        fontFamily: 'var(--mono)', fontSize: 11,
        letterSpacing: '0.02em',
        background: active ? colors.fg : 'transparent',
        color: active ? 'var(--paper)' : colors.fg,
        border: `1px solid ${colors.border}`,
        borderRadius: 999,
        cursor: onClick ? 'pointer' : 'default',
        whiteSpace: 'nowrap',
        transition: 'all 0.15s',
      }}
    >
      {children}
    </button>
  );
}

function Rule({ vertical, style = {} }) {
  return (
    <div style={{
      background: 'var(--line)',
      ...(vertical ? { width: 1, alignSelf: 'stretch' } : { height: 1, width: '100%' }),
      ...style,
    }} />
  );
}

// Vertical dotted/hairline key-value pairs
function KV({ k, v, mono = true, blur, copy }) {
  const [copied, setCopied] = React.useState(false);
  const onCopy = async (e) => {
    e.stopPropagation();
    const text = typeof v === 'string' ? v : (e.currentTarget.parentElement?.querySelector('[data-kv-val]')?.textContent || '');
    try { await navigator.clipboard.writeText(text); } catch {}
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
        <span data-kv-val className={blur ? 'sensitive' : ''} style={{
          flex: 1, minWidth: 0,
          fontFamily: mono ? 'var(--mono)' : 'var(--sans)',
          fontSize: mono ? 13 : 14,
          color: 'var(--ink)',
          letterSpacing: mono ? '-0.01em' : 0,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>{v}</span>
        {copy && (
          <button onClick={onCopy} title={copied ? 'Copied' : 'Copy'} style={{
            flexShrink: 0,
            background: 'transparent', border: '1px solid var(--line)',
            width: 20, height: 20, padding: 0, cursor: 'pointer',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
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
        )}
      </div>
    </div>
  );
}

// Modal shell
function Modal({ open, onClose, children, width = 560, title, back }) {
  if (!open) return null;
  return (
    <div
      onClick={onClose}
      style={{
        position: 'absolute', inset: 0,
        background: 'rgba(26,22,19,0.28)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        zIndex: 50,
        animation: 'kb-fade 0.15s ease-out',
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width,
          maxHeight: '88%',
          background: 'var(--paper)',
          border: '1px solid var(--ink)',
          boxShadow: '0 20px 60px -10px rgba(40,20,20,0.3)',
          display: 'flex', flexDirection: 'column',
          animation: 'kb-rise 0.2s ease-out',
        }}
      >
        <div style={{
          height: 44,
          padding: '0 14px',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          borderBottom: '1px solid var(--line)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            {back && (
              <button onClick={back} style={{
                background: 'transparent', border: 'none', cursor: 'pointer',
                color: 'var(--ink-2)', padding: 4, display: 'flex', alignItems: 'center',
              }}>
                <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                  <path d="M9 2 L4 7 L9 12" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round" strokeLinejoin="round"/>
                </svg>
              </button>
            )}
            <span style={{ fontFamily: 'var(--sans)', fontSize: 17, fontWeight: 600 }}>{title}</span>
          </div>
          <button onClick={onClose} style={{
            background: 'transparent', border: 'none', cursor: 'pointer',
            color: 'var(--ink-2)', padding: 4, display: 'flex', alignItems: 'center',
          }}>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M2 2 L12 12 M12 2 L2 12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
            </svg>
          </button>
        </div>
        <div style={{ padding: 20, overflow: 'auto' }}>
          {children}
        </div>
      </div>
    </div>
  );
}

// Connection protocol icon (simple geometric shapes per protocol)
function ProtocolIcon({ kind, size = 28 }) {
  const s = size;
  switch (kind) {
    case 'xpub':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <circle cx="14" cy="14" r="12" fill="var(--ink)"/>
          <path d="M10 8 L10 20 M10 8 L15 8 Q17.5 8 17.5 11 Q17.5 14 15 14 L10 14 M13 14 L17.5 20" stroke="var(--paper)" strokeWidth="1.4" fill="none" strokeLinecap="round"/>
        </svg>
      );
    case 'descriptor':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <rect x="2" y="2" width="24" height="24" fill="var(--ink)"/>
          <path d="M7 9 H21 M7 14 H17 M7 19 H14" stroke="var(--paper)" strokeWidth="1.4"/>
        </svg>
      );
    case 'core-ln':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <circle cx="14" cy="14" r="12" fill="var(--ink)"/>
          <path d="M15 6 L9 16 L13 16 L11 22 L19 11 L15 11 Z" fill="var(--accent)"/>
        </svg>
      );
    case 'lnd':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <rect x="2" y="2" width="24" height="24" fill="var(--ink)"/>
          <text x="14" y="19" textAnchor="middle" fontFamily="var(--serif)" fontSize="13" fontWeight="700" fill="var(--paper)">lnd</text>
        </svg>
      );
    case 'nwc':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <rect x="2" y="2" width="24" height="24" fill="var(--ink)" transform="rotate(45 14 14) scale(0.72)" transform-origin="14 14"/>
          <rect x="6" y="6" width="16" height="16" fill="var(--ink)"/>
          <path d="M9 14 L13 10 L13 14 L19 14 L15 18 L15 14 Z" fill="var(--accent)"/>
        </svg>
      );
    case 'cashu':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <circle cx="14" cy="14" r="12" fill="var(--ink)"/>
          <circle cx="14" cy="14" r="6" fill="none" stroke="var(--accent)" strokeWidth="1.4"/>
          <circle cx="14" cy="14" r="2" fill="var(--accent)"/>
        </svg>
      );
    case 'csv':
      return (
        <svg width={s} height={s} viewBox="0 0 28 28">
          <rect x="2" y="2" width="24" height="24" fill="none" stroke="var(--ink)" strokeWidth="1.2"/>
          <path d="M7 9 H21 M7 14 H21 M7 19 H21 M11 6 V22 M17 6 V22" stroke="var(--ink)" strokeWidth="1.2"/>
        </svg>
      );
    default:
      return <div style={{ width: s, height: s, background: 'var(--ink)' }} />;
  }
}

Object.assign(window, { Button, Input, DateInput, Card, Pill, Rule, KV, Modal, ProtocolIcon });
