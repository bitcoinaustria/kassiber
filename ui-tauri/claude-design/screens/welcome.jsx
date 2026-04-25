// Welcome flow — three-step sequence:
//  1. WelcomeIntro    → editorial/manifesto; single CTA
//  2. WelcomeSetup    → name + workspace + tax residency
//  3. WelcomeEncrypt  → encrypt-at-rest decision (optional)
// Each step shares the masthead + step indicator so it feels like one document.

const WELCOME_STEPS = [
  { id: 'intro',   label: 'Intro',     n: '—' },
  { id: 'setup',   label: 'Identity',  n: '01' },
  { id: 'encrypt', label: 'Encryption',n: '02' },
];

function WelcomeShell({ step, children, rightSlot }) {
  return (
    <div style={{
      flex: 1, position: 'relative',
      display: 'flex', flexDirection: 'column',
      background: 'var(--paper)',
      overflow: 'hidden',
    }}>
      {/* Promise bar — the single load-bearing fact */}
      <div style={{
        borderBottom: '1px solid var(--ink)',
        padding: '9px 28px',
        display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10,
        fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.18em',
        textTransform: 'uppercase', color: 'var(--ink-2)',
        background: 'var(--paper-2)',
      }}>
        <span style={{
          width: 6, height: 6, borderRadius: '50%',
          background: 'var(--accent)',
          boxShadow: '0 0 0 3px rgba(166,47,47,0.12)',
        }} />
        <span>Watch-only</span>
        <span style={{ color: 'var(--ink-3)' }}>·</span>
        <span>This app never touches your private keys.</span>
      </div>

      {/* Step indicator */}
      {step !== 'intro' && (
        <div style={{
          borderBottom: '1px solid var(--line)',
          padding: '10px 28px',
          display: 'flex', alignItems: 'center', gap: 18,
          fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.14em',
          textTransform: 'uppercase',
        }}>
          {WELCOME_STEPS.filter(s => s.id !== 'intro').map((s, i, arr) => {
            const active = s.id === step;
            const done = arr.findIndex(x => x.id === step) > i;
            return (
              <React.Fragment key={s.id}>
                <span style={{
                  display: 'flex', alignItems: 'center', gap: 8,
                  color: active ? 'var(--ink)' : (done ? 'var(--ink-2)' : 'var(--ink-3)'),
                }}>
                  <span style={{
                    width: 18, height: 18, border: '1px solid',
                    borderColor: active ? 'var(--accent)' : (done ? 'var(--ink-2)' : 'var(--ink-3)'),
                    color: active ? 'var(--accent)' : 'inherit',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    fontSize: 9, fontWeight: 700,
                  }}>{done ? '✓' : s.n}</span>
                  <span>{s.label}</span>
                </span>
                {i < arr.length - 1 && <span style={{ color: 'var(--ink-3)' }}>·</span>}
              </React.Fragment>
            );
          })}
          <span style={{ marginLeft: 'auto', color: 'var(--ink-3)' }}>Setup</span>
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        {children}
      </div>
    </div>
  );
}

// ——— Step 1 — Intro / Manifesto ——————————————————————————————————————

function WelcomeIntro({ onNext }) {
  const facts = [
    ['Local',         'Plain files on your disk. Export anytime.'],
    ['Watch-only',    'xpubs & read-keys. Never private keys.'],
    ['Jurisdictions', 'Presets for AT · DE · CH · more coming.'],
    ['Encrypted',     'Optional at-rest passphrase.'],
  ];
  return (
    <WelcomeShell step="intro">
      <div style={{
        flex: 1,
        display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        padding: '40px 48px',
      }}>
        <div style={{
          width: '100%', maxWidth: 520,
          display: 'flex', flexDirection: 'column',
        }}>
          {/* Wordmark */}
          <Wordmark size={22} />

          {/* Heading */}
          <h1 style={{
            marginTop: 28, marginBottom: 0,
            fontFamily: 'var(--sans)', fontSize: 40, fontWeight: 600,
            lineHeight: 1.1, letterSpacing: '-0.02em',
            color: 'var(--ink)',
          }}>
            Your books. Your keys.
          </h1>

          {/* Subline */}
          <p style={{
            marginTop: 14, marginBottom: 0,
            fontFamily: 'var(--sans)', fontSize: 14, lineHeight: 1.55,
            color: 'var(--ink-2)',
            maxWidth: 440,
          }}>
            A Bitcoin-only ledger that runs locally. No cloud, no custodian, no account.
          </p>

          {/* Fact list — compact, mono, aligned to the app's density */}
          <dl style={{
            margin: '28px 0 0', padding: 0,
            borderTop: '1px solid var(--line)',
          }}>
            {facts.map(([k, v]) => (
              <div key={k} style={{
                display: 'grid', gridTemplateColumns: '140px 1fr',
                padding: '9px 0',
                borderBottom: '1px solid var(--line)',
                alignItems: 'baseline',
              }}>
                <dt style={{
                  fontFamily: 'var(--mono)', fontSize: 10,
                  letterSpacing: '0.14em', textTransform: 'uppercase',
                  color: 'var(--ink-3)',
                }}>{k}</dt>
                <dd style={{
                  margin: 0,
                  fontFamily: 'var(--sans)', fontSize: 13,
                  color: 'var(--ink-2)',
                }}>{v}</dd>
              </div>
            ))}
          </dl>

          {/* CTA row */}
          <div style={{
            marginTop: 28,
            display: 'flex', alignItems: 'center', gap: 16,
          }}>
            <Button size="lg" onClick={onNext}
              icon={<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M2 7 H11 M7 3 L11 7 L7 11" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>}
            >Continue</Button>
            <span style={{
              fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.1em',
              textTransform: 'uppercase', color: 'var(--ink-3)',
            }}>Two-minute setup</span>
          </div>
        </div>
      </div>
    </WelcomeShell>
  );
}

// ——— Step 2 — Setup ——————————————————————————————————————————————————

function WelcomeSetup({ initial, onBack, onNext }) {
  const [name, setName] = React.useState(initial?.name || '');
  const [workspace, setWorkspace] = React.useState(initial?.workspace || '');
  const [country, setCountry] = React.useState(initial?.country || 'AT');
  const submit = () => {
    if (!name.trim()) return;
    onNext({ name: name.trim(), workspace: workspace.trim() || 'My Books', country });
  };
  return (
    <WelcomeShell step="setup">
      <div style={{
        flex: 1, display: 'grid', gridTemplateColumns: '1fr 1fr',
      }}>
        {/* Left: context panel */}
        <div style={{
          padding: '48px 56px',
          borderRight: '1px solid var(--line)',
          background: 'var(--paper-2)',
          display: 'flex', flexDirection: 'column', justifyContent: 'space-between',
          position: 'relative', overflow: 'hidden',
        }}>
          <div>
            <div style={{
              fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.2em',
              textTransform: 'uppercase', color: 'var(--accent)',
              marginBottom: 14,
            }}>Step 01 of 02</div>
            <h2 style={{
              fontFamily: 'var(--sans)', fontSize: 64, fontWeight: 600,
              margin: 0, lineHeight: 0.95, letterSpacing: '-0.025em',
              color: 'var(--ink)',
            }}>Tell us<br/>who's writing.</h2>
            <p style={{
              marginTop: 20, maxWidth: 380,
              fontFamily: 'var(--sans)', fontSize: 14, lineHeight: 1.55,
              color: 'var(--ink-2)',
            }}>
              Your name and workspace live only on this device. The workspace becomes a folder of plain files — you can rename, move, or delete it at any time.
            </p>
          </div>
          <div style={{
            marginTop: 32,
            fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.12em',
            color: 'var(--ink-3)', lineHeight: 1.7,
          }}>
            <div>// Stored at</div>
            <div style={{ color: 'var(--ink-2)' }}>~/.kassiber/{(workspace || 'my-books').toLowerCase().replace(/\s+/g,'-')}/</div>
          </div>
        </div>

        {/* Right: form */}
        <div style={{
          padding: '48px 56px',
          display: 'flex', flexDirection: 'column', gap: 24,
        }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
            <Input label="Your name" value={name} onChange={e => setName(e.target.value)} placeholder="e.g. Alice" />
            <Input label="Workspace name" value={workspace} onChange={e => setWorkspace(e.target.value)} placeholder="My Books" />

            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <span style={{
                fontFamily: 'var(--sans)', fontSize: 10, fontWeight: 600,
                letterSpacing: '0.12em', textTransform: 'uppercase',
                color: 'var(--ink-2)',
              }}>Tax residency</span>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {['AT', 'DE', 'CH', 'IT', 'NL', 'Other'].map(c => {
                  const active = country === c;
                  return (
                    <button
                      key={c} type="button"
                      onClick={() => setCountry(c)}
                      style={{
                        background: active ? 'var(--ink)' : 'transparent',
                        color: active ? 'var(--paper)' : 'var(--ink)',
                        border: '1px solid ' + (active ? 'var(--ink)' : 'var(--line)'),
                        padding: '6px 14px',
                        fontFamily: 'var(--mono)', fontSize: 11,
                        letterSpacing: '0.08em', textTransform: 'uppercase',
                        fontWeight: 600,
                        cursor: 'pointer',
                      }}
                    >{c}</button>
                  );
                })}
              </div>
              <span style={{
                fontFamily: 'var(--sans)', fontSize: 11, color: 'var(--ink-3)',
                fontStyle: 'italic',
              }}>Jurisdiction presets load sensible defaults. You can customize everything later.</span>
            </div>
          </div>

          <div style={{
            marginTop: 'auto',
            borderTop: '1px solid var(--ink)',
            paddingTop: 20,
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          }}>
            <button onClick={onBack} style={{
              background: 'transparent', border: 'none', cursor: 'pointer',
              fontFamily: 'var(--mono)', fontSize: 11, letterSpacing: '0.12em',
              textTransform: 'uppercase', color: 'var(--ink-2)',
              padding: '4px 0',
            }}>← Back</button>
            <Button size="lg" onClick={submit} disabled={!name.trim()}
              icon={<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M2 7 H11 M7 3 L11 7 L7 11" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>}
            >Continue</Button>
          </div>
        </div>
      </div>
    </WelcomeShell>
  );
}

// ——— Step 3 — Encryption —————————————————————————————————————————————

function WelcomeEncrypt({ initial, onBack, onFinish }) {
  const [mode, setMode] = React.useState(initial?.mode || 'encrypt'); // 'encrypt' | 'plain'
  const [passphrase, setPassphrase] = React.useState(initial?.passphrase || '');
  const [confirm, setConfirm] = React.useState(initial?.confirm || '');
  const [showPw, setShowPw] = React.useState(false);

  const pwStrength = scorePassphrase(passphrase);
  const pwMatch = passphrase.length === 0 || passphrase === confirm;
  const canFinish = mode === 'plain'
    || (passphrase.length >= 12 && pwMatch);

  const finish = () => {
    if (!canFinish) return;
    onFinish({ encrypted: mode === 'encrypt', passphraseSet: mode === 'encrypt' && passphrase.length > 0 });
  };

  return (
    <WelcomeShell step="encrypt">
      <div style={{
        flex: 1, display: 'grid', gridTemplateColumns: '1fr 1fr',
      }}>
        {/* Left: context */}
        <div style={{
          padding: '48px 56px',
          borderRight: '1px solid var(--line)',
          background: 'var(--paper-2)',
          display: 'flex', flexDirection: 'column', justifyContent: 'space-between',
          position: 'relative', overflow: 'hidden',
        }}>
          <div>
            <div style={{
              fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.2em',
              textTransform: 'uppercase', color: 'var(--accent)',
              marginBottom: 14,
            }}>Step 02 of 02</div>
            <h2 style={{
              fontFamily: 'var(--sans)', fontSize: 64, fontWeight: 600,
              margin: 0, lineHeight: 0.95, letterSpacing: '-0.025em',
              color: 'var(--ink)',
            }}>Lock the<br/><span style={{ fontStyle: 'italic' }}>door.</span></h2>
            <p style={{
              marginTop: 20, maxWidth: 380,
              fontFamily: 'var(--sans)', fontSize: 14, lineHeight: 1.55,
              color: 'var(--ink-2)',
            }}>
              Kassiber can encrypt your database file at rest with a passphrase only you know. Anyone with your disk would see opaque ciphertext — not balances, not addresses, not tags.
            </p>
            <div style={{
              marginTop: 20, padding: 14,
              background: 'var(--paper)', border: '1px solid var(--line)',
              fontFamily: 'var(--sans)', fontSize: 12, lineHeight: 1.55,
              color: 'var(--ink-2)',
            }}>
              <div style={{ display: 'flex', gap: 8, alignItems: 'flex-start' }}>
                <span style={{ color: 'var(--accent)', fontWeight: 700 }}>⚠</span>
                <div>
                  <b style={{ color: 'var(--ink)' }}>We can't recover it.</b> Kassiber never sees your passphrase. If you lose it, the encrypted workspace is unreadable — including by us. Write it down.
                </div>
              </div>
            </div>
          </div>
          <div style={{
            marginTop: 24,
            fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.1em',
            color: 'var(--ink-3)', lineHeight: 1.8,
          }}>
            <div>// Cipher</div>
            <div style={{ color: 'var(--ink-2)' }}>AES-256-GCM</div>
            <div style={{ marginTop: 6 }}>// Key derivation</div>
            <div style={{ color: 'var(--ink-2)' }}>Argon2id · 256 MB · 3 passes</div>
          </div>
        </div>

        {/* Right: choice + passphrase */}
        <div style={{
          padding: '48px 56px',
          display: 'flex', flexDirection: 'column', gap: 20,
        }}>
          {/* Two choice cards */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
            <ChoiceCard
              active={mode === 'encrypt'}
              onClick={() => setMode('encrypt')}
              n="A" title="Encrypt"
              tagline="Recommended"
              desc="Passphrase required to open the workspace. Data at rest is unreadable without it."
            />
            <ChoiceCard
              active={mode === 'plain'}
              onClick={() => setMode('plain')}
              n="B" title="Plain"
              tagline="Insecure · not recommended"
              warning
              desc="Debug / evaluation only. Your database is written in the clear — anyone with disk access can read every balance, address and tag."
            />
          </div>

          {/* Passphrase reveal */}
          {mode === 'encrypt' && (
            <div style={{
              border: '1px solid var(--ink)',
              padding: '18px 20px',
              display: 'flex', flexDirection: 'column', gap: 14,
              background: 'var(--paper-2)',
            }}>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <Input
                  label="Passphrase"
                  value={passphrase}
                  onChange={e => setPassphrase(e.target.value)}
                  placeholder="at least 12 characters"
                  type={showPw ? 'text' : 'password'}
                  mono
                />
                <Input
                  label="Confirm passphrase"
                  value={confirm}
                  onChange={e => setConfirm(e.target.value)}
                  placeholder="repeat"
                  type={showPw ? 'text' : 'password'}
                  mono
                />
              </div>

              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{
                  flex: 1, height: 4, background: 'var(--line)',
                  display: 'flex', gap: 1,
                }}>
                  {[0,1,2,3].map(i => (
                    <div key={i} style={{
                      flex: 1,
                      background: i < pwStrength.level
                        ? (pwStrength.level >= 3 ? '#3fa66a'
                          : pwStrength.level === 2 ? 'var(--ink)'
                          : 'var(--accent)')
                        : 'transparent',
                    }} />
                  ))}
                </div>
                <span style={{
                  fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.14em',
                  textTransform: 'uppercase',
                  color: passphrase ? 'var(--ink-2)' : 'var(--ink-3)',
                  minWidth: 72, textAlign: 'right',
                }}>{passphrase ? pwStrength.label : '— none —'}</span>
                <button type="button" onClick={() => setShowPw(s => !s)} style={{
                  background: 'transparent', border: '1px solid var(--line)',
                  padding: '4px 10px', cursor: 'pointer',
                  fontFamily: 'var(--mono)', fontSize: 10,
                  letterSpacing: '0.1em', textTransform: 'uppercase',
                  color: 'var(--ink-2)',
                }}>{showPw ? 'Hide' : 'Show'}</button>
              </div>

              {!pwMatch && (
                <div style={{
                  fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.08em',
                  textTransform: 'uppercase', color: 'var(--accent)',
                }}>Passphrases don't match.</div>
              )}
              {passphrase && passphrase.length < 12 && (
                <div style={{
                  fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.08em',
                  textTransform: 'uppercase', color: 'var(--ink-3)',
                }}>At least 12 characters required · {passphrase.length}/12</div>
              )}
            </div>
          )}

          {mode === 'plain' && (
            <div style={{
              border: '1px solid var(--accent)',
              borderLeft: '4px solid var(--accent)',
              padding: '14px 18px',
              fontFamily: 'var(--sans)', fontSize: 12, lineHeight: 1.55,
              color: 'var(--ink-2)',
              background: 'rgba(166, 47, 47, 0.05)',
              display: 'flex', gap: 10, alignItems: 'flex-start',
            }}>
              <span style={{
                fontFamily: 'var(--mono)', fontSize: 11, fontWeight: 700,
                color: 'var(--accent)', letterSpacing: '0.1em',
                textTransform: 'uppercase', flexShrink: 0,
              }}>⚠ Insecure</span>
              <div>
                <b style={{ color: 'var(--ink)' }}>Do not use this for real books.</b> Plain mode is intended for debugging and early evaluation only — your database is readable by anything with disk access. Switch to encrypted before tracking real funds via <b>Settings → App lock</b>.
              </div>
            </div>
          )}

          <div style={{
            marginTop: 'auto',
            borderTop: '1px solid var(--ink)',
            paddingTop: 20,
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          }}>
            <button onClick={onBack} style={{
              background: 'transparent', border: 'none', cursor: 'pointer',
              fontFamily: 'var(--mono)', fontSize: 11, letterSpacing: '0.12em',
              textTransform: 'uppercase', color: 'var(--ink-2)',
              padding: '4px 0',
            }}>← Back</button>
            <Button size="lg" onClick={finish} disabled={!canFinish}
              icon={<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M2 7 H11 M7 3 L11 7 L7 11" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>}
            >Open ledger</Button>
          </div>
        </div>
      </div>
    </WelcomeShell>
  );
}

function ChoiceCard({ active, onClick, n, title, tagline, desc, warning }) {
  return (
    <button onClick={onClick} style={{
      textAlign: 'left',
      border: '1px solid ' + (active ? (warning ? 'var(--accent)' : 'var(--ink)') : 'var(--line)'),
      background: active ? 'var(--paper-2)' : 'transparent',
      padding: '16px 16px 18px',
      cursor: 'pointer',
      position: 'relative',
      display: 'flex', flexDirection: 'column', gap: 8,
      outline: 'none',
      boxShadow: active ? '4px 4px 0 ' + (warning ? 'var(--accent)' : 'var(--ink)') : 'none',
      transition: 'box-shadow 0.12s',
    }}>
      {warning && (
        <div aria-hidden="true" style={{
          position: 'absolute', top: 0, right: 0,
          background: 'var(--accent)', color: 'var(--paper)',
          fontFamily: 'var(--mono)', fontSize: 9, fontWeight: 700,
          letterSpacing: '0.16em', textTransform: 'uppercase',
          padding: '3px 8px',
        }}>⚠ Insecure</div>
      )}
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
        <span style={{
          fontFamily: 'var(--mono)', fontSize: 11, letterSpacing: '0.12em',
          color: active ? 'var(--accent)' : 'var(--ink-3)', fontWeight: 700,
        }}>{n}</span>
        <span style={{
          fontFamily: 'var(--sans)', fontSize: 20, fontWeight: 600,
          letterSpacing: '-0.01em', color: 'var(--ink)',
        }}>{title}</span>
        {active && !warning && (
          <span style={{
            marginLeft: 'auto',
            fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em',
            textTransform: 'uppercase', color: 'var(--accent)', fontWeight: 700,
          }}>● selected</span>
        )}
      </div>
      <div style={{
        fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em',
        textTransform: 'uppercase',
        color: warning ? 'var(--accent)' : 'var(--ink-3)',
        fontWeight: warning ? 700 : 400,
      }}>{tagline}</div>
      <div style={{
        fontFamily: 'var(--sans)', fontSize: 12.5, lineHeight: 1.5,
        color: 'var(--ink-2)',
      }}>{desc}</div>
    </button>
  );
}

function scorePassphrase(pw) {
  if (!pw) return { level: 0, label: '— none —' };
  if (pw.length < 12) return { level: 0, label: 'Too short' };
  let score = 1;
  if (pw.length >= 16) score++;
  if (pw.length >= 20) score++;
  if (/[a-z]/.test(pw) && /[A-Z]/.test(pw)) score++;
  if (/\d/.test(pw) && /[^A-Za-z0-9]/.test(pw)) score++;
  const label = ['Too short', 'Weak', 'OK', 'Strong', 'Excellent'][Math.min(score, 4)];
  return { level: Math.min(score, 4), label };
}

// ——— Orchestrator — drives the 3 steps and submits aggregated state ——————

function WelcomeScreen({ t, onEnter }) {
  const [step, setStep] = React.useState('intro'); // intro | setup | encrypt
  const [identity, setIdentity] = React.useState(null);

  if (step === 'intro') {
    return <WelcomeIntro onNext={() => setStep('setup')} />;
  }
  if (step === 'setup') {
    return (
      <WelcomeSetup
        initial={identity}
        onBack={() => setStep('intro')}
        onNext={(id) => { setIdentity(id); setStep('encrypt'); }}
      />
    );
  }
  // encrypt
  return (
    <WelcomeEncrypt
      initial={null}
      onBack={() => setStep('setup')}
      onFinish={(enc) => onEnter({ ...identity, ...enc })}
    />
  );
}

window.WelcomeScreen = WelcomeScreen;
