import { useContext, useState } from "react";
import { useTranslation } from "react-i18next";
import { FileSearch, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { DaemonScopeContext, useDaemonMutation } from "@/daemon/client";
import { isFilePickerAvailable, pickChainAnalysisSource, type AnalysisSourceSelection } from "@/lib/filePicker";
import { DEFAULT_ANALYSIS_QUERY, formatAnalysisAmount } from "@/lib/chainAnalysis";
import { payjoinOptions, psbtAssistantContext, type PsbtAnalysis, type PsbtComparison } from "@/lib/chainAnalysisWorkbench";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { ANALYSIS_NETWORKS, type AnalysisNetwork, useUiStore } from "@/store/ui";
import { EntropyPanel } from "./EntropyPanel";
import { CodeList, FeatureDetails, StructuredValue } from "./FeatureDetails";
import { Fact } from "./EvidenceDetails";

export function PsbtPanel({ onError, initialNetwork }: { onError: (value: unknown) => void; initialNetwork?: string }) {
  const { t } = useTranslation("chainAnalysis");
  const { t: settingsT } = useTranslation("settings");
  const boundary = useContext(DaemonScopeContext);
  const assistant = useContext(AssistantSessionContext);
  const [before, setBefore] = useState<AnalysisSourceSelection | null>(null);
  const [after, setAfter] = useState<AnalysisSourceSelection | null>(null);
  // Pin the context for this inspection. A later Settings change must not
  // relabel selected files, completed results, or an in-flight entropy job.
  const [network] = useState(() => ANALYSIS_NETWORKS.includes(initialNetwork as AnalysisNetwork)
    ? initialNetwork as AnalysisNetwork
    : useUiStore.getState().analysisNetwork);
  const [payjoin, setPayjoin] = useState(false);
  const [payment, setPayment] = useState("0");
  const [feeOutput, setFeeOutput] = useState("");
  const [maximum, setMaximum] = useState("0");
  const [minimumRate, setMinimumRate] = useState("");
  const [substitute, setSubstitute] = useState(false);
  const [outcome, setOutcome] = useState<{ key: string; analysis?: PsbtAnalysis; comparison?: PsbtComparison } | null>(null);
  const [entropySide, setEntropySide] = useState<"before" | "after" | null>(null);
  const [entropyJob, setEntropyJob] = useState<{ job_id: string; source_token: string; network: string } | null>(null);
  const analyze = useDaemonMutation<PsbtAnalysis>("ui.chain_analysis.psbt.analyze", { invalidateQueries: false });
  const compare = useDaemonMutation<PsbtComparison>("ui.chain_analysis.psbt.compare", { invalidateQueries: false });
  const busy = analyze.isPending || compare.isPending;
  const inputKey = JSON.stringify([before?.source_token, after?.source_token, network, payjoin, payment, feeOutput, maximum, minimumRate, substitute]);
  const current = outcome?.key === inputKey ? outcome : null;
  const payjoinConstraints = payjoinOptions({
    payment,
    feeOutput,
    maximum,
    minimumRate,
    substitute
  });
  const validPayjoin = !payjoin || !!payjoinConstraints;
  const assistantInput = before ? psbtAssistantContext({
    beforeToken: before.source_token,
    afterToken: after?.source_token,
    network,
    payjoinEnabled: payjoin,
    constraints: payjoinConstraints,
    currentInputKey: inputKey,
    comparisonInputKey: outcome?.comparison ? outcome.key : undefined,
    entropyJob
  }) : null;
  const pick = async (side: "before" | "after") => {
      if (!boundary || boundary.isCurrent?.() === false)
          return;
      try {
          const selected = await pickChainAnalysisSource("psbt", boundary.expectedScope);
          if (selected && boundary.isCurrent?.() !== false) {
              (side === "before" ? setBefore : setAfter)(selected);
              setEntropySide(null);
          }
      }
      catch (error) {
          onError(error);
      }
  };
  const send = () => {
      if (!assistantInput || !assistant || assistant.isStreaming || boundary?.isCurrent?.() === false)
          return;
      // Only opaque grants and categorical context enter a provider prompt.
      const prompt = t("psbt.assistantPrompt", { selection: JSON.stringify(assistantInput) });
      const ui = useUiStore.getState();
      ui.setAssistantDockDiscovered(true);
      ui.setAssistantDockMinimized(false);
      ui.setAssistantDockExpanded(true);
      if (assistant.selection?.model)
          assistant.sendPrompt(prompt);
      else
          useAssistantDraftStore.getState().setDraft(prompt);
  };
  const entropySource = entropySide === "after" ? after : before;
  return <section className="space-y-5 rounded-xl border bg-card p-4 sm:p-6">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 className="flex items-center gap-2 text-lg font-semibold">
          <FileSearch className="size-5" />
          {t("psbt.title")}
          </h2>
        <p className="mt-2 max-w-3xl text-sm text-muted-foreground">
          {t("psbt.help")}
          </p>
        </div>
      <Button
        size="sm"
        variant="outline"
        onClick={send}
        disabled={!assistantInput || !assistant || assistant.isStreaming}>
        <Sparkles className="size-3.5" />
        {t("ask")}
        </Button>
      </div>
    <div className="grid gap-4 sm:grid-cols-2">
      {(["before", "after"] as const).map(side => {
      const source = side === "before" ? before : after;
      return <div key={side} className="rounded-lg border border-dashed p-4">
          <h3 className="text-sm font-medium">
            {t(side === "before" ? "psbt.original" : "psbt.proposal")}
            </h3>
          <p className="my-3 break-all text-xs text-muted-foreground">
            {source ? `${source.filename} · ${source.size_bytes} B` : t("psbt.noFile")}
            </p>
          <Button
            size="sm"
            variant="outline"
            disabled={busy || !isFilePickerAvailable || !boundary}
            onClick={() => void pick(side)}>
            {t("workbench.chooseFile")}
            </Button>
          {source && <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => {
              (side === "before" ? setBefore : setAfter)(null);
              setEntropySide(null);
              setEntropyJob(null);
            }}>
            {t("workbench.clearSelection")}
          </Button>}
          </div>;
    })}
      </div>
    {!isFilePickerAvailable && <p className="text-xs">
      {t("workbench.pickerUnavailable")}
      </p>}
    <p className="text-xs text-muted-foreground" title={t("psbt.networkHelp")}>
      {settingsT("analysisNetwork.effective", { network })}
    </p>
    <details className="rounded-lg border p-4">
      <summary className="cursor-pointer text-sm font-medium">
        {t("psbt.payjoin")}
        </summary>
      <label className="my-3 flex items-center gap-2 text-xs">
        <input
        type="checkbox"
        checked={payjoin}
        onChange={e => setPayjoin(e.target.checked)} />
        {t("psbt.enablePayjoin")}
        </label>
      {payjoin && <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <label className="ca-field">
          {t("psbt.paymentIndex")}
          <input
          className="ca-input"
          type="number"
          min={0}
          value={payment}
          onChange={e => setPayment(e.target.value)} />
          </label>
      <label className="ca-field">
          {t("psbt.feeIndex")}
          <input
          className="ca-input"
          type="number"
          min={0}
          value={feeOutput}
          onChange={e => setFeeOutput(e.target.value)} />
          </label>
      <label className="ca-field">
          {t("psbt.maximumFee")}
          <input
          className="ca-input"
          inputMode="numeric"
          value={maximum}
          onChange={e => setMaximum(e.target.value)} />
          </label>
      <label className="ca-field">
          {t("psbt.minimumRate")}
          <input
          className="ca-input"
          inputMode="decimal"
          value={minimumRate}
          onChange={e => setMinimumRate(e.target.value)} />
          </label>
      <label className="flex items-center gap-2 text-xs sm:col-span-2">
          <input
          type="checkbox"
          checked={substitute}
          onChange={e => setSubstitute(e.target.checked)} />
          {t("psbt.substitution")}
          </label>
    </div>}
      </details>
    <div className="flex flex-wrap gap-2">
      <Button
        size="sm"
        disabled={!before || busy}
        onClick={async () => {
            if (!before)
                return;
            try {
                const response = await analyze.mutateAsync({
                    psbt_token: before.source_token,
                    network
                });
                if (response.data && boundary?.isCurrent?.() !== false)
                    setOutcome({
                        key: inputKey,
                        analysis: response.data
                    });
            }
            catch (error) {
                onError(error);
            }
        }}>
        {t("psbt.analyze")}
        </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={!before || !after || busy || !validPayjoin}
        onClick={async () => {
            if (!before || !after || !validPayjoin)
                return;
            try {
                const response = await compare.mutateAsync({
                    before_token: before.source_token,
                    after_token: after.source_token,
                    network,
                    ...(payjoin ? { payjoin: payjoinConstraints } : {})
                });
                if (response.data && boundary?.isCurrent?.() !== false)
                    setOutcome({
                        key: inputKey,
                        comparison: response.data
                    });
            }
            catch (error) {
                onError(error);
            }
        }}>
        {t("psbt.compare")}
        </Button>
      </div>
    {outcome && !current && <p role="status" className="text-xs text-muted-foreground">
      {t("workbench.inputsChanged")}
      </p>}
    {current?.analysis && <PsbtResult result={current.analysis} />}
    {current?.comparison && <div className="space-y-5">
      <div className="grid gap-4 lg:grid-cols-2">
        <PsbtResult result={current.comparison.before} title={t("psbt.original")} />
        <PsbtResult result={current.comparison.after} title={t("psbt.proposal")} />
        </div>
      <h3 className="text-sm font-medium">
        {t("psbt.changes")}
        </h3>
      <div className="grid gap-3 text-xs sm:grid-cols-3">
        <Fact label={t("psbt.inputsAdded")} value={current.comparison.delta.inputs_added.length} />
        <Fact label={t("psbt.inputsRemoved")} value={current.comparison.delta.inputs_removed.length} />
        <Fact label={t("psbt.feeDelta")} value={formatAnalysisAmount(current.comparison.delta.fee_msat)} />
        </div>
      <div className="text-xs">
        <StructuredValue value={current.comparison.delta} />
        </div>
      {current.comparison.payjoin && <div className="space-y-3 rounded-lg border p-4">
        <h3 className="text-sm font-medium">
          {t("psbt.payjoin")} · {current.comparison.payjoin.status}
          </h3>
        <div className="divide-y">
          {current.comparison.payjoin.checks.map(check => <details className="py-2" key={check.code}>
            <summary className="cursor-pointer text-xs">
              {check.code.replace(/_/g, " ")} · <strong>
                {check.status}
                </strong>
              </summary>
            <div className="mt-2 text-xs">
              <StructuredValue value={check.details} />
              </div>
            </details>)}
          </div>
        <CodeList title={t("entropyPanel.limitations")} values={current.comparison.payjoin.limitations} />
        </div>}
    </div>}
    {current && <div className="flex gap-2">
      <Button
        size="sm"
        variant="outline"
        onClick={() => setEntropySide("before")}>
        {t("psbt.entropyOriginal")}
        </Button>
      {current.comparison && <Button
        size="sm"
        variant="outline"
        disabled={!!current.comparison.payjoin}
        onClick={() => setEntropySide("after")}>
        {t("psbt.entropyProposal")}
        </Button>}
      </div>}
    {current?.comparison?.payjoin && <p className="text-xs text-muted-foreground">
      {t("entropyPanel.unsupported")}
      </p>}
    {entropySide && entropySource && !(entropySide === "after" && payjoin) && <div className="rounded-lg border p-4">
      <h3 className="mb-3 text-sm font-medium">
        {t(entropySide === "before" ? "psbt.entropyOriginal" : "psbt.entropyProposal")}
        </h3>
      <EntropyPanel
      key={`${entropySource.source_token}:${network}`}
      subject="psbt:transaction"
      query={{
        ...DEFAULT_ANALYSIS_QUERY,
        network
      }}
      psbtSource={{
        source_token: entropySource.source_token,
        network
      }}
      onJob={job_id => setEntropyJob({
        job_id,
        source_token: entropySource.source_token,
        network
      })}
      onError={onError} />
      </div>}
  </section>;
}

