import { useState, type ReactNode } from "react";
import {
  AlertTriangle,
  Eye,
  FileSearch,
  Loader2,
  Network,
  Radar,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import {
  formatPrivacyInt as fmtInt,
  formatPrivacyMsat as fmtMsat,
  privacyEvidenceTone as evidenceTone,
  shortPrivacyId as shortId,
  type EvidenceLevel,
  type PrivacyMirrorPayload,
  type PsbtPrivacyResult,
  type UnknownRow,
} from "@/lib/privacyMirror";
import { cn } from "@/lib/utils";

export function EvidenceBadge({ level }: { level?: EvidenceLevel }) {
  const { t } = useTranslation("privacyMirror");
  const key = level || "unknown";
  const label =
    key === "exact"
      ? t("evidence.exact")
      : key === "derived"
        ? t("evidence.derived")
        : key === "unknown"
          ? t("evidence.unknown")
          : key;
  return (
    <Badge variant="outline" className={cn("rounded-md", evidenceTone(key))}>
      {label}
    </Badge>
  );
}

function Panel({
  title,
  icon,
  children,
  action,
  testId,
}: {
  title: string;
  icon?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  testId?: string;
}) {
  return (
    <section data-testid={testId} className="rounded-md border bg-background">
      <div className="flex items-center justify-between gap-3 border-b px-4 py-3">
        <div className="flex min-w-0 items-center gap-2">
          {icon}
          <h2 className="truncate text-sm font-semibold">{title}</h2>
        </div>
        {action}
      </div>
      <div className="p-4">{children}</div>
    </section>
  );
}

function Metric({ label, value, level }: { label: string; value: ReactNode; level?: EvidenceLevel }) {
  return (
    <div className="rounded-md border bg-muted/20 p-3">
      <div className="flex items-start justify-between gap-2">
        <p className="text-xs text-muted-foreground">{label}</p>
        {level ? <EvidenceBadge level={level} /> : null}
      </div>
      <p className="mt-2 font-mono text-xl tabular-nums">{value}</p>
    </div>
  );
}

function FindingList({ findings }: { findings?: Array<{ id?: string; title?: string; detail?: string; evidence_level?: EvidenceLevel; severity?: string }> }) {
  const rows = findings ?? [];
  if (!rows.length) return null;
  return (
    <div className="grid gap-2">
      {rows.map((finding) => (
        <div key={finding.id || finding.title} className="rounded-md border bg-muted/20 p-3">
          <div className="flex items-start justify-between gap-2">
            <div>
              <p className="text-sm font-medium">{finding.title || finding.id}</p>
              <p className="mt-1 text-xs text-muted-foreground">{finding.detail}</p>
            </div>
            <EvidenceBadge level={finding.evidence_level} />
          </div>
        </div>
      ))}
    </div>
  );
}

function PrivacyMirrorPsbtPanel() {
  const { t } = useTranslation("privacyMirror");
  const [psbt, setPsbt] = useState("");
  const [result, setResult] = useState<PsbtPrivacyResult | null>(null);
  const mutation = useDaemonMutation<PsbtPrivacyResult>("ui.reports.psbt_privacy");
  const canRun = psbt.trim().length > 0 && !mutation.isPending;

  async function runAnalysis() {
    const envelope = await mutation.mutateAsync({ psbt });
    setResult(envelope.data ?? null);
  }

  return (
    <Panel title={t("psbt.title")} icon={<FileSearch className="size-4" aria-hidden="true" />}>
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.7fr)]">
        <div className="space-y-2">
          <Textarea
            value={psbt}
            onChange={(event) => setPsbt(event.target.value)}
            rows={7}
            spellCheck={false}
            placeholder={t("psbt.placeholder")}
            aria-label={t("psbt.inputAria")}
            className="font-mono text-xs"
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button type="button" onClick={() => void runAnalysis()} disabled={!canRun}>
              {mutation.isPending ? <Loader2 className="mr-2 size-4 animate-spin" aria-hidden="true" /> : null}
              {t("psbt.analyze")}
            </Button>
            {mutation.error ? (
              <p className="text-sm text-destructive">
                {mutation.error instanceof Error ? mutation.error.message : String(mutation.error)}
              </p>
            ) : null}
          </div>
        </div>
        <div className="space-y-3">
          <div className="grid grid-cols-3 gap-2">
            <Metric
              label={t("psbt.metric.merge")}
              value={fmtInt(result?.summary?.cluster_merge_delta)}
              level={result?.summary?.evidence_level}
            />
            <Metric
              label={t("psbt.metric.unknown")}
              value={fmtInt(result?.summary?.unknown_input_count)}
              level={result?.summary?.unknown_input_count ? "unknown" : "derived"}
            />
            <Metric
              label={t("psbt.metric.blast")}
              value={fmtInt(result?.summary?.blast_radius_score)}
              level={result?.summary?.evidence_level}
            />
          </div>
          <FindingList findings={result?.findings} />
          <div className="rounded-md border bg-muted/20 p-3">
            <p className="text-xs font-medium text-muted-foreground">{t("psbt.whatIf")}</p>
            <div className="mt-2 grid gap-1.5">
              {(result?.what_if ?? []).map((item) => (
                <div key={item.scenario} className="flex items-center justify-between gap-2 text-sm">
                  <span className="truncate">{item.scenario}</span>
                  <span className="font-mono tabular-nums">{item.cluster_merge_delta ?? 0}</span>
                </div>
              ))}
              {!result?.what_if?.length ? (
                <p className="text-sm text-muted-foreground">{t("psbt.empty")}</p>
              ) : null}
            </div>
          </div>
        </div>
      </div>
    </Panel>
  );
}

