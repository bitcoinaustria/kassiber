/**
 * Overview placeholder.
 *
 * Real translation of claude-design/screens/overview.jsx is the next
 * step in the JSX-import sequence. This stub exists so the Welcome
 * flow has somewhere to navigate after onboarding completes.
 */

import { useUiStore } from "@/store/ui";

export function Overview() {
  const identity = useUiStore((s) => s.identity);
  const setIdentity = useUiStore((s) => s.setIdentity);

  return (
    <div className="mx-auto min-h-screen max-w-3xl p-12 font-sans text-ink">
      <div className="kb-mono-caption mb-2">overview · placeholder</div>
      <h1 className="m-0 text-4xl font-semibold tracking-tight">
        {identity?.workspace ?? "My Books"}
      </h1>
      <p className="mt-2 text-sm text-ink-2">
        Welcome, {identity?.name ?? "—"}. The Overview screen is the next
        translation target.{" "}
        <button
          onClick={() => setIdentity(null)}
          className="cursor-pointer border-none bg-transparent p-0 font-mono text-[11px] uppercase tracking-wider text-accent underline-offset-2 hover:underline"
        >
          Reset onboarding
        </button>
      </p>

      <pre className="mt-8 border border-line bg-paper-2 p-4 font-mono text-xs">
        {JSON.stringify(identity, null, 2)}
      </pre>
    </div>
  );
}
