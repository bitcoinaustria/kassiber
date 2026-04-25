// Profiles / workspaces switcher screen
// Lists workspaces → profiles with tax policy + account/wallet counts.
// Also exports a compact ProfileSwitcherPopover for the header crumb.

function ProfilesScreen({ user, onPick, onBack, onNewWorkspace, onNewProfile }) {
  const workspaces = MOCK.workspaces;
  const [activeId, setActiveId] = React.useState(user?.profileId || 'p1');

  return (
    <div style={{ flex: 1, overflow: 'auto', background: 'var(--paper)', padding: 22 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'flex-end', justifyContent: 'space-between', marginBottom: 22 }}>
        <div>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
            Identity · {workspaces.length} workspaces · {workspaces.reduce((a, w) => a + w.profiles.length, 0)} profiles
          </div>
          <h2 style={{ margin: '4px 0 0', fontFamily: 'var(--sans)', fontSize: 32, fontWeight: 600, letterSpacing: '-0.01em', color: 'var(--ink)' }}>
            Switch profile
          </h2>
          <div style={{ marginTop: 6, fontFamily: 'var(--sans)', fontSize: 12.5, color: 'var(--ink-2)', maxWidth: 640, lineHeight: 1.5 }}>
            Each profile keeps its own books, tax policy, accounts and wallets.
            Nothing is shared across profiles — switching reloads the ledger in read-only mode.
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <Button variant="ghost" size="sm" onClick={onBack}>← Back</Button>
          <Button variant="secondary" size="sm" onClick={onNewProfile}>+ Profile</Button>
          <Button size="sm" onClick={onNewWorkspace}>+ Workspace</Button>
        </div>
      </div>

      {/* Workspaces */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
        {workspaces.map(ws => (
          <div key={ws.id}>
            {/* Workspace header */}
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, marginBottom: 10, paddingBottom: 6, borderBottom: '1px solid var(--ink)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <svg width="12" height="12" viewBox="0 0 12 12"><rect x="1" y="1" width="10" height="10" stroke="var(--ink)" fill="none" strokeWidth="1"/></svg>
                <span style={{ fontFamily: 'var(--sans)', fontSize: 19, color: 'var(--ink)', letterSpacing: '-0.005em' }}>{ws.name}</span>
              </div>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.08em', textTransform: 'uppercase' }}>
                {ws.kind} · {ws.currency} · {ws.jurisdiction} · since {ws.created}
              </span>
              <span style={{ flex: 1 }} />
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>
                {ws.profiles.length} profile{ws.profiles.length === 1 ? '' : 's'}
              </span>
            </div>

            {/* Profile grid */}
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 10 }}>
              {ws.profiles.map(p => {
                const isActive = p.id === activeId;
                return (
                  <button key={p.id} onClick={() => { setActiveId(p.id); onPick && onPick({ workspace: ws, profile: p }); }}
                    style={{
                      textAlign: 'left', cursor: 'pointer',
                      border: '1px solid ' + (isActive ? 'var(--ink)' : 'var(--line)'),
                      background: isActive ? 'var(--paper-2)' : 'var(--paper)',
                      padding: '14px 16px',
                      display: 'flex', flexDirection: 'column', gap: 10,
                      position: 'relative',
                      fontFamily: 'var(--sans)',
                    }}>
                    {isActive && (
                      <span style={{
                        position: 'absolute', top: 10, right: 12,
                        fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em',
                        color: 'var(--accent)', textTransform: 'uppercase',
                        display: 'flex', alignItems: 'center', gap: 5,
                      }}>
                        <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--accent)' }} />
                        Active
                      </span>
                    )}

                    {/* name + role */}
                    <div>
                      <div style={{ fontFamily: 'var(--sans)', fontSize: 17, color: 'var(--ink)', letterSpacing: '-0.005em' }}>{p.name}</div>
                      <div style={{ marginTop: 3, fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', letterSpacing: '0.08em', textTransform: 'uppercase' }}>
                        {p.role} · opened {p.lastOpened}
                      </div>
                    </div>

                    {/* tax policy */}
                    <div style={{
                      borderLeft: '2px solid var(--accent)', paddingLeft: 10,
                      fontFamily: 'var(--sans)', fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.4,
                    }}>
                      <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.12em', color: 'var(--ink-3)', textTransform: 'uppercase', marginBottom: 2 }}>
                        Tax policy
                      </div>
                      {p.taxPolicy}
                    </div>

                    {/* counts */}
                    <div style={{ display: 'flex', gap: 18, fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink)' }}>
                      <span>
                        <span style={{ color: 'var(--ink-3)', fontSize: 9, letterSpacing: '0.12em', textTransform: 'uppercase', display: 'block', marginBottom: 2 }}>Accounts</span>
                        {p.accounts}
                      </span>
                      <span>
                        <span style={{ color: 'var(--ink-3)', fontSize: 9, letterSpacing: '0.12em', textTransform: 'uppercase', display: 'block', marginBottom: 2 }}>Wallets</span>
                        {p.wallets}
                      </span>
                      <span style={{ flex: 1 }} />
                      <span style={{ alignSelf: 'flex-end', color: isActive ? 'var(--accent)' : 'var(--ink-3)', letterSpacing: '0.08em', textTransform: 'uppercase', fontSize: 10 }}>
                        {isActive ? 'Current →' : 'Open →'}
                      </span>
                    </div>
                  </button>
                );
              })}

              {/* New profile tile */}
              <button onClick={onNewProfile} style={{
                cursor: 'pointer',
                border: '1px dashed var(--line-2, #c9bfad)',
                background: 'transparent',
                padding: '14px 16px',
                minHeight: 150,
                display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 6,
                fontFamily: 'var(--sans)',
                color: 'var(--ink-3)',
              }}>
                <div style={{ fontSize: 22, fontFamily: 'var(--sans)', color: 'var(--ink-2)' }}>+</div>
                <div style={{ fontSize: 12, color: 'var(--ink-2)' }}>New profile in {ws.name}</div>
                <div style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.12em', textTransform: 'uppercase' }}>
                  Inherit tax defaults
                </div>
              </button>
            </div>
          </div>
        ))}

        {/* New workspace */}
        <button onClick={onNewWorkspace} style={{
          cursor: 'pointer',
          border: '1px dashed var(--line-2, #c9bfad)',
          background: 'transparent',
          padding: '18px 20px',
          display: 'flex', alignItems: 'center', gap: 14,
          fontFamily: 'var(--sans)',
        }}>
          <div style={{ fontSize: 26, fontFamily: 'var(--sans)', color: 'var(--ink-2)', lineHeight: 1 }}>+</div>
          <div style={{ textAlign: 'left' }}>
            <div style={{ fontFamily: 'var(--sans)', fontSize: 16, color: 'var(--ink)' }}>New workspace</div>
            <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>Separate books · separate tax policy · separate backups</div>
          </div>
        </button>
      </div>
    </div>
  );
}

// Compact popover fired by clicking the header crumb
function ProfileSwitcherPopover({ open, onClose, user, onPick, onManage }) {
  if (!open) return null;
  const workspaces = MOCK.workspaces;
  return (
    <>
      <div onClick={onClose} style={{ position: 'absolute', inset: 0, zIndex: 60 }} />
      <div style={{
        position: 'absolute', top: 46, right: 18, zIndex: 61,
        width: 340,
        background: 'var(--paper)',
        border: '1px solid var(--ink)',
        boxShadow: '0 20px 60px -20px rgba(0,0,0,0.35)',
        fontFamily: 'var(--sans)',
      }}>
        <div style={{ padding: '10px 14px 8px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'baseline', justifyContent: 'space-between' }}>
          <span style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>Switch profile</span>
          <button onClick={onManage} style={{ background: 'transparent', border: 'none', padding: 0, cursor: 'pointer', fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.14em', textTransform: 'uppercase', color: 'var(--accent)' }}>
            Manage →
          </button>
        </div>
        <div style={{ maxHeight: 380, overflow: 'auto' }}>
          {workspaces.map(ws => (
            <div key={ws.id}>
              <div style={{ padding: '10px 14px 4px', fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.12em', color: 'var(--ink-3)', textTransform: 'uppercase' }}>
                {ws.name} · {ws.kind}
              </div>
              {ws.profiles.map(p => (
                <button key={p.id} onClick={() => onPick({ workspace: ws, profile: p })} style={{
                  width: '100%', textAlign: 'left',
                  background: p.active ? 'var(--paper-2)' : 'transparent',
                  border: 'none', borderTop: '1px solid var(--line)',
                  padding: '8px 14px',
                  cursor: 'pointer',
                  display: 'flex', alignItems: 'center', gap: 10,
                }}>
                  <span style={{
                    width: 8, height: 8, borderRadius: '50%',
                    background: p.active ? 'var(--accent)' : 'transparent',
                    border: '1px solid ' + (p.active ? 'var(--accent)' : 'var(--line)'),
                    flexShrink: 0,
                  }} />
                  <span style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontFamily: 'var(--sans)', fontSize: 14, color: 'var(--ink)' }}>{p.name}</div>
                    <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>
                      {p.accounts} acct · {p.wallets} wallet · {p.taxPolicy.split('·')[0].trim()}
                    </div>
                  </span>
                  <span style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', letterSpacing: '0.08em' }}>{p.lastOpened}</span>
                </button>
              ))}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}

window.ProfilesScreen = ProfilesScreen;
window.ProfileSwitcherPopover = ProfileSwitcherPopover;
