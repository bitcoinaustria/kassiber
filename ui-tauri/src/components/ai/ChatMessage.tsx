/**
 * One message in the assistant thread.
 *
 * Distinguishes user vs assistant visually but keeps things minimal —
 * matching the cosmetic style of the existing Ai02 input bar. Streams the
 * assistant content live with a typing indicator until any visible-answer
 * text lands.
 */

import { Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { ChatLoader } from "./ChatLoader";
import { ChatMarkdown } from "./ChatMarkdown";
import { ChatReasoning } from "./ChatReasoning";
import { ChatToolCall } from "./ChatToolCall";
import {
  ChainOfThought,
  ChainOfThoughtContent,
  ChainOfThoughtHeader,
} from "@/components/ai-elements";
import type { AiChatMessage } from "@/daemon/stream";
import { cn } from "@/lib/utils";

interface ChatMessageProps {
  message: AiChatMessage;
}

export function ChatMessage({ message }: ChatMessageProps) {
  const { t } = useTranslation("assistant");
  if (message.role === "user") {
    return (
      <div className="flex w-full justify-end">
        <div className="max-w-[82%] rounded-2xl rounded-tr-sm bg-primary px-3 py-2 text-sm text-primary-foreground shadow-sm sm:max-w-[72%]">
          <p className="whitespace-pre-wrap break-words">{message.content}</p>
        </div>
      </div>
    );
  }

  const isStreaming =
    message.status === "streaming" || message.status === "pending";
  const hasAnswer = Boolean(message.content);
  const hasToolCalls = Boolean(message.toolCalls?.length);
  const showLoader =
    !hasAnswer &&
    (message.status === "pending" || message.status === "streaming");
  const loaderLabel =
    message.activityLabel ??
    (message.thinking ? t("message.thinking") : t("message.generating"));

  return (
    <div className="flex w-full justify-start">
      <div className="w-full min-w-0 px-1 py-1 text-sm">
        {message.thinking ? (
          <ChatReasoning
            thinking={message.thinking}
            isStreaming={isStreaming}
            hasAnswer={hasAnswer}
          />
        ) : null}
        {hasToolCalls ? (
          <div
            className={cn(
              message.thinking ? "mt-3" : undefined,
              "mb-4 w-full min-w-0",
            )}
          >
            <ChainOfThought>
              <ChainOfThoughtHeader icon={Wrench}>
                {t("message.toolUsage")}
              </ChainOfThoughtHeader>
              <ChainOfThoughtContent>
                <div className="mt-2 space-y-3 border-l border-border/70 py-1 pl-4">
                  {message.toolCalls?.map((toolCall) => (
                    <ChatToolCall key={toolCall.callId} toolCall={toolCall} />
                  ))}
                </div>
              </ChainOfThoughtContent>
            </ChainOfThought>
          </div>
        ) : null}
        {hasAnswer ? <ChatMarkdown content={message.content} /> : null}
        {hasAnswer ? (
          <DeterministicAnswerFacts message={message} t={t} />
        ) : null}
        {message.provenance ? (
          <AnswerProvenance provenance={message.provenance} t={t} />
        ) : null}
        {showLoader ? (
          <ChatLoader className="mt-1" label={loaderLabel} />
        ) : null}
        {message.status === "error" ? (
          <p className="text-sm text-destructive">
            {message.errorMessage ?? t("message.chatFailed")}
            {message.errorCode ? (
              <span className="ml-2 rounded-md bg-destructive/10 px-1.5 py-0.5 font-mono text-[10px] uppercase">
                {message.errorCode}
              </span>
            ) : null}
          </p>
        ) : null}
        {message.status === "cancelled" ? (
          <p className="mt-1 text-xs italic text-muted-foreground">
            {t("message.stoppedByUser")}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function DeterministicAnswerFacts({
  message,
  t,
}: {
  message: AiChatMessage;
  t: TFunction<"assistant">;
}) {
  const facts = collectDeterministicFacts(message, t).slice(0, 4);
  if (facts.length === 0) return null;
  return (
    <div className="mt-3 grid gap-2 sm:grid-cols-2">
      {facts.map((fact) => (
        <div
          key={`${fact.source}-${fact.label}`}
          className="rounded-md border border-border/70 bg-muted/20 px-3 py-2"
        >
          <div className="text-[10px] font-medium uppercase tracking-normal text-muted-foreground">
            {fact.source}
          </div>
          <div className="mt-0.5 text-sm font-medium text-foreground">
            {fact.label}
          </div>
          {fact.detail ? (
            <div className="mt-0.5 text-xs text-muted-foreground">
              {fact.detail}
            </div>
          ) : null}
        </div>
      ))}
    </div>
  );
}

function AnswerProvenance({
  provenance,
  t,
}: {
  provenance: NonNullable<AiChatMessage["provenance"]>;
  t: TFunction<"assistant">;
}) {
  const parts = provenanceParts(provenance, t);
  if (parts.length === 0) return null;
  return (
    <div className="mt-3 flex flex-wrap gap-1.5 text-[11px] text-muted-foreground">
      {parts.map((part) => (
        <span
          key={part}
          className="rounded-md border border-border/70 bg-background px-2 py-1"
        >
          {part}
        </span>
      ))}
    </div>
  );
}

interface DeterministicFact {
  source: string;
  label: string;
  detail?: string;
}

function collectDeterministicFacts(
  message: AiChatMessage,
  t: TFunction<"assistant">,
): DeterministicFact[] {
  const facts: DeterministicFact[] = [];
  for (const toolCall of message.toolCalls ?? []) {
    const envelope = asRecord(toolCall.result);
    const kind = typeof envelope?.kind === "string" ? envelope.kind : "";
    const data = asRecord(envelope?.data);
    if (!kind || !data) continue;
    if (kind === "ui.report.blockers") {
      const blockers = Array.isArray(data.blockers) ? data.blockers : [];
      facts.push({
        source: t("facts.reportReadiness"),
        label: data.ready
          ? t("facts.ready")
          : t("facts.blockers", { count: blockers.length }),
        detail: blockers
          .map((item) => asRecord(item)?.title)
          .filter((title): title is string => typeof title === "string")
          .slice(0, 2)
          .join(", "),
      });
    } else if (kind === "ui.reports.summary") {
      const metrics = asRecord(data.metrics);
      const assetFlow = Array.isArray(data.asset_flow) ? data.asset_flow : [];
      const firstAsset = asRecord(assetFlow[0]);
      facts.push({
        source: t("facts.summary"),
        label: t("facts.activeTx", {
          count: Number(metrics?.active_transactions ?? 0),
        }),
        detail: firstAsset
          ? t("facts.assetFlow", {
              asset: firstAsset.asset ?? t("facts.asset"),
              inbound: formatSat(firstAsset.inbound_amount_sat),
              outbound: formatSat(firstAsset.outbound_amount_sat),
            })
          : undefined,
      });
    } else if (
      kind === "ui.reports.balance_sheet" ||
      kind === "ui.reports.portfolio_summary"
    ) {
      const totals = Array.isArray(data.totals_by_asset)
        ? data.totals_by_asset
        : [];
      const firstTotal = asRecord(totals[0]);
      facts.push({
        source:
          kind === "ui.reports.balance_sheet"
            ? t("facts.balanceSheet")
            : t("facts.portfolio"),
        label: t("facts.assetTotals", { count: totals.length }),
        detail: firstTotal
          ? t("facts.assetTotalDetail", {
              asset: firstTotal.asset ?? t("facts.asset"),
              quantity: formatSat(firstTotal.quantity_sat),
            })
          : undefined,
      });
    } else if (kind === "ui.reports.tax_summary") {
      const summary = asRecord(data.summary);
      const rows = Array.isArray(data.rows) ? data.rows : [];
      const firstRow = asRecord(rows[0]);
      facts.push({
        source: t("facts.taxSummary"),
        label: t("facts.rows", { count: Number(summary?.row_count ?? 0) }),
        detail:
          typeof firstRow?.gain_loss === "number"
            ? t("facts.gainLoss", { value: formatMoney(firstRow.gain_loss) })
            : undefined,
      });
    } else if (kind === "ui.rates.coverage") {
      const summary = asRecord(data.summary);
      facts.push({
        source: t("facts.rateCoverage"),
        label: t("facts.missingPrices", {
          count: Number(summary?.missing_price_transactions ?? 0),
        }),
        detail: t("facts.coverableFromCache", {
          count: Number(summary?.cache_coverable_missing ?? 0),
        }),
      });
    } else if (kind === "ui.audit.changes_since_last_answer") {
      let label = t("facts.noChangesSinceBaseline");
      if (data.status === "baseline_required") {
        label = t("facts.baselineRequired");
      } else if (data.changed) {
        label = t("facts.changedSinceBaseline");
      }
      facts.push({
        source: t("facts.changeAudit"),
        label,
      });
    } else if (kind === "ui.maintenance.run") {
      const blockers = Array.isArray(data.blockers) ? data.blockers : [];
      facts.push({
        source: t("facts.maintenance"),
        label: data.ready
          ? t("facts.reportsReady")
          : t("facts.blockers", { count: blockers.length }),
        detail:
          typeof data.sync_mode === "string"
            ? t("facts.syncMode", { mode: data.sync_mode })
            : undefined,
      });
    }
  }
  return facts;
}

function provenanceParts(
  provenance: NonNullable<AiChatMessage["provenance"]>,
  t: TFunction<"assistant">,
): string[] {
  const parts: string[] = [];
  const toolCount = provenance.tools_used?.length ?? 0;
  if (toolCount > 0) {
    parts.push(t("provenance.localTools", { count: toolCount }));
  }
  if (
    provenance.active_transactions !== null &&
    provenance.active_transactions !== undefined
  ) {
    parts.push(
      t("provenance.activeTx", {
        count: Number(provenance.active_transactions),
      }),
    );
  }
  if (provenance.quarantines !== null && provenance.quarantines !== undefined) {
    parts.push(
      t("provenance.quarantine", { count: Number(provenance.quarantines) }),
    );
  }
  if (
    provenance.missing_price_transactions !== null &&
    provenance.missing_price_transactions !== undefined
  ) {
    parts.push(
      t("provenance.missingPrices", {
        count: Number(provenance.missing_price_transactions),
      }),
    );
  }
  if (provenance.auto_journal_processed) {
    parts.push(
      provenance.journals_processed_at
        ? t("provenance.journalsRefreshedAt", {
            time: shortTime(provenance.journals_processed_at),
          })
        : t("provenance.journalsRefreshed"),
    );
  } else if (provenance.journals_processed_at) {
    parts.push(
      t("provenance.journalsAt", {
        time: shortTime(provenance.journals_processed_at),
      }),
    );
  }
  if (provenance.auto_sync_attempted) {
    parts.push(
      provenance.auto_sync_ok === false
        ? t("provenance.syncFailed")
        : t("provenance.syncChecked"),
    );
  }
  if (provenance.generated_at) {
    parts.push(
      t("provenance.answeredAt", { time: shortTime(provenance.generated_at) }),
    );
  }
  return parts;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function formatCount(value: unknown): string {
  return Number(value ?? 0).toLocaleString("en-US");
}

function formatSat(value: unknown): string {
  return `${formatCount(value)} sat`;
}

function formatMoney(value: number): string {
  return value.toLocaleString("en-US", {
    maximumFractionDigits: 2,
    minimumFractionDigits: 2,
  });
}

function shortTime(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
