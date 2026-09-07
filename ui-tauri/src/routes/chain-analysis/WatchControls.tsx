import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import { shortAnalysisId } from "@/lib/chainAnalysis";
import { watchQueryScope, type WatchBinding } from "@/lib/chainAnalysisWatchScope";
import type { AnalysisQuery } from "@/lib/chainAnalysis";
type Rule = "output_spent" | "confirmations" | "connection_supported" | "attribution_changed" | "findings_changed";
type Definition = {
    rule: Rule;
    query?: AnalysisQuery;
    case_id?: string;
    threshold?: number;
};
type WatchStatus = "observed" | "unknown" | "unavailable" | "partial";
type EventCode = "coverage_lost" | "threshold_reached" | "threshold_reversed" | "spend_observed" | "connection_supported" | "attribution_changed" | "findings_changed" | "evidence_changed";
type Preview = {
    plan_id: string;
    baseline: {
        status: WatchStatus;
    };
    definition: Definition;
};
type Watch = {
    id: string;
    enabled: boolean;
    revision: number;
    definition: Definition;
    baseline: {
        status: WatchStatus;
    };
    checked_at: string;
};
type Inbox = {
    items: {
        id: string;
        watch_id: string;
        code: EventCode;
        created_at: string;
        acknowledged_at: string | null;
    }[];
    unread_count: number;
    next_cursor: number | null;
};
/** Local preview is always followed by explicit activation with unchanged facts. */
export function WatchAction({ query, caseId, output = false, onError }: {
    query?: AnalysisQuery;
    caseId?: string;
    output?: boolean;
    onError: (error: unknown) => void;
}) {
    const { t } = useTranslation("chainAnalysis");
    const [open, setOpen] = useState(false);
    const [rule, setRule] = useState<Rule>(output ? "output_spent" : query?.mode === "path" ? "connection_supported" : "findings_changed");
    const [threshold, setThreshold] = useState(1);
    const [plan, setPlan] = useState<Preview | null>(null);
    const [saved, setSaved] = useState(false);
    const [needsEncryption, setNeedsEncryption] = useState(false);
    const preview = useDaemonMutation<Preview>("ui.chain_analysis.watches.preview", { invalidateQueries: false });
    const create = useDaemonMutation("ui.chain_analysis.watches.create");
    const binding = useDaemon<WatchBinding>("ui.networks.binding", undefined, { enabled: open });
    const scopedQuery = watchQueryScope(query, binding.data?.data);
    const bound = binding.data?.data?.state === "bound";
    const busy = preview.isPending || create.isPending;
    const options: Rule[] = output ? ["output_spent", "confirmations", "attribution_changed", "findings_changed"] : query?.mode === "path" ? ["connection_supported", "findings_changed"] : ["findings_changed"];
    return <div className="text-xs">
    <Button size="sm" variant="ghost" onClick={() => { setOpen(!open); setPlan(null); }}>{t("watch.action")}</Button>
    {open && <div className="my-2 flex flex-wrap items-center gap-2 rounded border p-3">
      <p className="basis-full text-muted-foreground">{t("watch.localOnly")}</p>
      {!bound && <p className="basis-full">{binding.isFetching ? t("scopeLoading") : binding.isError ? binding.error.message : <Link to="/settings/bitcoin" className="underline">{t("watch.bindBook")}</Link>}</p>}
      {bound && <p className="basis-full text-muted-foreground">{scopedQuery?.chain ? `${scopedQuery.chain} / ${scopedQuery.network}` : t("watch.bookScope", { environment: binding.data?.data?.environment })}</p>}
      <select className="ca-input" aria-label={t("watch.rule")} value={rule} disabled={busy} onChange={event => { setRule(event.target.value as Rule); setPlan(null); setSaved(false); }}>
        {options.map(value => <option value={value} key={value}>{t(`watch.rules.${value}`)}</option>)}
      </select>
      {rule === "confirmations" && <input className="ca-input w-24" type="number" min={1} max={10000} value={threshold} aria-label={t("watch.threshold")} onChange={event => { setThreshold(Number(event.target.value)); setPlan(null); setSaved(false); }}/>}
      {!plan ? <Button size="sm" variant="outline" disabled={busy || saved || !bound} onClick={async () => {
                    try {
                        const response = await preview.mutateAsync({ rule, ...(caseId ? { case_id: caseId } : { query: scopedQuery }), ...(rule === "confirmations" ? { threshold } : {}) });
                        setPlan(response.data ?? null);
                    }
                    catch (error) {
                        setNeedsEncryption(error instanceof DaemonRequestError && error.envelope.error?.code === "watch_requires_encrypted_database");
                        onError(error);
                    }
                }}>{t("watch.preview")}</Button> : <>
        <span>{t("watch.baseline", { status: t(`watch.status.${plan.baseline.status}`) })}</span>
        <Button size="sm" disabled={busy} onClick={async () => {
                    try {
                        await create.mutateAsync({ plan });
                        setPlan(null);
                        setSaved(true);
                    }
                    catch (error) {
                        setPlan(null);
                        onError(error);
                    }
                }}>{t("watch.enable")}</Button>
      </>}
      {needsEncryption && <Link to="/settings/security" className="underline">{t("watch.encryption")}</Link>}
      {saved && <span role="status">{t("watch.saved")}</span>}
    </div>}
  </div>;
}
export function WatchInbox({ onOpen, onError }: {
    onOpen: (query: AnalysisQuery) => void;
    onError: (error: unknown) => void;
}) {
    const { t } = useTranslation("chainAnalysis");
    const [before, setBefore] = useState<number>();
    const watches = useDaemon<{
        items: Watch[];
    }>("ui.chain_analysis.watches.list");
    const events = useDaemon<Inbox>("ui.chain_analysis.watches.inbox", { limit: 20, ...(before ? { before } : {}) });
    const remove = useDaemonMutation("ui.chain_analysis.watches.delete");
    const [removing, setRemoving] = useState<string | null>(null);
    const configure = useDaemonMutation("ui.chain_analysis.watches.configure");
    const acknowledge = useDaemonMutation("ui.chain_analysis.watches.acknowledge");
    const evaluate = useDaemonMutation("ui.chain_analysis.watches.evaluate");
    const busy = remove.isPending || configure.isPending || acknowledge.isPending || evaluate.isPending;
    const items = watches.data?.data?.items ?? [];
    const refresh = async () => { await Promise.all([watches.refetch(), events.refetch()]); };
    return <details className="rounded border">
    <summary className="cursor-pointer px-4 py-3 text-sm font-medium">{t("watch.inbox")} {events.data?.data?.unread_count ? `(${events.data.data.unread_count})` : ""}</summary>
    <div className="space-y-3 px-4 pb-4 text-xs">
      <p className="text-muted-foreground">{t("watch.localOnly")}</p>
      {(events.isError || watches.isError) && <p role="alert">{events.error?.message || watches.error?.message}</p>}
      {!items.length && <p>{t("watch.empty")}</p>}
      {items.map(watch => <div key={watch.id} className="flex flex-wrap items-center gap-2 border-b py-2">
        <div className="min-w-0 flex-1"><span>{t(`watch.rules.${watch.definition.rule}`)} · {t(`watch.status.${watch.baseline.status}`)}</span><p className="break-all font-mono text-muted-foreground">{watch.definition.query?.chain} / {watch.definition.query?.network} · {watch.definition.query?.observer && t(watch.definition.query.observer)} · {watch.definition.query?.subject ? shortAnalysisId(watch.definition.query.subject) : t("overview")}</p></div>
        <Button size="sm" variant="ghost" disabled={busy} onClick={async () => {
                if (removing !== watch.id) {
                    setRemoving(watch.id);
                    return;
                }
                try {
                    await remove.mutateAsync({ id: watch.id, expected_revision: watch.revision });
                    setRemoving(null);
                    await refresh();
                }
                catch (error) {
                    onError(error);
                }
            }}>{t(removing === watch.id ? "watch.confirmDelete" : "watch.delete")}</Button>
        {watch.definition.query && <Button size="sm" variant="ghost" onClick={() => onOpen(watch.definition.query!)}>{t("watch.open")}</Button>}
        <Button size="sm" variant="ghost" disabled={busy} onClick={async () => {
                try {
                    await configure.mutateAsync({ id: watch.id, expected_revision: watch.revision, enabled: !watch.enabled });
                    await refresh();
                }
                catch (error) {
                    onError(error);
                }
            }}>{t(watch.enabled ? "watch.pause" : "watch.resume")}</Button>
      </div>)}
      {items.length > 0 && <Button size="sm" variant="outline" disabled={busy} onClick={async () => {
                try {
                    await evaluate.mutateAsync({});
                    await refresh();
                }
                catch (error) {
                    onError(error);
                }
            }}>{t("watch.check")}</Button>}
      {events.data?.data?.items.map(item => <div key={item.id} className="flex flex-wrap items-center gap-2 border-b py-2">
        <span className="flex-1">{t(`watch.events.${item.code}`)} <time className="text-muted-foreground">{item.created_at}</time></span>
        {items.find(watch => watch.id === item.watch_id)?.definition.query && <Button size="sm" variant="ghost" onClick={() => onOpen(items.find(watch => watch.id === item.watch_id)!.definition.query!)}>{t("watch.open")}</Button>}
        {!item.acknowledged_at && <Button size="sm" variant="ghost" disabled={busy} onClick={async () => {
                    try {
                        await acknowledge.mutateAsync({ id: item.id });
                        await refresh();
                    }
                    catch (error) {
                        onError(error);
                    }
                }}>{t("watch.acknowledge")}</Button>}
      </div>)}
      {before && <Button size="sm" variant="ghost" onClick={() => setBefore(undefined)}>{t("watch.latest")}</Button>}
      {events.data?.data?.next_cursor && <Button size="sm" variant="ghost" onClick={() => setBefore(events.data!.data!.next_cursor!)}>{t("watch.older")}</Button>}
    </div>
  </details>;
}
