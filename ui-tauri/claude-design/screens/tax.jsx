// Capital-gains export flow — jurisdiction presets (AT / DE / CH / Other)
// Default: FIFO · EUR

const JURISDICTIONS = {
  AT: { code: 'AT', name: 'Austria',     policy: '§27a EStG · KESt 27,5 %', rate: 0.275, rateLabel: 'KESt 27,5 %', defaultMethod: 'fifo', internalsNonTaxable: true,  longTermDays: 365, ccy: '€', locale: 'de-AT' },
  DE: { code: 'DE', name: 'Germany',     policy: '§ 23 EStG · private sales',  rate: 0.26375, rateLabel: 'Est. 26,375 %',  defaultMethod: 'fifo', internalsNonTaxable: true,  longTermDays: 365, ccy: '€', locale: 'de-DE' },
  CH: { code: 'CH', name: 'Switzerland', policy: 'Private wealth · tax-exempt',   rate: 0.00,    rateLabel: 'Private · 0 %',       defaultMethod: 'fifo', internalsNonTaxable: true,  longTermDays: 0,   ccy: 'CHF', locale: 'de-CH' },
  XX: { code: 'XX', name: 'Generic',     policy: 'Generic capital-gains preset',    rate: 0.20,    rateLabel: 'Est. 20 %',            defaultMethod: 'fifo', internalsNonTaxable: true,  longTermDays: 365, ccy: '€', locale: 'en-GB' },
};

