// Kassiber wordmark + logomark
// Logomark: Bitcoin ₿ glyph with horizontal stripe treatment.
// onDark=true swaps the stripe fill to white for use on dark backgrounds.

function LogoMark({ size = 28, onDark = false }) {
  // Use the logomark SVG files directly, via <img>.
  // "Dark" variant = for light backgrounds (₿ is dark). "Light" variant = for dark backgrounds.
  const src = onDark ? 'assets/logomark-light.svg' : 'assets/logomark-dark.svg';
  const h = size * (484.5 / 590);
  return (
    <img src={src} width={size} height={h}
      alt="Kassiber"
      style={{ display: 'block', width: size, height: h }} />
  );
}

function Wordmark({ size = 22, color = 'currentColor' }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, color }}>
      <span style={{
        fontFamily: 'var(--sans)',
        fontSize: size,
        fontWeight: 600,
        letterSpacing: '-0.01em',
        color: 'var(--ink)',
        lineHeight: 1,
      }}>Kassiber</span>
    </div>
  );
}

// Large lockup for the welcome screen
function SealLockup({ size = 220 }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 18 }}>
      <div style={{
        fontFamily: 'var(--sans)',
        fontSize: size * 0.34,
        fontWeight: 600,
        letterSpacing: '-0.02em',
        color: 'var(--ink)',
        lineHeight: 1,
      }}>Kassiber</div>
    </div>
  );
}

Object.assign(window, { LogoMark, Wordmark, SealLockup });
