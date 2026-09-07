import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import type {
  AnalysisCase,
  AnalysisQuery,
  AnalysisResult,
} from "@/lib/chainAnalysis";
import {
  analysisNetworkInput,
  shortAnalysisId,
} from "@/lib/chainAnalysis";
import { EvidenceDetails, Fact } from "./EvidenceDetails";
import { WatchAction } from "./WatchControls";
import { CaseComparison } from "./CaseComparison";

export function SavedInvestigations({
  result,
  onLoad,
  onError,
  onNotice,
}: {
  result: AnalysisResult | null;
  onLoad: (saved: AnalysisCase) => void;
  onError: (error: unknown) => void;
  onNotice: (notice: string) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const [title, setTitle] = useState("");
  const [cursor, setCursor] = useState<string>();
  const [previous, setPrevious] = useState<AnalysisCase[]>([]);
  const list = useDaemon<{ items: AnalysisCase[]; next_cursor?: string }>(
    "ui.chain_analysis.cases.list",
    { limit: 20, ...(cursor ? { cursor } : {}) },
  );
  const save = useDaemonMutation<AnalysisCase>("ui.chain_analysis.cases.save", {
    invalidateQueries: false,
  });
  const get = useDaemonMutation<AnalysisCase>("ui.chain_analysis.cases.get", {
    invalidateQueries: false,
  });
  const remove = useDaemonMutation("ui.chain_analysis.cases.delete", {
    invalidateQueries: false,
  });
  const compare = useDaemonMutation<Record<string, unknown>>(
    "ui.chain_analysis.cases.compare",
    { invalidateQueries: false },
  );
  const [comparison, setComparison] = useState<Record<string, unknown> | null>(
    null,
  );
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const busy =
    save.isPending || get.isPending || remove.isPending || compare.isPending;
  const items = [...previous, ...(list.data?.data?.items || [])];
  const refresh = async () => {
    setPrevious([]);
    setCursor(undefined);
    await list.refetch();
  };
  return (
    <div className="px-4 pb-4">
      <form
        className="flex flex-wrap gap-2"
        onSubmit={async (event) => {
          event.preventDefault();
          if (!result || !title.trim() || busy) return;
          try {
            await save.mutateAsync({
              title: title.trim(),
              query: result.query,
              expected_snapshot_id: result.snapshot_id,
            });
            setTitle("");
            await refresh();
            onNotice(t("case.saved"));
          } catch (error) {
            onError(error);
          }
        }}
      >
        <input
          className="ca-input min-w-48 flex-1"
          aria-label={t("case.name")}
          placeholder={t("case.placeholder")}
          value={title}
          maxLength={200}
          onChange={(event) => setTitle(event.target.value)}
        />
        <Button
          type="submit"
          size="sm"
          variant="outline"
          disabled={!result || !title.trim() || busy}
        >
          {t("case.save")}
        </Button>
      </form>
      {list.isError && (
        <p className="mt-3 text-sm text-destructive" role="alert">
          {list.error.message}
        </p>
      )}
      {!items.length && !list.isFetching && (
        <p className="mt-4 text-xs text-muted-foreground">{t("case.empty")}</p>
      )}
      <div className="mt-3 max-h-72 space-y-1 overflow-auto">
        {items.map((item) => (
          <div
            key={item.id}
            className="flex flex-wrap items-center gap-2 border-b py-2 last:border-0"
          >
            <div className="min-w-40 flex-1">
              <p className="break-words text-sm font-medium">{item.title}</p>
              <p className="font-mono text-[10px] text-muted-foreground">
                {item.created_at} · {shortAnalysisId(item.snapshot_id)}
              </p>
            </div>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={async () => {
                try {
                  const response = await get.mutateAsync({ id: item.id });
                  if (response.data) onLoad(response.data);
                } catch (error) {
                  onError(error);
                }
              }}
            >
              {t("case.load")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={async () => {
                try {
                  const response = await compare.mutateAsync({ id: item.id });
                  setComparison(response.data || null);
                } catch (error) {
                  onError(error);
                }
              }}
            >
              {t("case.compare")}
            </Button>
            <WatchAction caseId={item.id} query={item.query} onError={onError} />
            {confirmDelete === item.id ? (
              <div className="flex items-center gap-2 text-xs">
                <span>{t("case.deleteConfirm")}</span>
                <Button
                  size="sm"
                  variant="destructive"
                  disabled={busy}
                  onClick={async () => {
                    try {
                      await remove.mutateAsync({ id: item.id });
                      setConfirmDelete(null);
                      await refresh();
                    } catch (error) {
                      onError(error);
                    }
                  }}
                >
                  {t("case.delete")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setConfirmDelete(null)}
                >
                  {t("dismiss")}
                </Button>
              </div>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => setConfirmDelete(item.id)}
              >
                {t("case.delete")}
              </Button>
            )}
          </div>
        ))}
      </div>
      {list.data?.data?.next_cursor && (
        <Button
          className="mt-2"
          size="sm"
          variant="ghost"
          disabled={list.isFetching}
          onClick={() => {
            setPrevious(items);
            setCursor(list.data?.data?.next_cursor);
          }}
        >
          {t("case.more")}
        </Button>
      )}
      {comparison && <CaseComparison comparison={comparison} />}
    </div>
  );
}

interface AcquisitionPlan {
  plan_id: string;
  args: Record<string, unknown>;
  snapshot_id: string;
  backend: { name: string; kind: string; chain: string; network: string };
  effects: {
    egresses: true;
    max_transactions: number;
    max_requests: number;
    scheduling_deadline_seconds?: number;
    request_timeout_seconds?: number;
    max_response_bytes?: number;
  };
  limitations: unknown[];
}
interface AcquisitionResult {
  acquired_count: number;
  request_count: number;
  complete: boolean;
  frontier: unknown[];
}

interface BookBinding {
  profile_id: string;
  state: "bound" | "unbound";
  environment_id?: string;
  revision?: number;
  chain_instance_id?: string | null;
  domains: Array<{ chain: string; network: string }>;
}
type AcquisitionProps = {
  query: AnalysisQuery;
  onError: (error: unknown) => void;
  onAcquired: () => void;
};

export function AcquisitionPanel(props: AcquisitionProps) {
  const { t } = useTranslation(["settings", "common"]);
  const binding = useDaemon<BookBinding>("ui.networks.binding");
  if (binding.isLoading) return <p className="px-4 pb-4 text-sm">{t("common:state.loading")}</p>;
  const book = binding.data?.data;
  const supported = book?.domains.filter(domain => ["bitcoin", "liquid"].includes(domain.chain)) ?? [];
  if (binding.isError || book?.state !== "bound" || !supported.length) {
    return <div className="space-y-2 px-4 pb-4 text-sm">
      <p role={binding.isError ? "alert" : undefined}>{binding.isError ? binding.error.message : t("bookNetwork.help")}</p>
      <Link className="underline" to="/settings/bitcoin">{t("bookNetwork.title")}</Link>
      {binding.isError && <Button variant="outline" onClick={() => void binding.refetch()}>{t("common:actions.retry")}</Button>}
    </div>;
  }
  return <AcquisitionPanelContent key={`${book.profile_id}:${book.environment_id}:${book.revision}`} {...props} book={book} />;
}

function AcquisitionPanelContent({ query, onError, onAcquired, book }: AcquisitionProps & { book: BookBinding }) {
  const { t } = useTranslation("chainAnalysis");
  const { t: settingsT } = useTranslation("settings");
  const options = useDaemon<{
    backends: Array<{
      name: string;
      kind: string;
      chain?: string;
      network?: string;
      chain_instance_id?: string;
    }>;
  }>("ui.backends.options");
  const [backend, setBackend] = useState("");
  const [subject, setSubject] = useState(query.subject || "");
  const domains = book.domains.filter(domain => ["bitcoin", "liquid"].includes(domain.chain));
  const [chain, setChain] = useState(domains.find(domain => domain.chain === query.chain)?.chain ?? domains[0].chain);
  const domain = domains.find(item => item.chain === chain);
  const network = analysisNetworkInput(domain?.network);
  const needsGenesis = chain === "liquid" && network !== "main";
  const [depth, setDepth] = useState(Math.min(query.depth, 10));
  const [limit, setLimit] = useState(50);
  const [genesis, setGenesis] = useState("");
  const [plan, setPlan] = useState<AcquisitionPlan | null>(null);
  const [plannedInput, setPlannedInput] = useState("");
  const [outcome, setOutcome] = useState<AcquisitionResult | null>(null);
  const preview = useDaemonMutation<AcquisitionPlan>(
    "ui.chain_analysis.acquire.plan",
    { invalidateQueries: false },
  );
  const apply = useDaemonMutation<AcquisitionResult>(
    "ui.chain_analysis.acquire.apply",
  );
  const args = {
    backend,
    subject: subject.trim(),
    chain,
    network,
    direction: query.direction,
    depth,
    max_transactions: limit,
    ...(needsGenesis && genesis.trim() ? { genesis_hash: genesis.trim() } : {}),
  };
  const input = JSON.stringify({ args, environment_id: book.environment_id, revision: book.revision, chain_instance_id: book.chain_instance_id });
  const backends = (options.data?.data?.backends || []).filter((item) =>
    ["bitcoinrpc", "esplora", "liquid-esplora", "electrum"].includes(item.kind)
      && item.chain === chain && !!item.network && analysisNetworkInput(item.network) === network
      && (!item.chain_instance_id || item.chain_instance_id === book.chain_instance_id),
  );
  const busy = preview.isPending || apply.isPending;
  const compatibleBackend = backends.some(item => item.name === backend);
  const ready = !!domain && compatibleBackend && !options.isError && !options.isLoading && !!subject.trim()
    && (!needsGenesis || /^[a-f0-9]{64}$/.test(genesis.trim()));
  return (
    <div className="px-4 pb-4">
      <p className="max-w-3xl text-xs leading-relaxed text-muted-foreground">
        {t("acquire.description")}
      </p>
      <form
        className="mt-3 space-y-3"
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy || !ready) return;
          setPlan(null);
          setOutcome(null);
          try {
            const response = await preview.mutateAsync(args);
            if (response.data) {
              setPlan(response.data);
              setPlannedInput(input);
            }
          } catch (error) {
            onError(error);
          }
        }}
      >
        <fieldset
          disabled={busy}
          className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3"
        >
          <label className="ca-field">
            {t("acquire.backend")}
            <select
              className="ca-select"
              required
              value={backend}
              onChange={(event) => setBackend(event.target.value)}
            >
              <option value="">{t("acquire.select")}</option>
              {backends.map((item) => (
                <option key={item.name} value={item.name}>
                  {item.name} · {item.kind}
                  {item.network ? ` · ${item.network}` : ""}
                </option>
              ))}
            </select>
          </label>
          <label className="ca-field sm:col-span-2">
            {t("acquire.subject")}
            <input
              className="ca-input font-mono"
              required
              value={subject}
              onChange={(event) => setSubject(event.target.value)}
            />
          </label>
          <label className="ca-field">
            {t("chain")}
            <select
              className="ca-select"
              value={chain}
              onChange={(event) =>
                setChain(event.target.value as "bitcoin" | "liquid")
              }
            >
              {domains.map(item => <option key={item.chain} value={item.chain}>{item.chain === "bitcoin" ? "Bitcoin" : "Liquid"}</option>)}
            </select>
          </label>
          <div className="ca-field">
            <Fact label={t("network")} value={domain?.network} />
            <Link className="text-xs underline" to="/settings/bitcoin">{settingsT("bookNetwork.title")}</Link>
            {book.chain_instance_id && <Fact label={settingsT("bookNetwork.instance")} value={book.chain_instance_id} />}
          </div>
          <div className="grid grid-cols-2 gap-2">
            <label className="ca-field">
              {t("acquire.depth")}
              <input
                className="ca-input"
                type="number"
                min={1}
                max={10}
                required
                value={depth}
                onChange={(event) => setDepth(Number(event.target.value))}
              />
            </label>
            <label className="ca-field">
              {t("acquire.limit")}
              <input
                className="ca-input"
                type="number"
                min={1}
                max={200}
                required
                value={limit}
                onChange={(event) => setLimit(Number(event.target.value))}
              />
            </label>
          </div>
          {needsGenesis && <label className="ca-field sm:col-span-2 lg:col-span-3">
            {t("acquire.genesis")}
            <input
              className="ca-input font-mono"
              value={genesis}
              required
              pattern="[a-fA-F0-9]{64}"
              maxLength={64}
              onChange={(event) => setGenesis(event.target.value.toLowerCase())}
            />
          </label>}
        </fieldset>
        {options.isError && (
          <p className="text-xs text-destructive" role="alert">
            {options.error.message}
          </p>
        )}
        {!backends.length && !options.isFetching && (
          <p className="text-xs text-muted-foreground">{t("acquire.none")} <Link className="underline" to={chain === "liquid" ? "/settings/liquid" : "/settings/bitcoin"}>{t("acquire.backend")}</Link></p>
        )}
        <Button
          type="submit"
          variant="outline"
          size="sm"
          disabled={busy || !ready}
        >
          {t("acquire.preview")}
        </Button>
      </form>
      {plan && (
        <div className="mt-4 space-y-3 rounded-md border border-amber-500/40 bg-amber-500/5 p-3">
          <dl className="grid grid-cols-3 gap-3">
            <Fact label={t("acquire.backend")} value={plan.backend.name} />
            <Fact
              label={t("acquire.transactions")}
              value={plan.effects.max_transactions}
            />
            <Fact
              label={t("acquire.requests")}
              value={plan.effects.max_requests}
            />
          </dl>
          <dl className="grid grid-cols-3 gap-3">
            <Fact
              label={t("acquire.deadline")}
              value={plan.effects.scheduling_deadline_seconds}
            />
            <Fact
              label={t("acquire.timeout")}
              value={plan.effects.request_timeout_seconds}
            />
            <Fact
              label={t("acquire.responseCap")}
              value={plan.effects.max_response_bytes}
            />
          </dl>
          <EvidenceDetails
            value={plan.limitations}
            label={t("acquire.limitations")}
          />
          {input !== plannedInput && (
            <p className="text-xs">{t("acquire.planChanged")}</p>
          )}
          <Button
            size="sm"
            disabled={busy || !ready || input !== plannedInput}
            onClick={async () => {
              if (busy || !ready || input !== plannedInput) return;
              try {
                const response = await apply.mutateAsync({ plan });
                setOutcome(response.data || null);
                setPlan(null);
                onAcquired();
              } catch (error) {
                onError(error);
                setPlan(null);
              }
            }}
          >
            {t("acquire.apply")}
          </Button>
        </div>
      )}
      {outcome && (
        <div className="mt-4 text-xs" role="status">
          <p>
            {t("acquire.finished", {
              count: outcome.acquired_count,
              requests: outcome.request_count,
            })}
          </p>
          {!outcome.complete && (
            <p className="mt-1">{t("acquire.incomplete")}</p>
          )}
          <EvidenceDetails value={outcome.frontier} label={t("frontier")} />
        </div>
      )}
    </div>
  );
}

export { EntropyPanel, EntropyResult } from "./EntropyPanel";