function TaxReportScreen({ hideSensitive, onBack }) {
  const [year, setYear] = React.useState(2025);
  const [jur, setJur] = React.useState('AT');
  const j = JURISDICTIONS[jur];
  const [method, setMethod] = React.useState(j.defaultMethod);
  const [step, setStep] = React.useState(1); // 1 configure  2 preview

  const fmt = (n) => n.toLocaleString(j.locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const ccy = j.ccy;

  const lots = [
    { acquired: '2022-03-18', disposed: '2025-11-04', sats: 12_000_000, costEur:  3_851.20, proceedsEur:  8_204.18, type: 'LT' },
    { acquired: '2023-07-02', disposed: '2025-11-04', sats:  8_000_000, costEur:  2_412.00, proceedsEur:  5_469.45, type: 'LT' },
    { acquired: '2024-11-14', disposed: '2025-12-01', sats:  3_500_000, costEur:  2_188.70, proceedsEur:  2_392.08, type: 'ST' },
    { acquired: '2025-02-09', disposed: '2025-12-20', sats:  1_800_000, costEur:  1_011.55, proceedsEur:  1_290.12, type: 'ST' },
    { acquired: '2025-04-22', disposed: '2026-01-08', sats:    900_000, costEur:    614.90, proceedsEur:    635.14, type: 'ST' },
  ];
  const totals = lots.reduce((a, l) => ({
    sats: a.sats + l.sats,
    cost: a.cost + l.costEur,
    proceeds: a.proceeds + l.proceedsEur,
    gain: a.gain + (l.proceedsEur - l.costEur),
  }), { sats: 0, cost: 0, proceeds: 0, gain: 0 });
  const kest = totals.gain * j.rate;

  return (
    <div style={{ flex: 1, overflow: 'auto', background: 'var(--paper)', padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', justifyContent: 'space-between', marginBottom: 18 }}>
        <div>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
            Report · Capital gains · {j.name}
          </div>
          <h2 style={{ margin: '4px 0 0', fontFamily: 'var(--sans)', fontSize: 32, fontWeight: 600, letterSpacing: '-0.01em', color: 'var(--ink)' }}>
            Capital gains
          </h2>
        </div>
        <div style={{ display: 'flex', gap: 18, alignItems: 'center' }}>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.08em' }}>
            STEP {step} / 2
          </div>
          <Button variant="ghost" size="sm" onClick={onBack}>← Back</Button>
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '340px 1fr', gap: 14 }}>
        {/* Left: config */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <Card title="Jurisdiction">
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {Object.values(JURISDICTIONS).map(x => (
                  <Pill key={x.code} active={jur === x.code} onClick={() => { setJur(x.code); setMethod(x.defaultMethod); }} color={jur === x.code ? 'ink' : 'muted'}>
                    {x.code}
                  </Pill>
                ))}
              </div>
              <div style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)' }}>
                {j.name} · {j.policy}
              </div>
            </div>
          </Card>

          <Card title="Reporting period">
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {[2023, 2024, 2025, 2026].map(y => (
                  <Pill key={y} active={year === y} onClick={() => setYear(y)} color={year === y ? 'ink' : 'muted'}>
                    {y}
                  </Pill>
                ))}
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                <DateInput label="From" value={`${year}-01-01`} onChange={()=>{}} />
                <DateInput label="To" value={`${year}-12-31`} onChange={()=>{}} />
              </div>
            </div>
          </Card>

          <Card title="Cost-basis method">
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {[
                ['fifo', 'FIFO', 'First-in, first-out · common default'],
                ['lifo', 'LIFO', 'Last-in, first-out'],
                ['hifo', 'HIFO', 'Highest-in, first-out (tax optimization)'],
                ['spec', 'Specific ID', 'Per-lot selection'],
              ].map(([k, name, desc]) => (
                <label key={k} style={{
                  display: 'flex', gap: 10, padding: 10,
                  border: '1px solid var(--line)',
                  background: method === k ? 'var(--paper)' : 'transparent',
                  cursor: 'pointer',
                }}>
                  <input type="radio" name="method" checked={method === k} onChange={() => setMethod(k)} style={{ accentColor: 'var(--accent)', marginTop: 2 }} />
                  <div>
                    <div style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--ink)', fontWeight: 600 }}>{name}</div>
                    <div style={{ fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)', marginTop: 2 }}>{desc}</div>
                  </div>
                </label>
              ))}
            </div>
          </Card>

          <Card title="Policy">
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <CheckRow key={`internal-${jur}`} label="Treat internal transfers as non-taxable" def={j.internalsNonTaxable} />
              <CheckRow key={`rate-${jur}`} label={`Apply ${j.rateLabel} flat rate`} def={j.rate > 0} />
              <CheckRow label="Include Lightning channel fees as cost" def />
              <CheckRow label="Aggregate lots per UTXO set" />
            </div>
          </Card>

          <Button size="lg" onClick={() => setStep(2)}
            icon={<svg width="12" height="12" viewBox="0 0 12 12"><path d="M2 6 H10 M6 2 L10 6 L6 10" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>}
          >
            Generate preview
          </Button>
        </div>

        {/* Right: preview */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
            <StatTile label="Proceeds" value={<span className={hideSensitive?'sensitive':''}>{ccy} {fmt(totals.proceeds)}</span>} sub={`${lots.length} disposals`} />
            <StatTile label="Cost basis" value={<span className={hideSensitive?'sensitive':''}>{ccy} {fmt(totals.cost)}</span>} sub={method.toUpperCase()} />
            <StatTile label="Net gain" value={<span className={hideSensitive?'sensitive':''} style={{color:'#3fa66a'}}>+ {ccy} {fmt(totals.gain)}</span>} sub={`${year} tax year`} />
            <StatTile label={j.rateLabel} value={<span className={hideSensitive?'sensitive':''} style={{color:'var(--accent)'}}>{ccy} {fmt(kest)}</span>} sub="Estimated liability" />
          </div>

          <Card title={`Disposed lots · ${year}`} pad={false}>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead><tr style={{ borderBottom: '1px solid var(--ink)' }}>
                <th style={thStyle2}>Acquired</th>
                <th style={thStyle2}>Disposed</th>
                <th style={thStyle2}>Holding</th>
                <th style={{...thStyle2,textAlign:'right'}}>Sats</th>
                <th style={{...thStyle2,textAlign:'right'}}>Cost {ccy}</th>
                <th style={{...thStyle2,textAlign:'right'}}>Proceeds {ccy}</th>
                <th style={{...thStyle2,textAlign:'right'}}>Gain {ccy}</th>
              </tr></thead>
              <tbody>
                {lots.map((l, i) => {
                  const gain = l.proceedsEur - l.costEur;
                  return (
                    <tr key={i} style={{ borderBottom: '1px solid var(--line)' }}>
                      <td style={tdStyle2}>{l.acquired}</td>
                      <td style={tdStyle2}>{l.disposed}</td>
                      <td style={tdStyle2}>
                        <span style={{
                          fontSize: 9, letterSpacing: '0.1em', padding: '2px 6px',
                          border: `1px solid ${l.type === 'LT' ? '#3fa66a' : 'var(--ink-3)'}`,
                          color: l.type === 'LT' ? '#3fa66a' : 'var(--ink-2)',
                        }}>{l.type === 'LT' ? '> 1Y' : '< 1Y'}</span>
                      </td>
                      <td style={{...tdStyle2, textAlign:'right'}} className={hideSensitive?'sensitive':''}>{l.sats.toLocaleString('en-US')}</td>
                      <td style={{...tdStyle2, textAlign:'right'}} className={hideSensitive?'sensitive':''}>{l.costEur.toFixed(2)}</td>
                      <td style={{...tdStyle2, textAlign:'right'}} className={hideSensitive?'sensitive':''}>{l.proceedsEur.toFixed(2)}</td>
                      <td style={{...tdStyle2, textAlign:'right', color: '#3fa66a'}} className={hideSensitive?'sensitive':''}>+ {gain.toFixed(2)}</td>
                    </tr>
                  );
                })}
                <tr style={{ background: 'var(--paper)' }}>
                  <td style={{...tdStyle2, fontWeight: 600}} colSpan={3}>Total</td>
                  <td style={{...tdStyle2, textAlign:'right', fontWeight: 600}} className={hideSensitive?'sensitive':''}>{totals.sats.toLocaleString('en-US')}</td>
                  <td style={{...tdStyle2, textAlign:'right', fontWeight: 600}} className={hideSensitive?'sensitive':''}>{totals.cost.toFixed(2)}</td>
                  <td style={{...tdStyle2, textAlign:'right', fontWeight: 600}} className={hideSensitive?'sensitive':''}>{totals.proceeds.toFixed(2)}</td>
                  <td style={{...tdStyle2, textAlign:'right', fontWeight: 600, color: '#3fa66a'}} className={hideSensitive?'sensitive':''}>+ {totals.gain.toFixed(2)}</td>
                </tr>
              </tbody>
            </table>
          </Card>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 10 }}>
            <ExportFormat name="CSV" sub="Spreadsheet" detail="17 columns · UTF-8" />
            <ExportFormat name="PDF" sub="Human-readable" detail={`4 pages · ${j.name} format`} primary />
            <ExportFormat name="JSON" sub="Envelope" detail="Machine-readable" />
          </div>
        </div>
      </div>
    </div>
  );
}

