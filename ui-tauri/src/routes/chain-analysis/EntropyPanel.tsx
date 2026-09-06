import { useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { DaemonScopeContext, useDaemonMutation } from "@/daemon/client";
import { analysisNetworkInput, shortAnalysisId, type AnalysisEntropyRequest, type AnalysisNode, type AnalysisQuery } from "@/lib/chainAnalysis";
import { entropyLinks, entropyLinkChanges, entropyScenario, entropySubjectDomain, type EntropyJob, type EntropyScenario } from "@/lib/chainAnalysisWorkbench";
import { Fact } from "./EvidenceDetails";
import { CodeList, StructuredValue } from "./FeatureDetails";

export function EntropyPanel({ subject, query, onError, psbtSource, onJob, observedNodes = [] }: {
  subject: string; query: AnalysisQuery; onError: (error: unknown) => void;
  observedNodes?: AnalysisNode[];
  psbtSource?: { source_token: string; network: string };
  onJob?: (jobId: string) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const boundary = useContext(DaemonScopeContext);
  const [value, setValue] = useState(subject);
  const [domainEdit, setDomainEdit] = useState<{ subject: string; chain: "bitcoin" | "liquid" | ""; network: string } | null>(null);
  const inferredDomain = entropySubjectDomain(value, observedNodes, query);
  const domain: { chain: "bitcoin" | "liquid" | ""; network: string } = domainEdit?.subject === value.trim() ? domainEdit : inferredDomain.status === "resolved"
    ? inferredDomain
    : { chain: query.chain || "", network: query.network ? analysisNetworkInput(query.network) : "" };
  const { chain, network } = domain;
  const resolvedSubject = entropySubjectDomain(value, observedNodes, { chain: chain || undefined, network: network || undefined });
  const domainReady = !!chain && !!network && !(chain === "liquid" && network === "signet") && resolvedSubject.status !== "ambiguous";
  const [observer, setObserver] = useState<"owner" | "public">(query.observer === "public" ? "public" : "owner");
  const [budget, setBudget] = useState(200000);
  const [duration, setDuration] = useState(1000);
  const [kind, setKind] = useState<EntropyScenario["kind"]>("independent");
  const [protocol, setProtocol] = useState<EntropyScenario["protocol"]>("generic");
  const [received, setReceived] = useState("0");
  const [paid, setPaid] = useState("0");
  const [job, setJob] = useState<EntropyJob | null>(null);
  const [baseline, setBaseline] = useState<Record<string, unknown> | null>(null);
  const [pollError, setPollError] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
      mounted.current = true;
      return () => {
          mounted.current = false;
      };
  }, []);
  useEffect(() => {
      setValue(subject);
      setDomainEdit(null);
  }, [subject]);
  const start = useDaemonMutation<EntropyJob>(psbtSource ? "ui.chain_analysis.psbt.entropy.start" : "ui.chain_analysis.entropy.start", { invalidateQueries: false });
  const { mutateAsync: getJob } = useDaemonMutation<EntropyJob>("ui.chain_analysis.jobs.get", { invalidateQueries: false });
  const cancel = useDaemonMutation<EntropyJob>("ui.chain_analysis.jobs.cancel", { invalidateQueries: false });
  const accept = (next: EntropyJob | undefined) => {
      if (next && mounted.current && boundary?.isCurrent?.() !== false) {
          setJob(next);
          onJob?.(next.job_id);
      }
  };
  useEffect(() => {
      if (!job || job.status !== "running" || pollError)
          return;
      let live = true;
      const timer = window.setTimeout(() => {
          void getJob({ job_id: job.job_id }).then(response => {
              if (live && boundary?.isCurrent?.() !== false && response.data)
                  setJob(response.data);
          }).catch(error => {
              if (live) {
                  setPollError(true);
                  onError(error);
              }
          });
      }, 500);
      return () => {
          live = false;
          window.clearTimeout(timer);
      };
  }, [job, getJob, boundary, onError, pollError]);
  const scenario = entropyScenario(kind, protocol, received, paid);
  const busy = start.isPending || job?.status === "running";
  const request: AnalysisEntropyRequest = {
    subject: resolvedSubject.status === "resolved" ? resolvedSubject.subject : value.trim(),
    chain: chain || undefined,
    network,
    observer,
    max_states: budget,
    max_duration_ms: duration,
    ...(scenario ? { scenario } : {})
  };
  const result = job?.result;
  const changes = baseline && result ? entropyLinkChanges(baseline, result) : null;
  return <section className="space-y-4">
    <p className="max-w-3xl text-sm leading-relaxed text-muted-foreground">
      {t("entropyPanel.description")}
      </p>
    <form onSubmit={async event => {
        event.preventDefault();
        if (busy || !scenario || (!psbtSource && !domainReady))
            return;
        setPollError(false);
        try {
            const args = psbtSource ? {
                psbt_token: psbtSource.source_token,
                network: psbtSource.network,
                scenario,
                max_states: budget,
                max_duration_ms: duration
            } : { ...request };
            accept((await start.mutateAsync(args)).data);
        }
        catch (error) {
            onError(error);
        }
    }}>
      <fieldset disabled={busy} className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {!psbtSource && <label className="ca-field sm:col-span-2">
          {t("entropyPanel.subject")}
          <input
          className="ca-input font-mono"
          required
          value={value}
          onChange={e => {
            setValue(e.target.value);
            setDomainEdit(null);
          }} />
          </label>}
        {!psbtSource && <>
        <label className="ca-field">
          {t("chain")}
          <select
            className="ca-input"
            required
            value={chain}
            onChange={e => setDomainEdit({ subject: value.trim(), chain: e.target.value as typeof chain, network })}>
            <option value="">{t("entropyPanel.chooseDomain")}</option>
            <option value="bitcoin">Bitcoin</option>
            <option value="liquid">Liquid</option>
          </select>
        </label>
        <label className="ca-field">
          {t("network")}
          <select
            className="ca-input"
            required
            value={network}
            onChange={e => setDomainEdit({ subject: value.trim(), chain, network: e.target.value })}>
            <option value="">{t("entropyPanel.chooseDomain")}</option>
            {["main", "test", "signet", "regtest"].filter(n => chain !== "liquid" || n !== "signet").map(n => <option key={n}>
              {n}
              </option>)}
            </select>
          </label>
          <label className="ca-field">
          {t("workbench.observer")}
          <select
            className="ca-input"
            value={observer}
            onChange={e => setObserver(e.target.value as typeof observer)}>
            <option value="owner">
              {t("workbench.owner")}
              </option>
            <option value="public">
              {t("workbench.public")}
              </option>
            </select>
          </label>
        </>}
        <label className="ca-field">
          {t("workbench.scenario")}
          <select
            className="ca-input"
            value={kind}
            onChange={e => setKind(e.target.value as typeof kind)}>
            <option value="independent">
              {t("workbench.independent")}
              </option>
            <option value="coinjoin_intrafees">
              {t("workbench.intrafees")}
              </option>
            </select>
          </label>
        <label className="ca-field">
          {t("workbench.protocol")}
          <select
            className="ca-input"
            value={protocol}
            onChange={e => setProtocol(e.target.value as typeof protocol)}>
            {["generic", "whirlpool", "joinmarket", "wabisabi"].map(p => <option key={p}>
              {p}
              </option>)}
            </select>
          </label>
        <label className="ca-field">
          {t("entropyPanel.budget")}
          <input
          className="ca-input"
          required
          type="number"
          min={1}
          max={2000000}
          value={budget}
          onChange={e => setBudget(Number(e.target.value))} />
          </label>
        <label className="ca-field">
          {t("workbench.duration")}
          <input
          className="ca-input"
          required
          type="number"
          min={1}
          max={30000}
          value={duration}
          onChange={e => setDuration(Number(e.target.value))} />
          </label>
        {kind === "coinjoin_intrafees" && <>
        <label className="ca-field">
          {t("workbench.received")}
          <input
          className="ca-input font-mono"
          inputMode="numeric"
          value={received}
          onChange={e => setReceived(e.target.value)} />
          </label>
        <label className="ca-field">
          {t("workbench.paid")}
          <input
          className="ca-input font-mono"
          inputMode="numeric"
          value={paid}
          onChange={e => setPaid(e.target.value)} />
          </label>
        </>}
        <p className="text-xs text-muted-foreground sm:col-span-2 lg:col-span-4">
          {t("workbench.scenarioHelp")}
          </p>
        {!scenario && <p className="text-xs text-destructive sm:col-span-2" role="alert">
          {t("workbench.exactMsat")}
          </p>}
        {!psbtSource && !domainReady && <p className="text-xs text-muted-foreground sm:col-span-2 lg:col-span-4">
          {t("entropyPanel.domainRequired")}
        </p>}
        <Button
          type="submit"
          size="sm"
          disabled={busy || !scenario || (!psbtSource && (!value.trim() || !domainReady))}>
          {t("entropyPanel.run")}
          </Button>
      </fieldset>
    </form>
    {job && <div className="rounded-lg border bg-muted/15 p-4 space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-medium">
            {t("workbench.submitted")}
            </h3>
          <p className="mt-1 break-all font-mono text-[10px] text-muted-foreground">
            {job.request.snapshot_id || result?.snapshot_id as string || "—"}
            </p>
          </div>
        {job.status === "running" && <Button
          size="sm"
          variant="outline"
          disabled={cancel.isPending || job.cancel_requested}
          onClick={async () => {
              try {
                  accept((await cancel.mutateAsync({ job_id: job.job_id })).data);
              }
              catch (e) {
                  onError(e);
              }
          }}>
          {job.cancel_requested ? t("workbench.cancelling") : t("workbench.cancel")}
          </Button>}
      </div>
      <div
        className="flex flex-wrap gap-5 text-xs"
        role="status"
        aria-live="polite">
        <span>
          {job.status} · {job.progress.phase}
          </span>
        <span>
          {t("workbench.work", { count: job.progress.states_explored })}
          </span>
        <span>
          {t("workbench.elapsed", { ms: job.progress.elapsed_ms ?? job.elapsed_ms })}
          </span>
        </div>
      {pollError && <Button
        size="sm"
        variant="outline"
        onClick={() => setPollError(false)}>
        {t("workbench.resume")}
        </Button>}
      {job.error_code && <p className="text-sm text-destructive" role="alert">
        {job.error_code}
        </p>}
      {result && <>
      <EntropyResult result={result} request={job.request} />
        {result.status === "exact" && <Button
        size="sm"
        variant="outline"
        onClick={() => setBaseline(result)}>
        {t("workbench.pinBaseline")}
        </Button>}
        {baseline && <div className="space-y-2 border-t pt-3">
        <h4 className="text-sm font-medium">
          {t("workbench.comparison")}
          </h4>
        <div className="text-xs">
          <StructuredValue value={baseline.scenario} />
          </div>
        <p className="text-xs text-muted-foreground">
          {t("workbench.comparisonHelp")}
          </p>
          {changes === null ? <p className="text-xs">
          {t("workbench.incomparable")}
          </p> : <>
        <dl className="grid gap-3 sm:grid-cols-3">
          <Fact label={t("workbench.before")} value={baseline.interpretation_count} />
          <Fact label={t("workbench.after")} value={result.interpretation_count} />
          <Fact label={t("workbench.changedLinks")} value={changes.length} />
          </dl>
            <div className="max-h-64 overflow-auto">
          <table className="w-full text-left text-xs">
            <thead>
              <tr>
                <th>
                  {t("workbench.link")}
                  </th>
                <th>
                  {t("workbench.before")}
                  </th>
                <th>
                  {t("workbench.after")}
                  </th>
                </tr>
              </thead>
            <tbody>
              {changes.slice(0, 100).map(r => <tr key={`${r.input_id}:${r.output_id}`} className="border-t">
                <td className="py-2 font-mono">
                  {shortAnalysisId(r.input_id)} → {shortAnalysisId(r.output_id)}
                  </td>
                <td className="break-all font-mono">
                  {r.before}
                  </td>
                <td className="break-all font-mono">
                  {r.after}
                  </td>
                </tr>)}
              </tbody>
            </table>
          </div>
        {changes.length > 100 && <p className="text-xs">
          {t("workbench.firstRows", {
            count: 100,
            total: changes.length
          })}
          </p>}
        </>}
        </div>}
      </>}
    </div>}
  </section>;
}

