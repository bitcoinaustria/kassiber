// Full transactions list — searchable, filterable
function TransactionsScreen({ hideSensitive, onBack, onImportLabels, onExportLabels }) {
  const [q, setQ] = React.useState('');
  const [typeFilter, setTypeFilter] = React.useState('all');
  const [moreOpen, setMoreOpen] = React.useState(false);
  const moreRef = React.useRef(null);
  // Primary types = accounting essentials; secondary = technical/edge-case (opened via "More")
  const primaryTypes   = ['all', 'Income', 'Expense', 'Transfer'];
  const secondaryTypes = ['Swap', 'Consolidation', 'Rebalance', 'Mint', 'Melt', 'Fee'];
  const secondaryActive = secondaryTypes.includes(typeFilter);
  React.useEffect(() => {
    if (!moreOpen) return;
    const onDoc = (e) => { if (moreRef.current && !moreRef.current.contains(e.target)) setMoreOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [moreOpen]);

  const typeColor = (t) => ({
    Income: '#3fa66a',
    Expense: 'var(--accent)',
    Transfer: '#6b7280',
    Swap: '#8b6f3c',
    Consolidation: '#5d6b7a',
    Rebalance: '#7d6b8a',
    Mint: '#3f7aa6',
    Melt: '#a66a3f',
    Fee: 'var(--ink-3)',
  }[t] || 'var(--ink-3)');

  const filtered = MOCK.txs.filter(tx => {
    if (typeFilter !== 'all' && tx.type !== typeFilter) return false;
    if (q && !(tx.counter + ' ' + tx.account + ' ' + tx.tag).toLowerCase().includes(q.toLowerCase())) return false;
    return true;
  });

  return (
    <div style={{ flex: 1, overflow: 'auto', background: 'var(--paper)', padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', justifyContent: 'space-between', marginBottom: 18 }}>
        <div>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
            Ledger · {filtered.length} entries · 2026
          </div>
          <h2 style={{ margin: '4px 0 0', fontFamily: 'var(--sans)', fontSize: 32, fontWeight: 600, letterSpacing: '-0.01em', color: 'var(--ink)' }}>
            Transactions
          </h2>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <Button variant="secondary" size="sm" onClick={onImportLabels}>↓ Import labels</Button>
          <Button variant="secondary" size="sm" onClick={onExportLabels}>↑ Export labels</Button>
          <div style={{ width: 1, height: 22, background: 'var(--line)', margin: '0 2px', alignSelf: 'center' }} />
          <Button variant="secondary" size="sm">⤓ CSV</Button>
          <Button variant="secondary" size="sm">⤓ JSON</Button>
          <Button size="sm">+ Manual entry</Button>
        </div>
      </div>

      {/* filter strip */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '10px 12px', border: '1px solid var(--line)', background: 'var(--paper-2)', marginBottom: 10 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 1 }}>
          <svg width="13" height="13" viewBox="0 0 13 13"><circle cx="5.5" cy="5.5" r="3.5" stroke="var(--ink-3)" strokeWidth="1.2" fill="none"/><path d="M8 8 L11 11" stroke="var(--ink-3)" strokeWidth="1.2"/></svg>
          <input value={q} onChange={e => setQ(e.target.value)} placeholder="Search counterparty, tag, account…"
            style={{ flex: 1, border: 'none', background: 'transparent', outline: 'none', fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)' }} />
        </div>
        <div style={{ width: 1, height: 20, background: 'var(--line)' }} />
        <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          {primaryTypes.map(t => (
            <Pill key={t} active={typeFilter === t} onClick={() => setTypeFilter(t)} color={typeFilter === t ? 'ink' : 'muted'}>
              {t}
            </Pill>
          ))}
          <div ref={moreRef} style={{ position: 'relative' }}>
            <Pill
              active={secondaryActive}
              onClick={() => setMoreOpen(o => !o)}
              color={secondaryActive ? 'ink' : 'muted'}
            >
              {secondaryActive ? typeFilter : 'More'} <span style={{ marginLeft: 4, opacity: 0.6 }}>▾</span>
            </Pill>
            {moreOpen && (
              <div style={{
                position: 'absolute', top: 32, right: 0, zIndex: 40,
                background: 'var(--paper)', border: '1px solid var(--ink)',
                boxShadow: '4px 4px 0 var(--ink)',
                padding: 6, minWidth: 160,
                display: 'flex', flexDirection: 'column', gap: 2,
                animation: 'kb-rise 0.14s ease-out',
              }}>
                <div style={{
                  fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)',
                  letterSpacing: '0.12em', textTransform: 'uppercase',
                  padding: '4px 8px 2px',
                }}>Advanced types</div>
                {secondaryTypes.map(t => (
                  <button key={t} onClick={() => { setTypeFilter(t); setMoreOpen(false); }}
                    style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      padding: '5px 8px',
                      background: typeFilter === t ? 'var(--ink)' : 'transparent',
                      color: typeFilter === t ? 'var(--paper)' : 'var(--ink)',
                      border: 'none', cursor: 'pointer',
                      fontFamily: 'var(--sans)', fontSize: 12, textAlign: 'left',
                    }}>
                    {t}
                  </button>
                ))}
                {secondaryActive && (
                  <>
                    <div style={{ height: 1, background: 'var(--line)', margin: '4px 0' }} />
                    <button onClick={() => { setTypeFilter('all'); setMoreOpen(false); }}
                      style={{
                        padding: '5px 8px', background: 'transparent', border: 'none', cursor: 'pointer',
                        fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)',
                        letterSpacing: '0.08em', textTransform: 'uppercase', textAlign: 'left',
                      }}>Clear filter</button>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* table */}
      <div style={{ border: '1px solid var(--line)', background: 'var(--paper-2)' }}>
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr style={{ borderBottom: '1px solid var(--ink)', background: 'var(--paper)' }}>
              <th style={thStyle2}>Date · time</th>
              <th style={thStyle2}>Type</th>
              <th style={thStyle2}>Account</th>
              <th style={thStyle2}>Counterparty</th>
              <th style={thStyle2}>Tag</th>
              <th style={{...thStyle2, textAlign:'right'}}>Sats</th>
              <th style={{...thStyle2, textAlign:'right'}}>BTC/EUR rate</th>
              <th style={{...thStyle2, textAlign:'right'}}>EUR</th>
              <th style={{...thStyle2, textAlign:'right'}}>Conf</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map(tx => (
              <tr key={tx.id} style={{ borderBottom: '1px solid var(--line)' }}>
                <td style={tdStyle2}>{tx.date}</td>
                <td style={tdStyle2}>
                  <span style={{
                    fontSize: 9, letterSpacing: '0.1em', textTransform: 'uppercase',
                    padding: '2px 6px',
                    border: '1px solid',
                    borderColor: typeColor(tx.type),
                    color:       typeColor(tx.type),
                  }}>{tx.type}</span>
                </td>
                <td style={{...tdStyle2, fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)'}}>{tx.account}</td>
                <td style={{...tdStyle2, fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink)'}}>{tx.counter}</td>
                <td style={tdStyle2}>
                  <span style={{
                    fontFamily: 'var(--mono)', fontSize: 10,
                    padding: '2px 7px',
                    background: 'var(--paper)',
                    border: '1px solid var(--line)',
                    color: 'var(--ink-2)', letterSpacing: '0.04em',
                  }}>{tx.tag}</span>
                </td>
                <td style={{...tdStyle2, textAlign:'right', color: tx.amountSat > 0 ? '#3fa66a' : 'var(--ink)'}} className={hideSensitive?'sensitive':''}>
                  {(tx.amountSat > 0 ? '+ ' : '− ') + Math.abs(tx.amountSat).toLocaleString('en-US')}
                </td>
                <td style={{...tdStyle2, textAlign:'right', color: 'var(--ink-3)'}}>€ {tx.rate.toLocaleString('de-AT', {minimumFractionDigits:2, maximumFractionDigits:2})}</td>
                <td style={{...tdStyle2, textAlign:'right'}} className={hideSensitive?'sensitive':''}>
                  {(tx.eur > 0 ? '+ €' : '− €') + Math.abs(tx.eur).toLocaleString('de-AT', {minimumFractionDigits:2, maximumFractionDigits:2})}
                </td>
                <td style={{...tdStyle2, textAlign:'right', color: 'var(--ink-3)'}}>{tx.conf}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

const thStyle2 = {
  textAlign: 'left',
  padding: '10px 14px',
  fontFamily: 'var(--sans)', fontSize: 9, fontWeight: 600,
  letterSpacing: '0.12em', textTransform: 'uppercase',
  color: 'var(--ink-3)',
};
const tdStyle2 = {
  padding: '10px 14px',
  fontFamily: 'var(--mono)', fontSize: 11,
  color: 'var(--ink-2)',
};

window.TransactionsScreen = TransactionsScreen;
