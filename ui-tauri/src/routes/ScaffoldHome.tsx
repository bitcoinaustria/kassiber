/**
 * Scaffold landing page.
 *
 * Placeholder until claude-design screens are translated. Verifies the
 * full stack end-to-end: Tailwind theme tokens, fonts, mock daemon
 * transport, TanStack Query, Zustand store.
 */

import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

interface StatusEnvelope {
  version: string;
  data_root: string;
  workspace: string | null;
  profile: string | null;
}

export function ScaffoldHome() {
  const status = useDaemon<StatusEnvelope>("status");
  const { hideSensitive, setHideSensitive } = useUiStore();

  return (
    <div className="min-h-screen p-12 max-w-3xl mx-auto font-sans text-ink">
      <header className="mb-10 pb-6 border-b border-line">
        <div className="kb-mono-caption mb-2">scaffold · phase 1.2 partial</div>
        <h1 className="text-4xl font-semibold tracking-tight m-0">
          Kassiber UI
        </h1>
        <p className="text-ink-2 mt-2 text-sm">
          Vite + React 19 + TypeScript + Tailwind v4 + TanStack Query/Router
          + Zustand. shadcn primitives install per-screen via{" "}
          <code className="font-mono text-xs">pnpm dlx shadcn@latest add</code>.
          Translation of <code className="font-mono text-xs">claude-design/</code>{" "}
          screens lands next.
        </p>
      </header>

      <section className="mb-8">
        <div className="kb-mono-caption mb-2">mock daemon · status</div>
        {status.isLoading && <div className="font-mono text-xs">loading…</div>}
        {status.data && (
          <pre
            className={`font-mono text-xs bg-paper-2 border border-line p-4 overflow-x-auto ${hideSensitive ? "sensitive" : ""}`}
          >
            {JSON.stringify(status.data, null, 2)}
          </pre>
        )}
      </section>

      <section className="mb-8">
        <div className="kb-mono-caption mb-2">privacy</div>
        <label className="flex items-center gap-3 text-sm">
          <input
            type="checkbox"
            checked={hideSensitive}
            onChange={(e) => setHideSensitive(e.target.checked)}
            className="accent-accent"
          />
          Hide sensitive data (blurs balances &amp; amounts)
        </label>
      </section>

      <section>
        <div className="kb-mono-caption mb-2">theme tokens</div>
        <div className="flex gap-2">
          {(
            [
              "bg-paper",
              "bg-paper-2",
              "bg-ink",
              "bg-ink-2",
              "bg-ink-3",
              "bg-line",
              "bg-line-2",
              "bg-accent",
              "bg-accent-2",
            ] as const
          ).map((cls) => (
            <div
              key={cls}
              className={`w-12 h-12 border border-line ${cls}`}
              title={cls}
            />
          ))}
        </div>
      </section>
    </div>
  );
}
