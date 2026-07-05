import { useState, type ReactNode } from "react";
import {
  BadgeCheck,
  ChevronDown,
  FileSearch,
  Layers,
  ListChecks,
  Loader2,
  Network,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";

import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import {
  HeuristicCoverage,
  LinkageGraph,
  PrivacyFindingCard,
  PrivacyScoreHero,
  ScoreWaterfall,
  SeverityRing,
} from "@/components/privacy/PrivacyScore";
import {
  TransactionGraphPanel,
  type TransactionGraphPayload,
} from "@/components/transactions/TransactionGraphTab";
import { transactionGraphLookupReferenceArgs } from "@/components/transactions/TransactionDetailsTab";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
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
  privacySeverity,
  privacySeverityTone,
  shortPrivacyId as shortId,
  transactionRowSeverity,
  type EvidenceLevel,
  type PrivacyMirrorPayload,
  type PsbtPrivacyResult,
} from "@/lib/privacyMirror";
import { heuristicComputedCount, privacyScoreModel } from "@/lib/privacyScore";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

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

export function SeverityMark({
  severity,
  dotOnly = false,
}: {
  severity?: string | null;
  dotOnly?: boolean;
}) {
  const { t } = useTranslation("privacyMirror");
  const key = privacySeverity(severity);
  const tone = privacySeverityTone(key);
  if (dotOnly) {
    return (
      <span
        className={cn("inline-block size-2 shrink-0 rounded-full", tone.dot)}
        aria-label={t(`severity.${key}`)}
      />
    );
  }
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wide",
        tone.text,
        tone.bg,
      )}
    >
      <span className={cn("size-1.5 rounded-full", tone.dot)} aria-hidden="true" />
      {t(`severity.${key}`)}
    </span>
  );
}

