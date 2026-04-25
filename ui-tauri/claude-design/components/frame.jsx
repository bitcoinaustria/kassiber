// Minimal macOS window chrome for Kassiber
// Parchment body, ink-black title bar with traffic lights, tight radius

function TrafficLights() {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
      <span style={{ width: 12, height: 12, borderRadius: '50%', background: '#ff5f57', border: '0.5px solid rgba(0,0,0,0.15)' }} />
      <span style={{ width: 12, height: 12, borderRadius: '50%', background: '#febc2e', border: '0.5px solid rgba(0,0,0,0.15)' }} />
      <span style={{ width: 12, height: 12, borderRadius: '50%', background: '#28c840', border: '0.5px solid rgba(0,0,0,0.15)' }} />
    </div>
  );
}

function MacFrame({ children, title = 'Kassiber', width = 1280, height = 820 }) {
  return (
    <div style={{
      width, height,
      borderRadius: 10,
      overflow: 'hidden',
      background: 'var(--paper)',
      boxShadow: '0 0 0 0.5px rgba(0,0,0,0.22), 0 28px 80px -12px rgba(40,20,20,0.35), 0 10px 30px -8px rgba(0,0,0,0.18)',
      display: 'flex', flexDirection: 'column',
      fontFamily: 'var(--sans)',
    }}>
      {/* Title bar */}
      <div style={{
        height: 32,
        background: '#222222',
        display: 'flex', alignItems: 'center',
        padding: '0 14px',
        flexShrink: 0,
        position: 'relative',
        borderBottom: '0.5px solid rgba(0,0,0,0.4)',
      }}>
        <TrafficLights />
        <div style={{
          position: 'absolute', left: 0, right: 0, textAlign: 'center',
          fontFamily: 'var(--sans)', fontSize: 12, fontWeight: 500,
          color: 'rgba(255,255,255,0.55)',
          letterSpacing: '0.02em',
          pointerEvents: 'none',
        }}>{title}</div>
      </div>

      {/* Body */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0, position: 'relative' }}>
        {children}
      </div>
    </div>
  );
}

window.MacFrame = MacFrame;
window.TrafficLights = TrafficLights;
