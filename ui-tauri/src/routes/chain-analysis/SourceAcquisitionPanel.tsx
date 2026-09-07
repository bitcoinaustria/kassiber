import { useContext, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { DaemonScopeContext, useDaemon, useDaemonMutation } from "@/daemon/client";

type Recipe = {
  backend: string; network: string; mode: "subject" | "blocks"; subject?: string;
  start_height?: number; end_height?: number; interval_seconds: number;
  duration_days: number; max_requests: number; max_bytes: number;
};
type SourcePlan = { spec: Recipe; plan_id: string; effects: { requires_running_unlocked_daemon: true } };
type Source = { id: string; spec: Recipe; status: string; revision: number; cursor_height: number | null;
  requests_used: number; bytes_used: number; expires_at: number; last_code: string | null };

/** Advanced acquisition stays inside the existing source drawer. */
export function SourceAcquisitionPanel({ subject = "", onError }: { subject?: string; onError: (error: unknown) => void }) {
  const { t } = useTranslation("chainAnalysis");
  const scope = useContext(DaemonScopeContext);
  const [open, setOpen] = useState(false);
  const liveScope = useRef(scope);
  liveScope.current = scope;
  const options = useDaemon<{ backends: Array<{ name: string; kind: string; network?: string }> }>("ui.backends.options");
  const environment = useDaemon<{ state: "bound" | "unbound"; environment?: string; domains: Array<{ chain: string; network: string; chain_instance_id?: string }> }>("ui.networks.binding");
  const bitcoin = environment.data?.data?.domains.find(domain => domain.chain === "bitcoin");
  const sources = useDaemon<{ items: Source[] }>("ui.chain_analysis.sources.list", { limit: 100 }, { enabled: open, staleTime: 0, refetchInterval: open ? 5000 : false });
  const [backend, setBackend] = useState("");
  const [mode, setMode] = useState<"subject" | "blocks">("subject");
  const [txid, setTxid] = useState(subject.match(/[a-f0-9]{64}/)?.[0] || "");
  const [start, setStart] = useState("0");
  const [end, setEnd] = useState("");
  const [interval, setInterval] = useState("300");
  const [days, setDays] = useState("30");
  const [requests, setRequests] = useState("10000");
  const [megabytes, setMegabytes] = useState("1024");
  const [preview, setPreview] = useState<{ input: string; plan: SourcePlan; scope: typeof scope } | null>(null);
  const plan = useDaemonMutation<SourcePlan>("ui.chain_analysis.sources.plan", { invalidateQueries: false });
  const authorize = useDaemonMutation<Source>("ui.chain_analysis.sources.authorize");
  const revoke = useDaemonMutation<Source>("ui.chain_analysis.sources.revoke");
  const enqueue = useDaemonMutation<Source>("ui.chain_analysis.sources.run");
  const backends = (options.data?.data?.backends || []).filter(item => item.kind === "bitcoinrpc" && item.network === bitcoin?.network);
  const selected = backends.find(item => item.name === backend);
  const spec = { backend, network: selected?.network, mode, ...(bitcoin?.chain_instance_id ? { chain_instance_id: bitcoin.chain_instance_id } : {}),
    ...(mode === "subject" ? { subject: txid.trim() } : { start_height: Number(start), ...(end.trim() ? { end_height: Number(end) } : {}) }),
    interval_seconds: Number(interval), duration_days: Number(days), max_requests: Number(requests), max_bytes: Number(megabytes) * 1024 * 1024 };
  const input = JSON.stringify(spec);
  const busy = plan.isPending || authorize.isPending || revoke.isPending || enqueue.isPending;
  const ready = preview && preview.input === input && preview.scope === scope;
  const act = async (operation: () => Promise<unknown>) => {
    try { await operation(); await sources.refetch(); } catch (error) { onError(error); }
  };
  return <details className="border-t px-4 py-3" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className="cursor-pointer text-sm font-medium">{t("sources.title")}</summary>
    <div className="mt-4 space-y-4">
      <p className="text-xs text-muted-foreground">{t("sources.lifecycle")}</p>
      {(options.error || environment.error || sources.error) && <div role="alert" className="space-y-2 text-sm">
        <p>{t("sources.loadError")}</p>
        <Button variant="outline" size="sm" onClick={() => void act(async () => { await Promise.all([options.refetch(), environment.refetch()]); })}>{t("sources.retry")}</Button>
      </div>}
      {environment.isPending ? <p role="status" className="text-sm text-muted-foreground">{t("sources.loading")}</p>
        : environment.data?.data?.state === "unbound" ? <Link className="text-sm underline underline-offset-4" to="/settings/bitcoin">{t("sources.bindBook")}</Link>
        : bitcoin && !options.isPending && !options.error && !backends.length ? <Link className="text-sm underline underline-offset-4" to="/connections">{t("sources.connectCore")}</Link> : null}
      {bitcoin && backends.length > 0 && <form className="space-y-3" onSubmit={event => {
        event.preventDefault();
        if (busy) return;
        const submittedScope = scope;
        setPreview(null);
        void act(async () => {
          const response = await plan.mutateAsync(spec);
          if (response.data && submittedScope === liveScope.current) setPreview({ input, plan: response.data, scope: submittedScope });
        });
      }}>
        <fieldset disabled={busy} className="grid gap-3 sm:grid-cols-2">
          <label className="ca-field">{t("acquire.backend")}
            <select className="ca-select" value={backend} onChange={event => setBackend(event.target.value)} required>
              <option value="">{t("sources.chooseCore")}</option>
              {backends.map(item => <option value={item.name} key={item.name}>{item.name}</option>)}
            </select>
          </label>
          <label className="ca-field">{t("sources.mode")}
            <select className="ca-select" value={mode} onChange={event => setMode(event.target.value as "subject" | "blocks")}>
              <option value="subject">{t("sources.subject")}</option><option value="blocks">{t("sources.blocks")}</option>
            </select>
          </label>
          {mode === "subject" ? <label className="ca-field sm:col-span-2">{t("sources.txid")}
            <input className="ca-input font-mono" value={txid} onChange={event => setTxid(event.target.value)} pattern="[a-f0-9]{64}" required />
          </label> : <>
            <label className="ca-field">{t("sources.start")}<input className="ca-input" type="number" min="0" step="1" value={start} onChange={event => setStart(event.target.value)} required /></label>
            <label className="ca-field">{t("sources.end")}<input className="ca-input" type="number" min={start} step="1" value={end} onChange={event => setEnd(event.target.value)} placeholder={t("sources.followTip")} /></label>
          </>}
        </fieldset>
        <details><summary className="cursor-pointer text-xs">{t("sources.budgets")}</summary>
          <fieldset disabled={busy} className="mt-3 grid gap-3 sm:grid-cols-2">
            <label className="ca-field">{t("sources.interval")}<input className="ca-input" type="number" min="30" max="86400" step="1" value={interval} onChange={event => setInterval(event.target.value)} required /></label>
            <label className="ca-field">{t("sources.days")}<input className="ca-input" type="number" min="1" max="365" step="1" value={days} onChange={event => setDays(event.target.value)} required /></label>
            <label className="ca-field">{t("sources.requests")}<input className="ca-input" type="number" min="4" max="100000000" step="1" value={requests} onChange={event => setRequests(event.target.value)} required /></label>
            <label className="ca-field">{t("sources.megabytes")}<input className="ca-input" type="number" min="8" step="1" value={megabytes} onChange={event => setMegabytes(event.target.value)} required /></label>
          </fieldset>
        </details>
        <Button variant="outline" size="sm" type="submit" disabled={busy || !selected}>{t("sources.preview")}</Button>
      </form>}
      {ready && <div className="space-y-3 rounded-md border p-3" role="status">
        <p className="break-all font-mono text-xs">{preview.plan.spec.network} · {preview.plan.spec.mode === "subject" ? preview.plan.spec.subject : `${preview.plan.spec.start_height} → ${preview.plan.spec.end_height ?? t("sources.followTip")}`}</p>
        <p className="text-sm">{t("sources.permission", { backend: preview.plan.spec.backend, seconds: preview.plan.spec.interval_seconds, days: preview.plan.spec.duration_days })}</p>
        <p className="text-xs text-muted-foreground">{t("sources.quota", { requests: preview.plan.spec.max_requests, megabytes: preview.plan.spec.max_bytes / 1024 / 1024 })}</p>
        <Button size="sm" disabled={busy} onClick={() => void act(async () => { await authorize.mutateAsync({ plan: preview.plan }); setPreview(null); })}>{t("sources.authorize")}</Button>
      </div>}
      {(sources.data?.data?.items || []).map(item => <div key={item.id} className="flex flex-wrap items-center justify-between gap-3 border-t pt-3">
        <div className="min-w-0 space-y-1">
          <p className="text-sm font-medium">{item.spec.backend} · {item.spec.mode === "blocks" ? t("sources.blocks") : t("sources.subject")}</p>
          <p className="text-xs text-muted-foreground">{t("sources.progress", { height: item.cursor_height ?? "—", used: item.requests_used, total: item.spec.max_requests })}</p>
          <p className="text-xs">{item.status === "active" ? t("sources.active") : item.status === "revoked" ? t("sources.revoked") : t("sources.paused")}{item.last_code && <span className="ml-2 text-muted-foreground">{t(reasonKey(item.last_code))}</span>}</p>
        </div>
        {item.status === "active" && <div className="flex gap-2">
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => void act(() => enqueue.mutateAsync({ id: item.id }))}>{t("sources.run")}</Button>
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void act(() => revoke.mutateAsync({ id: item.id, expected_revision: item.revision }))}>{t("sources.revoke")}</Button>
        </div>}
      </div>)}
    </div>
  </details>;
}

function reasonKey(code: string) {
  if (code === "backfilling") return "sources.backfilling" as const;
  if (code === "caught_up" || code === "range_complete" || code === "observed") return "sources.current" as const;
  if (code === "reorg_reconciling") return "sources.reorg" as const;
  if (code === "acquisition_quota") return "sources.limitReached" as const;
  if (code === "acquisition_expired") return "sources.expired" as const;
  if (["acquisition_source_changed", "scope_changed", "backend_network_mismatch"].includes(code)) return "sources.changed" as const;
  return "sources.unavailable" as const;
}