function Section({
  title,
  icon,
  count,
  defaultOpen = false,
  testId,
  children,
}: {
  title: string;
  icon?: ReactNode;
  count?: ReactNode;
  defaultOpen?: boolean;
  testId?: string;
  children: ReactNode;
}) {
  return (
    <Collapsible
      defaultOpen={defaultOpen}
      data-testid={testId}
      className="rounded-md border bg-background"
    >
      <CollapsibleTrigger className="group flex w-full items-center justify-between gap-3 border-b px-4 py-3 text-left data-[state=closed]:border-b-0 hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <span className="flex min-w-0 items-center gap-2">
          {icon}
          <span className="truncate text-sm font-semibold">{title}</span>
        </span>
        <span className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
          {count != null ? (
            <span className="font-mono tabular-nums">{count}</span>
          ) : null}
          <ChevronDown
            className="size-4 transition-transform group-data-[state=open]:rotate-180"
            aria-hidden="true"
          />
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent className="p-4">{children}</CollapsibleContent>
    </Collapsible>
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
        <div
          key={finding.id || finding.title}
          className={cn(
            "rounded-md border border-l-2 bg-muted/20 p-3",
            privacySeverityTone(privacySeverity(finding.severity)).stripe,
          )}
        >
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <SeverityMark severity={finding.severity} dotOnly />
                <p className="truncate text-sm font-medium">{finding.title || finding.id}</p>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">{finding.detail}</p>
            </div>
            <EvidenceBadge level={finding.evidence_level} />
          </div>
        </div>
      ))}
    </div>
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
              {columns.map((column, index) => (
                <TableHead key={column || `col-${index}`}>{column}</TableHead>
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

// Lazy, LOCAL-ONLY drill-in reusing the transaction flow diagram.
function TransactionFlowSheet({
  txid,
  onClose,
}: {
  txid: string;
  onClose: () => void;
}) {
  const { t } = useTranslation("privacyMirror");
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const query = useDaemon<TransactionGraphPayload>(
    "ui.transactions.graph",
    transactionGraphLookupReferenceArgs(txid),
  );
  return (
    <Sheet open onOpenChange={(open) => (!open ? onClose() : undefined)}>
      <SheetContent side="right" className="w-full gap-0 sm:max-w-2xl md:max-w-3xl">
        <SheetHeader className="border-b">
          <SheetTitle>{t("flow.title")}</SheetTitle>
          <SheetDescription>{t("flow.subtitle")}</SheetDescription>
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          <TransactionGraphPanel
            graph={query.data?.data}
            loading={query.isLoading}
            error={query.error instanceof Error ? query.error.message : null}
            hideSensitive={hideSensitive}
          />
        </div>
      </SheetContent>
    </Sheet>
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
            level={result?.summary?.unknown_input_count ? "unknown" : undefined}
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
                <span className="truncate">
                  {t(`psbt.scenario.${item.scenario}`, { defaultValue: item.scenario ?? "" })}
                </span>
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
  );
}

export function PrivacyMirrorPayloadView({
  payload,
  onRefresh,
  refreshing = false,
  onNavigate,
}: {
  payload: PrivacyMirrorPayload;
  onRefresh?: () => void;
  refreshing?: boolean;
  onNavigate?: (to: string) => void;
}) {
  const { t } = useTranslation("privacyMirror");
  const [flowTxid, setFlowTxid] = useState<string | null>(null);

  // Map a finding to where the user would act on it, so advice leads somewhere.
  const findingAction = (kind: string) => {
    if (
      kind === "source_proximity_coverage_gaps" ||
      kind === "unknown_provenance" ||
      kind === "coverage_degraded"
    ) {
      return onNavigate
        ? { label: t("action.reviewOrigins"), icon: <BadgeCheck className="size-3.5" aria-hidden="true" />, onClick: () => onNavigate("/source-of-funds") }
        : undefined;
    }
    return undefined;
  };
  const summary = payload.summary ?? {};
  const worst = summary.worst_risk;
  const worstSeverity = privacySeverity(worst?.severity);
  const model = privacyScoreModel(payload);
  const adversaries = payload.adversary_cards ?? [];
  const wallets = payload.wallet_view ?? [];
  const transactions = payload.transaction_view ?? [];
  const utxos = payload.utxo_view ?? [];
  const timeline = payload.timeline ?? [];
  const evidence = payload.evidence_drilldowns ?? [];
  const coverage = payload.coverage ?? {};

  const tellLabel = (kind?: string) =>
    kind ? t(`tellKind.${kind}`, { defaultValue: kind.replace(/_/g, " ") }) : "-";

  const notNominal =
    payload.local_only === false ||
    payload.advisory_only === false ||
    Boolean(coverage.degraded);

  return (
    <div data-testid="privacy-mirror-page" className={screenShellClassName}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">{t("title")}</h1>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">{t("subtitle")}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <EvidenceBadge level={summary.evidence_level} />
          {notNominal ? (
            <Badge
              variant="outline"
              className="rounded-md border-amber-500/40 text-amber-700 dark:text-amber-300"
            >
              {t("guardrail.degraded")}
            </Badge>
          ) : null}
          {onRefresh ? (
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              aria-label={t("refresh")}
              onClick={onRefresh}
              disabled={refreshing}
            >
              <RefreshCw className={cn("size-4", refreshing && "animate-spin")} aria-hidden="true" />
            </Button>
          ) : null}
        </div>
      </div>

      {/* Score hero + severity ring + waterfall. */}
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1.35fr)_minmax(260px,1fr)]">
        <PrivacyScoreHero model={model} />
        <div className="grid gap-3">
          <SeverityRing census={model.census} />
          <ScoreWaterfall
            factors={model.factors}
            score={model.score}
            base={model.base}
            coverageRatio={model.coverageRatio}
          />
        </div>
      </div>

      {/* Primary recommendation — the worst risk, what to fix first. */}
      <section
        data-testid="privacy-mirror-worst-risk"
        className={cn("rounded-md border border-l-2 bg-background", privacySeverityTone(worstSeverity).stripe)}
      >
        <div className="flex items-center justify-between gap-3 border-b px-4 py-3">
          <div className="flex min-w-0 items-center gap-2">
            <ShieldAlert
              className={cn("size-4 shrink-0", privacySeverityTone(worstSeverity).text)}
              aria-hidden="true"
            />
            <h2 className="truncate text-sm font-semibold">{t("primary.title")}</h2>
          </div>
          <SeverityMark severity={worst?.severity} />
        </div>
        <div className="flex flex-col gap-3 p-4 md:flex-row md:items-start md:justify-between">
          <div className="min-w-0">
            <p className="text-base font-medium">
              {worst?.kind
                ? t(`worstKind.${worst.kind}`, { defaultValue: worst.title || t("worst.fallback") })
                : worst?.title || t("worst.fallback")}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {worst?.kind
                ? t(`reco.${worst.kind}`, { defaultValue: worst.answer || t("worst.empty") })
                : worst?.answer || t("worst.empty")}
            </p>
          </div>
          <EvidenceBadge level={worst?.evidence_level} />
        </div>
      </section>

      {/* Ranked, severity-colored finding cards. */}
      <section className="space-y-2" data-testid="privacy-mirror-findings">
        <h2 className="text-sm font-semibold">
          {t("question.findings")}{" "}
          <span className="font-mono text-muted-foreground">({model.findings.length})</span>
        </h2>
        {model.findings.length ? (
          <div className="grid gap-2">
            {model.findings.map((finding) => (
              <PrivacyFindingCard
                key={finding.id}
                finding={finding}
                onViewFlow={setFlowTxid}
                action={findingAction(finding.kind)}
              />
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">{t("finding.empty")}</p>
        )}
      </section>

      {/* Pre-broadcast check — prevent a leak before you spend. Elevated, not buried. */}
      <section className="rounded-md border border-l-2 border-l-sky-500 bg-background" data-testid="privacy-mirror-psbt">
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <ShieldAlert className="size-4 shrink-0 text-sky-500" aria-hidden="true" />
          <div className="min-w-0">
            <h2 className="text-sm font-semibold">{t("section.psbt")}</h2>
            <p className="text-xs text-muted-foreground">{t("psbt.intent")}</p>
          </div>
        </div>
        <div className="p-4">
          <PrivacyMirrorPsbtPanel />
        </div>
      </section>

      {/* Linkage map. */}
      <Section
        title={t("linkage.title")}
        icon={<Network className="size-4 shrink-0" aria-hidden="true" />}
        count={fmtInt(wallets.length)}
        defaultOpen
      >
        <LinkageGraph wallets={wallets} evidenceLevel={summary.evidence_level} />
      </Section>

      {/* Who can infer it. */}
      <Section
        title={t("question.infer")}
        icon={<Network className="size-4 shrink-0" aria-hidden="true" />}
        count={fmtInt(adversaries.length)}
      >
        <div className="grid gap-3 lg:grid-cols-3">
          {adversaries.map((card) => (
            <div key={card.tier} className="rounded-md border bg-background">
              <div className="flex items-center gap-2 border-b px-4 py-3">
                <h3 className="text-sm font-semibold break-words">
                  {t(`adversaryTier.${card.tier}`, {
                    defaultValue: card.label || card.tier || t("adversary.unknown"),
                  })}
                </h3>
              </div>
              <div className="grid gap-3 p-4">
                <div className="grid grid-cols-2 gap-2">
                  <Metric label={t("adversary.clusters")} value={fmtInt(card.summary?.exposed_cluster_count)} level={card.evidence_level} />
                  <Metric label={t("adversary.wallets")} value={fmtInt(card.summary?.wallet_count)} level={card.evidence_level} />
                </div>
                <div className="space-y-2">
                  {(card.model_assumptions ?? []).slice(0, 3).map((assumption) => (
                    <div key={assumption.code} className="rounded-md border bg-muted/20 p-2 text-xs">
                      <div className="flex items-start justify-between gap-2">
                        <span>
                          {t(`assumption.${assumption.code}`, {
                            defaultValue: assumption.statement || assumption.code || "",
                          })}
                        </span>
                        <EvidenceBadge level={assumption.evidence_level} />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          ))}
        </div>
      </Section>

      {/* The evidence / why. */}
      <Section
        title={t("question.evidence")}
        icon={<FileSearch className="size-4 shrink-0" aria-hidden="true" />}
        count={fmtInt(evidence.length + timeline.length)}
      >
        <div className="space-y-4">
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("section.evidenceItems")}
            </p>
            <PrivacyTable
              columns={[t("table.section"), t("table.item"), t("table.kind"), t("table.evidence")]}
              rows={evidence.map((row) => [
                row.section || "-",
                shortId(row.id),
                tellLabel(row.kind),
                <EvidenceBadge key="e" level={row.evidence_level} />,
              ])}
            />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("section.timeline")}
            </p>
            <PrivacyTable
              columns={[t("table.event"), t("table.category"), t("table.transaction"), t("table.detail"), t("table.evidence")]}
              rows={timeline.map((row) => [
                tellLabel(row.kind || row.id || undefined),
                row.category || "-",
                shortId(row.txid),
                row.detail ? tellLabel(row.detail) : row.new_linkage ? t("timeline.newLinkage") : "-",
                <EvidenceBadge key="e" level={row.evidence_level} />,
              ])}
            />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("section.coverage")}
            </p>
            <div className="grid gap-2 rounded-md border bg-muted/20 p-3">
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
          </div>
        </div>
      </Section>

      {/* Raw records — full tabular data. */}
      <Section
        title={t("section.records")}
        icon={<Layers className="size-4 shrink-0" aria-hidden="true" />}
        count={fmtInt(wallets.length + transactions.length + utxos.length)}
      >
        <div className="space-y-4">
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("section.wallets")}
            </p>
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
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("section.transactionTells")}
            </p>
            <PrivacyTable
              columns={[t("table.transaction"), t("table.tells"), t("table.kinds"), t("table.evidence"), ""]}
              rows={transactions.map((row) => [
                <span key="tx" className="flex items-center gap-2">
                  <SeverityMark severity={transactionRowSeverity(row)} dotOnly />
                  {shortId(row.txid)}
                </span>,
                fmtInt(row.tell_count),
                (row.tell_kinds ?? []).map((kind) => tellLabel(kind)).join(", ") || "-",
                <EvidenceBadge key="e" level={row.evidence_level} />,
                row.txid ? (
                  <Button
                    key="flow"
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="h-7 gap-1 px-2 text-xs"
                    onClick={() => setFlowTxid(row.txid ?? null)}
                  >
                    <Network className="size-3.5" aria-hidden="true" />
                    {t("flow.view")}
                  </Button>
                ) : (
                  "-"
                ),
              ])}
            />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("section.coins")}
            </p>
            <PrivacyTable
              columns={[t("table.coin"), t("table.wallet"), t("table.amount"), t("table.role"), t("table.proximity"), t("table.evidence")]}
              rows={utxos.map((row) => [
                shortId(row.coin_id),
                shortId(row.wallet_id),
                fmtMsat(row.amount_msat),
                row.branch_role || "-",
                tellLabel(row.source_proximity),
                <EvidenceBadge key="e" level={row.evidence_level} />,
              ])}
            />
          </div>
        </div>
      </Section>

      {/* Heuristics checked — mirrors the am-i-exposed catalog with honest coverage. */}
      <Section
        title={t("heuristics.title")}
        icon={<ListChecks className="size-4 shrink-0" aria-hidden="true" />}
        count={`${heuristicComputedCount()}/34`}
      >
        <HeuristicCoverage />
      </Section>

      {flowTxid ? (
        <TransactionFlowSheet txid={flowTxid} onClose={() => setFlowTxid(null)} />
      ) : null}
    </div>
  );
}

export function PrivacyMirror() {
  const { t } = useTranslation("privacyMirror");
  const navigate = useNavigate();
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
    <PrivacyMirrorPayloadView
      payload={payload}
      onRefresh={() => void query.refetch()}
      refreshing={query.isFetching}
      onNavigate={(to) => void navigate({ to } as never)}
    />
  );
}