function CheckRow({ label, def }) {
  const [on, setOn] = React.useState(!!def);
  return (
    <label style={{ display: 'flex', alignItems: 'center', gap: 10, cursor: 'pointer' }}>
      <div style={{
        width: 30, height: 16, background: on ? 'var(--ink)' : 'var(--line-2)',
        position: 'relative', transition: 'background 0.15s',
      }}>
        <div style={{
          position: 'absolute', top: 2, left: on ? 16 : 2,
          width: 12, height: 12, background: 'var(--paper-2)',
          transition: 'left 0.15s',
        }}/>
      </div>
      <span style={{ fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink-2)' }}>{label}</span>
    </label>
  );
}

function ExportFormat({ name, sub, detail, primary }) {
  return (
    <button style={{
      background: primary ? 'var(--ink)' : 'var(--paper-2)',
      color: primary ? 'var(--paper)' : 'var(--ink)',
      border: `1px solid ${primary ? 'var(--ink)' : 'var(--line)'}`,
      padding: 16,
      textAlign: 'left',
      cursor: 'pointer',
      display: 'flex', flexDirection: 'column', gap: 2,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontFamily: 'var(--sans)', fontSize: 22 }}>{name}</span>
        <span style={{ fontFamily: 'var(--mono)', fontSize: 14 }}>⤓</span>
      </div>
      <span style={{ fontFamily: 'var(--sans)', fontSize: 11, opacity: 0.7 }}>{sub}</span>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 10, opacity: 0.55, marginTop: 6 }}>{detail}</span>
    </button>
  );
}

window.TaxReportScreen = TaxReportScreen;