export function EntropyResult({ result, request }: { result: Record<string, unknown>; request?: AnalysisEntropyRequest }) {
  const { t } = useTranslation("chainAnalysis");
  const [page, setPage] = useState(0);
  const exact = result.status === "exact", bounded = ["model_bounded", "timeout", "cancelled"].includes(String(result.status));
  const links = exact ? entropyLinks(result.link_counts) : [];
  const shown = Math.min(page, Math.max(0, Math.ceil(links.length / 50) - 1));
  return <div className="space-y-4">
    <dl className="grid gap-4 sm:grid-cols-3">
      <Fact label={t("entropyPanel.resultSubject")} value={result.subject || request?.subject} />
      {request && <>
      <Fact label={t("entropyPanel.requestDomain")} value={`${request.chain || "bitcoin"} / ${request.network || t("all")}`} />
      <Fact label={t("entropyPanel.budget")} value={request.max_states} />
      </>}
      </dl>
    <dl className="grid gap-4 sm:grid-cols-4">
      <Fact label={t("entropyPanel.status")} value={result.status} />
      <Fact label={t("entropyPanel.model")} value={result.model} />
      {(exact || bounded) && <Fact label={exact ? t("entropyPanel.partitions") : t("entropyPanel.lowerBound")} value={exact ? result.interpretation_count : result.interpretation_count_lower_bound} />}
      {exact && <Fact label={t("entropyPanel.bits")} value={result.entropy_bits} />}
      </dl>
    <Fact label={t("entropyPanel.reason")} value={result.reason} />
    {!!result.scenario && <div className="rounded-lg border p-3 text-xs">
      <h4 className="mb-2 font-medium">
        {t("workbench.scenario")}
        </h4>
      <StructuredValue value={result.scenario} />
      </div>}
    <div className="grid gap-4 sm:grid-cols-2">
      <CodeList title={t("entropyPanel.assumptions")} values={result.assumptions} />
      <CodeList title={t("entropyPanel.limitations")} values={result.limitations} />
      </div>
    {!!links.length && <details className="rounded-lg border p-3">
      <summary className="cursor-pointer text-xs font-medium">
        {t("entropyPanel.details")} · {links.length}
        </summary>
      <p className="my-3 text-xs text-muted-foreground">
        {t("workbench.conditionalLinks")}
        </p>
      <div className="max-h-72 overflow-auto">
        <table className="w-full text-left text-xs">
          <thead>
            <tr>
              <th>
                {t("from")}
                </th>
              <th>
                {t("to")}
                </th>
              <th>
                {t("entropyPanel.partitions")}
                </th>
              </tr>
            </thead>
          <tbody>
            {links.slice(shown * 50, shown * 50 + 50).map(r => <tr className="border-t" key={`${r.input_id}:${r.output_id}`}>
              <td className="break-all p-2 font-mono">
                {r.input_id}
                </td>
              <td className="break-all p-2 font-mono">
                {r.output_id}
                </td>
              <td className="break-all p-2 font-mono">
                {r.interpretation_count}/{String(result.interpretation_count)}
                </td>
              </tr>)}
            </tbody>
          </table>
        </div>
      <div className="mt-2 flex gap-2">
        <Button
          size="sm"
          variant="ghost"
          disabled={!shown}
          onClick={() => setPage(shown - 1)}>
          {t("workbench.previous")}
          </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={(shown + 1) * 50 >= links.length}
          onClick={() => setPage(shown + 1)}>
          {t("workbench.next")}
          </Button>
        </div>
      </details>}
  </div>;
}