export function PsbtResult({ result, title }: { result: PsbtAnalysis; title?: string }) {
  const { t } = useTranslation("chainAnalysis");
  const amount = (value: string | null) => value == null ? t("workbench.unknown") : formatAnalysisAmount(value);
  return <article className="space-y-4 rounded-lg border p-4">
    <h3 className="text-sm font-medium">
      {title || t("psbt.result")} · PSBT v{result.psbt_version}
      </h3>
    <dl className="grid gap-4 sm:grid-cols-3">
      <Fact label={t("psbt.totalIn")} value={amount(result.totals.input_msat)} />
      <Fact label={t("psbt.totalOut")} value={amount(result.totals.output_msat)} />
      <Fact label={t("psbt.fee")} value={amount(result.totals.fee_msat)} />
      <Fact label={t("psbt.knownInputs")} value={`${result.coverage.known_input_amounts}/${result.coverage.input_count}`} />
      <Fact label={t("psbt.verifiedPrevious")} value={result.coverage.previous_transaction_hashes_verified} />
      <Fact label={t("psbt.finalRate")} value={result.totals.final_fee_rate_sat_vb || t("workbench.unknown")} />
      </dl>
    <p className="text-xs text-muted-foreground">
      {t("psbt.validationHelp")}
      </p>
    <div className="max-h-64 overflow-auto">
      <table className="w-full text-left text-xs">
        <thead>
          <tr>
            <th>
              {t("psbt.input")}
              </th>
            <th>
              {t("amount")}
              </th>
            <th>
              {t("psbt.evidence")}
              </th>
            </tr>
          </thead>
        <tbody>
          {result.transaction_facts.inputs.map(input => <tr className="border-t" key={input.input_index}>
            <td className="p-2">
              {input.input_index}
              </td>
            <td className="p-2 font-mono">
              {amount(input.amount_msat)}
              </td>
            <td className="p-2">
              {input.utxo_evidence}
              </td>
            </tr>)}
          </tbody>
        </table>
      </div>
    <CodeList title={t("entropyPanel.limitations")} values={result.validation.limitations} />
    {!!result.findings.length && <CodeList title={t("findings")} values={result.findings.map(row => row.code)} />}
    <FeatureDetails snapshot={result.features} />
  </article>;
}
