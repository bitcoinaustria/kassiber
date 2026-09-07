import { useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Database, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { useUiStore } from "@/store/ui";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { DaemonScopeContext, useDaemon, useDaemonMutation } from "@/daemon/client";
import { isFilePickerAvailable, pickChainAnalysisSource, type AnalysisSourceSelection } from "@/lib/filePicker";
import { datasetPreviewKey, datasetReplacementManifest, isCurrentDatasetPreview, type Dataset, type DatasetClaim, type DatasetManifest, type DatasetPreview } from "@/lib/chainAnalysisWorkbench";
import { StructuredValue } from "./FeatureDetails";
import { Fact } from "./EvidenceDetails";
import { useAnalysisJob } from "./useAnalysisJob";
import { JobProgress } from "./JobProgress";

const INITIAL_MANIFEST: DatasetManifest = {
  dataset_key: "",
  name: "",
  version: "",
  chain: "bitcoin",
  network: "main",
  source: "",
  license: "",
  attribution_method: "",
  visibility: "private"
};

export function DatasetsPanel({ onError }: { onError: (value: unknown) => void }) {
  const { t } = useTranslation("chainAnalysis");
  const boundary = useContext(DaemonScopeContext);
  const assistant = useContext(AssistantSessionContext);
  const [manifest, setManifest] = useState(INITIAL_MANIFEST);
  const [source, setSource] = useState<AnalysisSourceSelection | null>(null);
  const [format, setFormat] = useState("csv"), [adapter, setAdapter] = useState("generic");
  const [preview, setPreview] = useState<{ key: string; preview: DatasetPreview } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string>();
  const [previous, setPrevious] = useState<Dataset[]>([]);
  const list = useDaemon<{ items: Dataset[]; next_cursor?: string }>("ui.chain_analysis.datasets.list", {
    limit: 50,
    ...(cursor ? { cursor } : {})
  }, { staleTime: 0 });
  const [selected, setSelected] = useState<Dataset | null>(null);
  const [confirmRevoke, setConfirmRevoke] = useState(false);
  const inspect = useDaemonMutation<Dataset>("ui.chain_analysis.datasets.get", { invalidateQueries: false });
  const validate = useAnalysisJob<DatasetPreview>("ui.chain_analysis.datasets.preview.start", onError);
  const importPack = useAnalysisJob<Dataset>("ui.chain_analysis.datasets.import.start", onError);
  const discard = useAnalysisJob<{ id: string; discarded: boolean }>("ui.chain_analysis.datasets.discard.start", onError);
  const discardedJob = useRef("");
  const submittedPreview = useRef("");
  const completedImport = useRef("");
  const revoke = useDaemonMutation<Dataset>("ui.chain_analysis.datasets.revoke", { invalidateQueries: false });
  const busy = validate.busy || importPack.busy || discard.busy || revoke.isPending;
  const key = datasetPreviewKey(source?.source_token, manifest, format, adapter);
  const ready = isCurrentDatasetPreview(key, preview);
  const items = [...previous, ...(list.data?.data?.items || [])];
  const refresh = async () => {
      setCursor(undefined);
      setPrevious([]);
      // A page change mounts the first-page query and fetches its current data.
      // Refetching this render's cursor would refresh the old page instead.
      if (!cursor) await list.refetch();
  };
  const args = {
    source_token: source?.source_token,
    manifest,
    format,
    adapter
  };
  useEffect(() => {
      if (validate.job?.status === "completed" && validate.job.result && boundary?.isCurrent?.() !== false) {
          setPreview({
              key: submittedPreview.current,
              preview: validate.job.result
          });
      }
  }, [validate.job, boundary]);
  useEffect(() => {
      const job = importPack.job;
      if (job?.status !== "completed" || !job.result || completedImport.current === job.job_id || boundary?.isCurrent?.() === false)
          return;
      completedImport.current = job.job_id;
      setSelected(job.result);
      setNotice(t("datasets.imported"));
      setCursor(undefined);
      setPrevious([]);
      void list.refetch();
  }, [importPack.job, boundary, t, list]);
  useEffect(() => {
      const job = discard.job;
      if (job?.status !== "completed" || !job.result?.discarded || discardedJob.current === job.job_id || boundary?.isCurrent?.() === false)
          return;
      discardedJob.current = job.job_id;
      setSelected(null);
      setConfirmRevoke(false);
      setCursor(undefined);
      setPrevious([]);
      void list.refetch();
  }, [discard.job, boundary, list]);
  const ask = () => {
      if (!assistant || assistant.isStreaming || (!selected && !source) || boundary?.isCurrent?.() === false)
          return;
      // UUID/token only. Never interpolate the imported label, source, filename,
      // manifest prose or chain identities into an outgoing assistant request.
      const prompt = selected ? t("datasets.assistantDataset", { id: selected.id }) : t("datasets.assistantSource", { token: source!.source_token });
      const ui = useUiStore.getState();
      ui.setAssistantDockDiscovered(true);
      ui.setAssistantDockMinimized(false);
      ui.setAssistantDockExpanded(true);
      if (assistant.selection?.model)
          assistant.sendPrompt(prompt);
      else
          useAssistantDraftStore.getState().setDraft(prompt);
  };
  return <section className="space-y-5 rounded-xl border bg-card p-4 sm:p-6">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 className="flex items-center gap-2 text-lg font-semibold">
          <Database className="size-5" />
          {t("datasets.title")}
        </h2>
        <p className="mt-2 max-w-3xl text-sm text-muted-foreground">
          {t("datasets.help")}
        </p>
      </div>
      <Button
        size="sm"
        variant="outline"
        onClick={ask}
        disabled={!assistant || assistant.isStreaming || (!selected && !source)}>
        <Sparkles className="size-3.5" />
        {t("ask")}
      </Button>
    </div>
    {validate.job && <JobProgress
    job={validate.job}
    pollError={validate.pollError}
    onResume={validate.resume}
    onCancel={() => void validate.cancel().catch(onError)} />}
    {importPack.job && <JobProgress
    job={importPack.job}
    pollError={importPack.pollError}
    onResume={importPack.resume}
    onCancel={() => void importPack.cancel().catch(onError)} />}
    {discard.job && <JobProgress
    job={discard.job}
    pollError={discard.pollError}
    onResume={discard.resume}
    onCancel={() => void discard.cancel().catch(onError)} />}
    {notice && <p role="status" className="rounded-lg border bg-muted/20 p-3 text-xs">
      {notice}
    </p>}
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
      <form className="space-y-4" onSubmit={async event => {
          event.preventDefault();
          if (!source || busy)
              return;
          setPreview(null);
          setNotice(null);
          try {
              submittedPreview.current = key;
              await validate.run(args);
          }
          catch (error) {
              onError(error);
          }
      }}>
        <fieldset disabled={busy} className="space-y-4">
          <div className="rounded-lg border border-dashed p-4">
            <p className="mb-3 break-all text-xs text-muted-foreground">
              {source ? `${source.filename} · ${source.size_bytes} B` : t("datasets.noFile")}
            </p>
            <Button
              size="sm"
              type="button"
              variant="outline"
              disabled={!isFilePickerAvailable || !boundary}
              onClick={async () => {
                  if (!boundary)
                      return;
                  try {
                      const value = await pickChainAnalysisSource("dataset", boundary.expectedScope);
                      if (value && boundary.isCurrent?.() !== false) {
                          setSource(value);
                          setPreview(null);
                      }
                  }
                  catch (error) {
                      onError(error);
                  }
              }}>
              {t("workbench.chooseFile")}
            </Button>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            {(["dataset_key", "name", "version", "source", "license", "attribution_method"] as const).map(field => <label className="ca-field" key={field}>
              {t(`datasets.manifest.${field}`)}
              <input
              required
              maxLength={field === "source" || field === "license" ? 512 : 256}
              className="ca-input"
              value={manifest[field]}
              onChange={e => {
                  setManifest({
                      ...manifest,
                      [field]: e.target.value
                  });
              }} />
            </label>)}
            <label className="ca-field">
              {t("chain")}
              <select
                className="ca-input"
                value={manifest.chain}
                onChange={e => setManifest({
                  ...manifest,
                  chain: e.target.value as DatasetManifest["chain"]
                })}>
                <option value="bitcoin">Bitcoin</option>
                <option value="liquid">Liquid</option>
              </select>
            </label>
            <label className="ca-field">
              {t("network")}
              <select
                className="ca-input"
                value={manifest.network}
                onChange={e => setManifest({
                  ...manifest,
                  network: e.target.value
                })}>
                {["main", "test", "regtest", ...(manifest.chain === "bitcoin" ? ["signet"] : [])].map(n => <option key={n}>
                  {n}
                </option>)}
              </select>
            </label>
            <label className="ca-field">
              {t("datasets.visibility")}
              <select
                className="ca-input"
                value={manifest.visibility}
                onChange={e => setManifest({
                  ...manifest,
                  visibility: e.target.value as DatasetManifest["visibility"]
                })}>
                <option value="private">
                  {t("datasets.private")}
                </option>
                <option value="public">
                  {t("datasets.public")}
                </option>
              </select>
            </label>
            <label className="ca-field">
              {t("datasets.format")}
              <select
                className="ca-input"
                value={format}
                onChange={e => setFormat(e.target.value)}>
                <option value="csv">CSV</option>
                <option value="jsonl">JSONL</option>
              </select>
            </label>
            <label className="ca-field">
              {t("datasets.adapter")}
              <select
                className="ca-input"
                value={adapter}
                onChange={e => setAdapter(e.target.value)}>
                {["generic", "am_i_exposed", "maru92"].map(a => <option key={a}>
                  {a}
                </option>)}
              </select>
            </label>
            <label className="ca-field">
              {t("datasets.replaces")}
              <input
              className="ca-input font-mono"
              value={manifest.expected_active_id || ""}
              onChange={e => setManifest({
                ...manifest,
                expected_active_id: e.target.value || undefined
              })} />
            </label>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("datasets.visibilityHelp")}
          </p>
          <p className="text-xs text-muted-foreground">
            {t("datasets.formatHelp")}
          </p>
          <Button
            type="submit"
            size="sm"
            disabled={!source || busy}>
            {validate.busy ? t("datasets.validating") : t("datasets.preview")}
          </Button>
        </fieldset>
        {preview && !ready && <p className="text-xs text-muted-foreground" role="status">
          {t("datasets.previewChanged")}
        </p>}
        {ready && preview && <div className="space-y-3 rounded-lg border bg-muted/15 p-4">
          <h3 className="text-sm font-medium">
            {t("datasets.validated")}
          </h3>
          <dl className="grid gap-3 sm:grid-cols-2">
            <Fact label={t("datasets.rows")} value={preview.preview.row_count} />
            <Fact label={t("datasets.bytes")} value={preview.preview.byte_count} />
            <Fact label="SHA-256" value={preview.preview.sha256} />
          </dl>
          <ClaimTable claims={preview.preview.sample_claims} />
          <p className="text-xs text-muted-foreground">
            {t("datasets.confirmHelp")}
          </p>
          <Button
            type="button"
            size="sm"
            disabled={busy || !ready}
            onClick={async () => {
                if (!ready || !preview)
                    return;
                try {
                    await importPack.run({
                        ...args,
                        expected_sha256: preview.preview.sha256
                    });
                    setPreview(null);
                }
                catch (error) {
                    setPreview(null);
                    onError(error);
                }
            }}>
            {importPack.busy ? t("datasets.importing") : t("datasets.confirmImport")}
          </Button>
        </div>}
      </form>
      <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <h3 className="text-sm font-medium">
            {t("datasets.installed")}
          </h3>
          <Button
            size="sm"
            variant="ghost"
            disabled={list.isFetching || busy}
            onClick={() => void refresh()}>
            {t("datasets.refresh")}
          </Button>
        </div>
        {list.isError && <p className="text-xs text-destructive" role="alert">
          {list.error.message}
        </p>}
        <div className="max-h-80 divide-y overflow-auto rounded-lg border">
          {!items.length && <p className="p-4 text-xs text-muted-foreground">
            {t("datasets.empty")}
          </p>}
          {items.map(pack => <button
            type="button"
            key={pack.id}
            className="block w-full p-3 text-left hover:bg-muted/30"
            disabled={inspect.isPending}
            onClick={async () => {
                try {
                    const response = await inspect.mutateAsync({ id: pack.id });
                    if (response.data && boundary?.isCurrent?.() !== false) {
                        setSelected(response.data);
                        setConfirmRevoke(false);
                    }
                }
                catch (error) {
                    onError(error);
                }
            }}>
            <span className="block text-sm font-medium">
              {pack.manifest.name} <span className="text-muted-foreground">
                {pack.version}
              </span>
            </span>
            <span className="mt-1 block text-[10px] text-muted-foreground">
              {pack.status} · {pack.visibility} · {pack.row_count}
            </span>
          </button>)}
        </div>
        {list.data?.data?.next_cursor && <Button
          size="sm"
          variant="ghost"
          disabled={list.isFetching}
          onClick={() => {
              setPrevious(items);
              setCursor(list.data?.data?.next_cursor);
          }}>
          {t("workbench.next")}
        </Button>}
        {selected && <div className="space-y-3 rounded-lg border p-4">
          <h3 className="text-sm font-medium">
            {selected.manifest.name} · {selected.version}
          </h3>
          <dl className="space-y-3">
            <Fact label={t("id")} value={selected.id} />
            <Fact label={t("status")} value={selected.status} />
            <Fact label="SHA-256" value={selected.content_sha256} />
          </dl>
          <div className="text-xs">
            <StructuredValue value={selected.manifest} />
          </div>
          {["failed", "staging"].includes(selected.status) && <div className="space-y-2">
            <p className="text-xs text-muted-foreground">
              {t("datasets.discardHelp")}
            </p>
            <Button
              size="sm"
              variant={confirmRevoke ? "destructive" : "outline"}
              disabled={busy}
              onClick={async () => {
                  if (!confirmRevoke) {
                      setConfirmRevoke(true);
                      return;
                  }
                  try {
                      await discard.run({ id: selected.id });
                  }
                  catch (error) {
                      onError(error);
                  }
              }}>
              {t(confirmRevoke ? "datasets.confirmDiscard" : "datasets.discard")}
            </Button>
            {confirmRevoke && <Button
              size="sm"
              variant="ghost"
              onClick={() => setConfirmRevoke(false)}>
              {t("dismiss")}
            </Button>}
          </div>}
          {["active", "superseded"].includes(selected.status) && <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={busy || selected.status !== "active"}
              onClick={() => {
                  setManifest(datasetReplacementManifest(selected));
                  setPreview(null);
              }}>
              {t("datasets.replace")}
            </Button>
            <Button
              size="sm"
              variant={confirmRevoke ? "destructive" : "outline"}
              disabled={busy}
              onClick={async () => {
                  if (!confirmRevoke) {
                      setConfirmRevoke(true);
                      return;
                  }
                  try {
                      const response = await revoke.mutateAsync({
                          id: selected.id,
                          expected_revision: selected.revision
                      });
                      if (boundary?.isCurrent?.() === false)
                          return;
                      if (response.data)
                          setSelected(response.data);
                      setConfirmRevoke(false);
                      await refresh();
                  }
                  catch (error) {
                      onError(error);
                  }
              }}>
              {confirmRevoke ? t("datasets.confirmRevoke") : t("datasets.revoke")}
            </Button>
            {confirmRevoke && <Button
              size="sm"
              variant="ghost"
              onClick={() => setConfirmRevoke(false)}>
              {t("dismiss")}
            </Button>}
          </div>}
        </div>}
      </section>
    </div>
    <DatasetQuery
    key={selected?.id || "all"}
    dataset={selected}
    onError={onError} />
  </section>;
}