export function PrivacyMirrorPayloadView({ payload }: { payload: PrivacyMirrorPayload }) {
  const { t } = useTranslation("privacyMirror");
  const summary = payload.summary ?? {};
  const worst = summary.worst_risk;
  const adversaries = payload.adversary_cards ?? [];
  const wallets = payload.wallet_view ?? [];
  const transactions = payload.transaction_view ?? [];
  const utxos = payload.utxo_view ?? [];
  const timeline = payload.timeline ?? [];
  const unknowns = payload.unknowns ?? [];
  const evidence = payload.evidence_drilldowns ?? [];
  const coverage = payload.coverage ?? {};

  return (
    <div data-testid="privacy-mirror-page" className={screenShellClassName}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">{t("title")}</h1>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">{t("subtitle")}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <EvidenceBadge level={summary.evidence_level} />
          <Badge variant="outline" className="rounded-md">
            {payload.local_only ? t("guardrail.local") : t("guardrail.degraded")}
          </Badge>
          <Badge variant="outline" className="rounded-md">
            {payload.advisory_only ? t("guardrail.advisory") : t("guardrail.degraded")}
          </Badge>
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
        <Metric label={t("metric.linkageScore")} value={fmtInt(summary.linkage_score)} level={summary.evidence_level} />
        <Metric label={t("metric.clusters")} value={fmtInt(summary.linkable_cluster_count)} level={summary.evidence_level} />
        <Metric label={t("metric.adversaries")} value={fmtInt(summary.adversary_view_count)} level={summary.evidence_level} />
        <Metric label={t("metric.utxos")} value={fmtInt(summary.utxo_count)} level={summary.evidence_level} />
        <Metric label={t("metric.unknowns")} value={fmtInt(summary.unknown_count)} level={summary.unknown_count ? "unknown" : "derived"} />
      </div>

      <Panel
        title={t("worst.title")}
        icon={<ShieldAlert className="size-4 text-amber-600" aria-hidden="true" />}
        testId="privacy-mirror-worst-risk"
      >
        <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
          <div>
            <p className="text-base font-medium">{worst?.title || t("worst.fallback")}</p>
            <p className="mt-1 text-sm text-muted-foreground">{worst?.answer || t("worst.empty")}</p>
          </div>
          <EvidenceBadge level={worst?.evidence_level} />
        </div>
      </Panel>

      <div data-testid="privacy-mirror-mobile-stack" className="grid gap-3 md:hidden">
        {adversaries.slice(0, 3).map((card) => (
          <Panel key={card.tier} title={card.label || card.tier || t("adversary.unknown")}>
            <div className="grid grid-cols-2 gap-2">
              <Metric label={t("adversary.clusters")} value={fmtInt(card.summary?.exposed_cluster_count)} level={card.evidence_level} />
              <Metric label={t("adversary.wallets")} value={fmtInt(card.summary?.wallet_count)} level={card.evidence_level} />
            </div>
          </Panel>
        ))}
      </div>

      <Tabs defaultValue="overview" className="space-y-3">
        <div className="overflow-x-auto">
          <TabsList className="inline-grid min-w-max grid-cols-8">
            <TabsTrigger value="overview">{t("tab.overview")}</TabsTrigger>
            <TabsTrigger value="adversaries">{t("tab.adversaries")}</TabsTrigger>
            <TabsTrigger value="wallets">{t("tab.wallets")}</TabsTrigger>
            <TabsTrigger value="transactions">{t("tab.transactions")}</TabsTrigger>
            <TabsTrigger value="utxos">{t("tab.utxos")}</TabsTrigger>
            <TabsTrigger value="timeline">{t("tab.timeline")}</TabsTrigger>
            <TabsTrigger value="evidence">{t("tab.evidence")}</TabsTrigger>
            <TabsTrigger value="psbt">{t("tab.psbt")}</TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="overview" className="space-y-3">
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(320px,0.6fr)]">
            <Panel title={t("exposure.title")} icon={<Eye className="size-4" aria-hidden="true" />}>
              <div className="grid gap-3 sm:grid-cols-3">
                <Metric label={t("metric.wallets")} value={fmtInt(summary.wallet_count)} level={summary.evidence_level} />
                <Metric label={t("metric.transactionTells")} value={fmtInt(summary.transaction_tell_count)} level={summary.evidence_level} />
                <Metric label={t("metric.findings")} value={fmtInt(summary.finding_count)} level={summary.evidence_level} />
              </div>
            </Panel>
            <Panel title={t("coverage.title")} icon={<Radar className="size-4" aria-hidden="true" />}>
              <div className="grid gap-2">
                <div className="flex items-center justify-between gap-2 text-sm">
                  <span>{t("coverage.known")}</span>
                  <span className="font-mono tabular-nums">{fmtInt(coverage.source_proximity_known_coin_count)}</span>
                </div>
                <div className="flex items-center justify-between gap-2 text-sm">
                  <span>{t("coverage.unknown")}</span>
                  <span className="font-mono tabular-nums">{fmtInt(coverage.source_proximity_unknown_coin_count)}</span>
                </div>
                <div className="flex items-center justify-between gap-2 text-sm">
                  <span>{t("coverage.degraded")}</span>
                  <span>{coverage.degraded ? t("yes") : t("bounded")}</span>
                </div>
              </div>
            </Panel>
          </div>
          <UnknownsPanel unknowns={unknowns} />
        </TabsContent>

        <TabsContent value="adversaries">
          <div className="grid gap-3 lg:grid-cols-3">
            {adversaries.map((card) => (
              <Panel key={card.tier} title={card.label || card.tier || t("adversary.unknown")} icon={<Network className="size-4" aria-hidden="true" />}>
                <div className="grid gap-3">
                  <div className="grid grid-cols-2 gap-2">
                    <Metric label={t("adversary.clusters")} value={fmtInt(card.summary?.exposed_cluster_count)} level={card.evidence_level} />
                    <Metric label={t("adversary.wallets")} value={fmtInt(card.summary?.wallet_count)} level={card.evidence_level} />
                  </div>
                  <div className="space-y-2">
                    {(card.model_assumptions ?? []).slice(0, 3).map((assumption) => (
                      <div key={assumption.code} className="rounded-md border bg-muted/20 p-2 text-xs">
                        <div className="flex items-start justify-between gap-2">
                          <span>{assumption.statement || assumption.code}</span>
                          <EvidenceBadge level={assumption.evidence_level} />
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              </Panel>
            ))}
          </div>
        </TabsContent>

        <TabsContent value="wallets">
          <PrivacyTable
            columns={[t("table.wallet"), t("table.coins"), t("table.amount"), t("table.edges"), t("table.evidence")]}
            rows={wallets.map((row) => [
              shortId(row.wallet_id),
              fmtInt(row.coin_count),
              fmtMsat(row.amount_msat),
              fmtInt(row.linkage_edge_count),
              <EvidenceBadge key="e" level={row.evidence_level} />,
            ])}
          />
        </TabsContent>

        <TabsContent value="transactions">
          <PrivacyTable
            columns={[t("table.transaction"), t("table.tells"), t("table.kinds"), t("table.penalties"), t("table.evidence")]}
            rows={transactions.map((row) => [
              shortId(row.txid),
              fmtInt(row.tell_count),
              (row.tell_kinds ?? []).join(", ") || "-",
              fmtInt(row.wallet_penalty_count),
              <EvidenceBadge key="e" level={row.evidence_level} />,
            ])}
          />
        </TabsContent>

        <TabsContent value="utxos">
          <PrivacyTable
            columns={[t("table.coin"), t("table.wallet"), t("table.amount"), t("table.role"), t("table.proximity"), t("table.evidence")]}
            rows={utxos.map((row) => [
              shortId(row.coin_id),
              shortId(row.wallet_id),
              fmtMsat(row.amount_msat),
              row.branch_role || "-",
              row.source_proximity || "-",
              <EvidenceBadge key="e" level={row.evidence_level} />,
            ])}
          />
        </TabsContent>

        <TabsContent value="timeline">
          <PrivacyTable
            columns={[t("table.event"), t("table.category"), t("table.transaction"), t("table.detail"), t("table.evidence")]}
            rows={timeline.map((row) => [
              row.kind || row.id || "-",
              row.category || "-",
              shortId(row.txid),
              row.detail || (row.new_linkage ? t("timeline.newLinkage") : "-"),
              <EvidenceBadge key="e" level={row.evidence_level} />,
            ])}
          />
        </TabsContent>

        <TabsContent value="evidence" className="space-y-3">
          <UnknownsPanel unknowns={unknowns} />
          <PrivacyTable
            columns={[t("table.section"), t("table.item"), t("table.kind"), t("table.evidence")]}
            rows={evidence.map((row) => [
              row.section || "-",
              shortId(row.id),
              row.kind || "-",
              <EvidenceBadge key="e" level={row.evidence_level} />,
            ])}
          />
        </TabsContent>

        <TabsContent value="psbt">
          <PrivacyMirrorPsbtPanel />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function UnknownsPanel({ unknowns }: { unknowns: UnknownRow[] }) {
  const { t } = useTranslation("privacyMirror");
  return (
    <Panel title={t("unknowns.title")} icon={<AlertTriangle className="size-4 text-amber-600" aria-hidden="true" />}>
      {unknowns.length ? (
        <div className="grid gap-2">
          {unknowns.map((row, index) => (
            <div key={`${row.source}-${row.code}-${index}`} className="rounded-md border bg-muted/20 p-3">
              <div className="flex items-start justify-between gap-2">
                <div>
                  <p className="text-sm font-medium">{row.code || row.source}</p>
                  <p className="mt-1 text-xs text-muted-foreground">{row.title || row.message}</p>
                </div>
                <EvidenceBadge level={row.evidence_level} />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{t("unknowns.empty")}</p>
      )}
    </Panel>
  );
}

function PrivacyTable({ columns, rows }: { columns: string[]; rows: ReactNode[][] }) {
  const { t } = useTranslation("privacyMirror");
  return (
    <div className="overflow-hidden rounded-md border bg-background">
      <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              {columns.map((column) => (
                <TableHead key={column}>{column}</TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.length ? (
              rows.map((row, rowIndex) => (
                <TableRow key={rowIndex}>
                  {row.map((cell, cellIndex) => (
                    <TableCell key={cellIndex} className="max-w-[260px] truncate">
                      {cell}
                    </TableCell>
                  ))}
                </TableRow>
              ))
            ) : (
              <TableRow>
                <TableCell colSpan={columns.length} className="text-muted-foreground">
                  {t("table.empty")}
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

export function PrivacyMirror() {
  const { t } = useTranslation("privacyMirror");
  const query = useDaemon<PrivacyMirrorPayload>("ui.reports.privacy_mirror", undefined, {
    refetchOnMount: "always",
  });
  const payload = query.data?.data;

  if (query.isLoading && !payload) {
    return <ScreenSkeleton titleWidth="w-48" />;
  }

  if (query.isError && !payload) {
    return (
      <ScreenNotice
        title={t("unavailable.title")}
        body={query.error instanceof Error ? query.error.message : t("unavailable.body")}
      />
    );
  }

  if (!payload) {
    return <ScreenNotice title={t("unavailable.title")} body={t("unavailable.body")} />;
  }

  return (
    <div className="relative">
      <div className="absolute right-3 top-3 z-10">
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={t("refresh")}
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
        >
          <RefreshCw className={cn("size-4", query.isFetching && "animate-spin")} aria-hidden="true" />
        </Button>
      </div>
      <PrivacyMirrorPayloadView payload={payload} />
    </div>
  );
}
