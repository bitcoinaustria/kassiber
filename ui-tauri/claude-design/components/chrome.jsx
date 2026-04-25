// App chrome: header, footer, nav
function AppHeader({ t, lang, setLang, name, workspace, onOpenSettings, route, setRoute,
  hideSensitive, setHideSensitive, onLock, onOpenProfiles, ccy, setCcy }) {
  const navItems = [
    ['overview', t.nav.overview],
    ['transactions', t.nav.transactions],
    ['reports', t.nav.reports],
  ];
  const [menuOpen, setMenuOpen] = React.useState(false);
  const menuRef = React.useRef(null);
  React.useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e) => { if (menuRef.current && !menuRef.current.contains(e.target)) setMenuOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [menuOpen]);

  return (
    <div style={{
      height: 54, flexShrink: 0,
      padding: '0 18px',
      display: 'flex', alignItems: 'center', gap: 28,
      borderBottom: '1px solid var(--ink)',
      background: 'var(--paper)',
    }}>
      <Wordmark size={20} />
      <div style={{ width: 1, height: 20, background: 'var(--line)' }} />
      <nav style={{ display: 'flex', gap: 4, flex: 1 }}>
        {navItems.map(([k, label]) => (
          <button key={k} onClick={() => setRoute(k)}
            style={{
              background: 'transparent', border: 'none', cursor: 'pointer',
              padding: '6px 12px',
              fontFamily: 'var(--sans)', fontSize: 12, fontWeight: 500,
              letterSpacing: '0.02em',
              color: route === k ? 'var(--ink)' : 'var(--ink-3)',
              position: 'relative',
            }}>
            {label}
            {route === k && <div style={{ position: 'absolute', bottom: -17, left: 0, right: 0, height: 2, background: 'var(--accent)' }} />}
          </button>
        ))}
      </nav>

      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        {/* Workspace — slim. Username lives inside Profiles. */}
        <button
          onClick={onOpenProfiles}
          title={`Workspace · ${workspace} · signed in as ${name}`}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            height: 26, padding: '0 9px',
            border: '1px solid var(--line)',
            background: 'transparent',
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}>
          <span style={{ width: 5, height: 5, background: 'var(--accent)' }} />
          <span style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink)', fontWeight: 500 }}>{workspace}</span>
          <svg width="7" height="7" viewBox="0 0 10 10" style={{ marginLeft: 1 }}><path d="M2 4 L5 7 L8 4" stroke="var(--ink-3)" strokeWidth="1.2" fill="none"/></svg>
        </button>

        {/* Hide-sensitive stays at top level — most-pressed control */}
        <button onClick={() => setHideSensitive && setHideSensitive(!hideSensitive)}
          title={hideSensitive ? 'Show sensitive data' : 'Hide sensitive data'} style={{
          background: hideSensitive ? 'var(--ink)' : 'transparent',
          border: '1px solid ' + (hideSensitive ? 'var(--ink)' : 'var(--line)'),
          width: 26, height: 26, padding: 0, cursor: 'pointer',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          {hideSensitive ? (
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M1.5 7 Q7 2.5 12.5 7 Q11 8.8 9 9.8" stroke="var(--paper)" strokeWidth="1.1" fill="none" strokeLinecap="round"/>
              <path d="M5 9.5 Q4 8.5 4 7 Q4 5.5 5.5 4.8" stroke="var(--paper)" strokeWidth="1.1" fill="none" strokeLinecap="round"/>
              <path d="M2 2 L12 12" stroke="var(--paper)" strokeWidth="1.2" strokeLinecap="round"/>
            </svg>
          ) : (
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M1.5 7 Q7 2 12.5 7 Q7 12 1.5 7 Z" stroke="var(--ink-2)" strokeWidth="1.1" fill="none"/>
              <circle cx="7" cy="7" r="1.7" stroke="var(--ink-2)" strokeWidth="1.1" fill="none"/>
            </svg>
          )}
        </button>

        {/* Overflow menu — currency, language, lock, settings */}
        <div ref={menuRef} style={{ position: 'relative' }}>
          <button onClick={() => setMenuOpen(o => !o)}
            title="More" aria-label="More options"
            style={{
              background: menuOpen ? 'var(--ink)' : 'transparent',
              border: '1px solid ' + (menuOpen ? 'var(--ink)' : 'var(--line)'),
              width: 26, height: 26, padding: 0, cursor: 'pointer',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <circle cx="3.5" cy="7" r="1" fill={menuOpen ? 'var(--paper)' : 'var(--ink-2)'} />
              <circle cx="7"   cy="7" r="1" fill={menuOpen ? 'var(--paper)' : 'var(--ink-2)'} />
              <circle cx="10.5" cy="7" r="1" fill={menuOpen ? 'var(--paper)' : 'var(--ink-2)'} />
            </svg>
          </button>
          {menuOpen && (
            <div style={{
              position: 'absolute', right: 0, top: 32, zIndex: 60,
              width: 230,
              background: 'var(--paper)',
              border: '1px solid var(--ink)',
              boxShadow: '4px 4px 0 var(--ink)',
              padding: 10,
              display: 'flex', flexDirection: 'column', gap: 10,
              animation: 'kb-rise 0.14s ease-out',
            }}>
              {/* Currency */}
              {ccy && setCcy && (
                <MenuRow label="Display">
                  <div style={{ display: 'flex', border: '1px solid var(--line)' }}>
                    {[['btc','₿'], ['eur','€']].map(([k, glyph]) => (
                      <button key={k} onClick={() => setCcy(k)} style={{
                        width: 26, height: 22, padding: 0,
                        background: ccy === k ? 'var(--ink)' : 'transparent',
                        color: ccy === k ? 'var(--paper)' : 'var(--ink-2)',
                        border: 'none', cursor: 'pointer',
                        fontFamily: 'var(--mono)', fontSize: 12, fontWeight: 600,
                      }}>{glyph}</button>
                    ))}
                  </div>
                </MenuRow>
              )}
              {/* Language */}
              <MenuRow label="Language">
                <div style={{ display: 'flex', border: '1px solid var(--line)' }}>
                  {['en', 'de'].map(l => (
                    <button key={l} onClick={() => setLang(l)} style={{
                      width: 26, height: 22, padding: 0,
                      background: lang === l ? 'var(--ink)' : 'transparent',
                      color: lang === l ? 'var(--paper)' : 'var(--ink-2)',
                      border: 'none', cursor: 'pointer',
                      fontFamily: 'var(--mono)', fontSize: 10, fontWeight: 600, letterSpacing: '0.08em',
                    }}>{l.toUpperCase()}</button>
                  ))}
                </div>
              </MenuRow>
              <div style={{ height: 1, background: 'var(--line)', margin: '2px -10px' }} />
              {/* Settings */}
              <button onClick={() => { setMenuOpen(false); onOpenSettings && onOpenSettings(); }}
                style={menuItemStyle}>
                <svg width="13" height="13" viewBox="0 0 14 14" fill="none" style={{ flexShrink: 0 }}>
                  <path d="M7 1.4 L7.7 2.9 L9.3 2.6 L9.4 4.3 L10.9 5 L10 6.4 L10.9 7.8 L9.4 8.5 L9.3 10.2 L7.7 9.9 L7 11.4 L6.3 9.9 L4.7 10.2 L4.6 8.5 L3.1 7.8 L4 6.4 L3.1 5 L4.6 4.3 L4.7 2.6 L6.3 2.9 Z"
                    stroke="var(--ink-2)" strokeWidth="1" strokeLinejoin="round" fill="none"/>
                  <circle cx="7" cy="6.4" r="1.3" stroke="var(--ink-2)" strokeWidth="1" fill="none"/>
                </svg>
                Settings
              </button>
              {/* Lock */}
              <button onClick={() => { setMenuOpen(false); onLock && onLock(); }}
                style={menuItemStyle}>
                <svg width="13" height="13" viewBox="0 0 14 14" fill="none" style={{ flexShrink: 0 }}>
                  <rect x="3" y="6.2" width="8" height="6.3" stroke="var(--ink-2)" strokeWidth="1.1" fill="none"/>
                  <path d="M4.8 6.2 V4.3 Q4.8 2 7 2 Q9.2 2 9.2 4.3 V6.2" stroke="var(--ink-2)" strokeWidth="1.1" fill="none"/>
                  <circle cx="7" cy="9.2" r="0.7" fill="var(--ink-2)"/>
                </svg>
                Lock Kassiber
                <span style={{ marginLeft: 'auto', fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', letterSpacing: '0.08em' }}>⌘L</span>
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function MenuRow({ label, children }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10 }}>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.08em', textTransform: 'uppercase' }}>{label}</span>
      {children}
    </div>
  );
}

const menuItemStyle = {
  display: 'flex', alignItems: 'center', gap: 10,
  width: '100%', padding: '6px 4px',
  background: 'transparent', border: 'none', cursor: 'pointer',
  fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)',
  textAlign: 'left',
};

function AppFooter({ onOpenSettings }) {
  // Price cycles through a few plausible values so the refresh button has something to show.
  const PRICE_SAMPLES = [71420.18, 71452.03, 71398.77, 71510.44, 71488.91, 71463.20];
  const [priceIdx, setPriceIdx] = React.useState(0);
  const [updated, setUpdated] = React.useState(Date.now());
  const [refreshing, setRefreshing] = React.useState(false);

  // Auto-refresh every 60s so it feels live
  React.useEffect(() => {
    const id = setInterval(() => {
      setPriceIdx(i => (i + 1) % PRICE_SAMPLES.length);
      setUpdated(Date.now());
    }, 60000);
    return () => clearInterval(id);
  }, []);

  const refresh = () => {
    setRefreshing(true);
    setTimeout(() => {
      setPriceIdx(i => (i + 1) % PRICE_SAMPLES.length);
      setUpdated(Date.now());
      setRefreshing(false);
    }, 650);
  };

  const price = PRICE_SAMPLES[priceIdx];
  const since = Math.floor((Date.now() - updated) / 1000);
  const sinceLabel = since < 5 ? 'just now'
    : since < 60 ? `${since}s ago`
    : `${Math.floor(since/60)}m ago`;

  return (
    <div style={{
      height: 28, flexShrink: 0,
      padding: '0 18px',
      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
      borderTop: '1px solid var(--line)',
      background: 'var(--paper)',
      fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)',
      letterSpacing: '0.05em',
      position: 'relative',
    }}>
      <div style={{ display: 'flex', gap: 18, alignItems: 'center' }}>
        <span>KASSIBER v0.1.0</span>
        <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
          <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#3fa66a' }} />
          WATCH-ONLY · LOCAL ENCRYPTED VAULT
        </span>
      </div>

      {/* Center donate */}
      <a href="#donate"
        style={{
          position: 'absolute', left: '50%', top: 0, transform: 'translateX(-50%)',
          height: '100%',
          display: 'inline-flex', alignItems: 'center', gap: 6,
          padding: '0 14px',
          fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.1em',
          color: 'var(--accent)',
          background: 'var(--paper-2)',
          borderLeft: '1px solid var(--line)',
          borderRight: '1px solid var(--line)',
          textDecoration: 'none',
          textTransform: 'uppercase',
        }}>
        <svg width="11" height="11" viewBox="0 0 14 14" fill="none" aria-hidden="true">
          <path d="M7 12 L2 7 Q0 5 2 3 Q4 1 7 4 Q10 1 12 3 Q14 5 12 7 Z"
            stroke="var(--accent)" strokeWidth="1.2" fill="var(--accent)" fillOpacity="0.18" strokeLinejoin="round"/>
        </svg>
        DONATE SATS
      </a>

      <div style={{ display: 'flex', gap: 14, alignItems: 'center' }}>
        {/* BTC/EUR rate block */}
        <span
          title={`Updated ${sinceLabel} · source: CoinGecko`}
          style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}
        >
          <span style={{ color: 'var(--ink-3)' }}>BTC/EUR</span>
          <span style={{ color: 'var(--ink)', fontWeight: 600 }}>
            €{price.toLocaleString('de-AT', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
          </span>
          <span style={{ color: 'var(--ink-3)' }}>· COINGECKO</span>
          <button
            onClick={refresh}
            title="Refresh rate"
            aria-label="Refresh BTC/EUR rate"
            style={{
              background: 'transparent', border: '1px solid var(--line)',
              width: 16, height: 16, padding: 0, cursor: 'pointer',
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              marginLeft: 2,
            }}
          >
            <svg width="9" height="9" viewBox="0 0 10 10" fill="none"
              style={{
                animation: refreshing ? 'kb-spin 0.65s linear infinite' : 'none',
              }}>
              <path d="M1.6 5 A3.4 3.4 0 1 1 5 8.4" stroke="var(--ink-2)" strokeWidth="1.1" fill="none" strokeLinecap="round"/>
              <path d="M1.6 5 L1.6 2.3 L4.3 2.3" stroke="var(--ink-2)" strokeWidth="1.1" fill="none" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
          </button>
        </span>
        <span>MAINNET</span>
        <a href="https://github.com/" target="_blank" rel="noreferrer"
          style={{ color: 'var(--ink-3)', textDecoration: 'none', fontFamily: 'inherit', fontSize: 'inherit', letterSpacing: 'inherit', display: 'inline-flex', alignItems: 'center', gap: 5 }}>
          <svg width="11" height="11" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d="M8 0C3.58 0 0 3.58 0 8a8 8 0 005.47 7.59c.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/>
          </svg>
          GITHUB
        </a>
      </div>
    </div>
  );
}

window.AppHeader = AppHeader;
window.AppFooter = AppFooter;