function DatasetQuery({ dataset, onError }: { dataset: Dataset | null; onError: (error: unknown) => void }) {
  const { t } = useTranslation("chainAnalysis");
  const boundary = useContext(DaemonScopeContext);
  const [mode, setMode] = useState<"subject" | "label">("subject"), [value, setValue] = useState("");
  const [chain, setChain] = useState("bitcoin"), [network, setNetwork] = useState("main"), [observer, setObserver] = useState("owner");
  const [outcome, setOutcome] = useState<{ key: string; items: DatasetClaim[]; next_cursor?: string } | null>(null);
  const search = useDaemonMutation<{ items: DatasetClaim[]; next_cursor?: string }>("ui.chain_analysis.datasets.query", { invalidateQueries: false });
  const args = {
    ...(mode === "subject" ? {
      subject: value.trim(),
      chain,
      network
    } : { label: value.trim() }),
    ...(dataset ? { dataset_id: dataset.id } : {}),
    observer,
    limit: 50
  };
  const key = JSON.stringify(args), current = outcome?.key === key ? outcome : null;
  const run = async (cursor?: string) => {
      if (!value.trim() || search.isPending || (mode === "label" && !dataset))
          return;
      try {
          const response = await search.mutateAsync({
              ...args,
              ...(cursor ? { cursor } : {})
          });
          if (response.data && boundary?.isCurrent?.() !== false)
              setOutcome({
                  key,
                  ...response.data,
                  items: response.data.items
              });
      }
      catch (error) {
          onError(error);
      }
  };
  return <section className="space-y-3 border-t pt-5">
    <h3 className="text-sm font-medium">
      {t("datasets.query")}
    </h3>
    <p className="text-xs text-muted-foreground">
      {t("datasets.queryHelp")}
    </p>
    <form className="flex flex-wrap items-end gap-3" onSubmit={e => {
        e.preventDefault();
        void run();
    }}>
      <label className="ca-field">
        {t("datasets.matchBy")}
        <select
          className="ca-input"
          value={mode}
          onChange={e => setMode(e.target.value as typeof mode)}>
          <option value="subject">
            {t("datasets.subject")}
          </option>
          <option value="label" disabled={!dataset}>
            {t("datasets.label")}
          </option>
        </select>
      </label>
      <label className="ca-field min-w-60 flex-1">
        {t(mode === "subject" ? "datasets.subject" : "datasets.label")}
        <input
        required
        className="ca-input"
        value={value}
        onChange={e => setValue(e.target.value)} />
      </label>
      {mode === "subject" && <>
      <label className="ca-field">
        {t("chain")}
        <select
          className="ca-input"
          value={chain}
          onChange={e => setChain(e.target.value)}>
          <option value="bitcoin">Bitcoin</option>
          <option value="liquid">Liquid</option>
        </select>
      </label>
      <label className="ca-field">
        {t("network")}
        <select
          className="ca-input"
          value={network}
          onChange={e => setNetwork(e.target.value)}>
          {["main", "test", "regtest", ...(chain === "bitcoin" ? ["signet"] : [])].map(n => <option key={n}>
            {n}
          </option>)}
        </select>
      </label>
    </>}
      <label className="ca-field">
        {t("workbench.observer")}
        <select
          className="ca-input"
          value={observer}
          onChange={e => setObserver(e.target.value)}>
          <option value="owner">
            {t("workbench.owner")}
          </option>
          <option value="public">
            {t("workbench.public")}
          </option>
        </select>
      </label>
      <Button size="sm" disabled={search.isPending || !value.trim()}>
        {t("datasets.search")}
      </Button>
    </form>
    {current && <>
    <ClaimTable claims={current.items} />
    {!current.items.length && <p className="text-xs text-muted-foreground">
      {t("datasets.noMatch")}
    </p>}
    {current.next_cursor && <Button
      size="sm"
      variant="outline"
      disabled={search.isPending}
      onClick={() => void run(current.next_cursor)}>
      {t("workbench.next")}
    </Button>}
  </>}
  </section>;
}

export function ClaimTable({ claims }: { claims: DatasetClaim[] }) {
  const { t } = useTranslation("chainAnalysis");
  return <div className="max-h-64 overflow-auto">
    <table className="w-full text-left text-xs">
      <thead>
        <tr>
          <th className="p-2">
            {t("datasets.subject")}
          </th>
          <th className="p-2">
            {t("datasets.label")}
          </th>
          <th className="p-2">
            {t("labels.category")}
          </th>
          <th className="p-2">
            {t("showEvidence")}
          </th>
        </tr>
      </thead>
      <tbody>
        {claims.map((claim, index) => <tr key={claim.id || index} className="border-t">
          <td className="break-all p-2 font-mono">
            {claim.subject}
          </td>
          <td className="break-words p-2">
            {claim.label}
          </td>
          <td className="p-2">
            {claim.category}
          </td>
          <td className="p-2">
            <details>
              <summary className="cursor-pointer">
                {claim.dataset_status || claim.confidence || t("showEvidence")}
              </summary>
              <StructuredValue value={claim} />
            </details>
          </td>
        </tr>)}
      </tbody>
    </table>
  </div>;
}
