/**
 * Connections placeholder.
 *
 * Real translation of claude-design/screens/connections.jsx (list +
 * detail + add-connection picker + XPub form) lands as the next
 * substantial step. The route is defined now so deep-links from the
 * Overview connections card can resolve.
 */

export function Connections() {
  return (
    <div className="flex-1 p-12 font-sans text-ink">
      <div className="kb-mono-caption mb-2">connections · placeholder</div>
      <h1 className="m-0 text-3xl font-semibold tracking-tight">Connections</h1>
      <p className="mt-2 text-sm text-ink-2">
        XPub, descriptor, Lightning, and exchange connection management lands
        here.
      </p>
    </div>
  );
}
