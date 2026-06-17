import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from "@/components/ai-elements";
import type { AiChatToolCall } from "@/daemon/stream";

interface ChatToolCallProps {
  toolCall: AiChatToolCall;
}

export function ChatToolCall({ toolCall }: ChatToolCallProps) {
  const { t } = useTranslation("assistant");
  const hasArguments = Object.keys(toolCall.arguments).length > 0;
  const hasResult = toolCall.result !== undefined && toolCall.result !== null;
  const summary = summarizeToolResult(toolCall.result, t);
  const errorText =
    toolCall.status === "error" || toolCall.status === "denied"
      ? toolCall.reason
      : undefined;
  return (
    <Tool
      className={
        toolCall.status === "error"
          ? "border-destructive/35 bg-destructive/5"
          : undefined
      }
    >
      <ToolHeader name={toolCall.name} state={toolCall.status} />
      <ToolContent>
        {hasArguments ? <ToolInput input={toolCall.arguments} /> : null}
        {summary ? (
          <ToolOutput output={summary} label={t("tool.summaryLabel")} />
        ) : null}
        {hasResult && !summary ? (
          <ToolOutput output={toolCall.result} label={t("tool.detailsLabel")} />
        ) : null}
        {errorText ? (
          <ToolOutput error={errorText} />
        ) : toolCall.reason ? (
          <ToolOutput output={toolCall.reason} label={t("tool.reasonLabel")} />
        ) : null}
      </ToolContent>
    </Tool>
  );
}

function summarizeToolResult(
  result: unknown,
  t: TFunction<"assistant">,
): string | null {
  const envelope = asRecord(result);
  const kind = typeof envelope?.kind === "string" ? envelope.kind : "";
  const data = asRecord(envelope?.data);
  if (!kind || !data) return null;

  switch (kind) {
    case "ui.overview.snapshot": {
      const connections = Array.isArray(data.connections)
        ? data.connections
        : [];
      const txs = Array.isArray(data.txs) ? data.txs : [];
      const fiat = asRecord(data.fiat);
      const priceEur =
        typeof data.priceEur === "number"
          ? t("tool.overviewSnapshot.priceEur", {
              value: formatMoney(data.priceEur),
            })
          : null;
      const realizedYtd =
        typeof fiat?.eurRealizedYTD === "number"
          ? t("tool.overviewSnapshot.realizedYtd", {
              value: formatMoney(fiat.eurRealizedYTD),
            })
          : null;
      return [
        t("tool.overviewSnapshot.connections", { count: connections.length }),
        t("tool.overviewSnapshot.recentTransactions", { count: txs.length }),
        priceEur,
        realizedYtd,
      ]
        .filter((part): part is string => Boolean(part))
        .join("; ");
    }
    case "ui.workspace.health": {
      const booksSet =
        asRecord(data.workspace)?.label ?? t("tool.workspaceHealth.noBooksSet");
      const books =
        asRecord(data.profile)?.label ?? t("tool.workspaceHealth.noBook");
      const journals = asRecord(data.journals);
      const reports = asRecord(data.reports);
      return t("tool.workspaceHealth.summary", {
        booksSet,
        books,
        journals: journals?.status ?? t("tool.workspaceHealth.statusUnknown"),
        reports: reports?.ready
          ? t("tool.workspaceHealth.reportsReady")
          : t("tool.workspaceHealth.reportsNotReady"),
      });
    }
    case "ui.next_actions": {
      const suggestions = Array.isArray(data.suggestions)
        ? data.suggestions
        : [];
      const titles = suggestions
        .map((item) => asRecord(item)?.title)
        .filter((title): title is string => typeof title === "string");
      return titles.length
        ? t("tool.nextActions.suggestions", {
            count: titles.length,
            titles: titles.join(", "),
          })
        : t("tool.nextActions.none");
    }
    case "ui.wallets.list": {
      const wallets = Array.isArray(data.wallets) ? data.wallets : [];
      const labels = wallets
        .map((item) => asRecord(item)?.label)
        .filter((label): label is string => typeof label === "string")
        .slice(0, 4);
      const walletsLabel = t("tool.walletsList.wallets", {
        count: wallets.length,
      });
      return labels.length
        ? t("tool.walletsList.withLabels", {
            wallets: walletsLabel,
            labels: labels.join(", "),
          })
        : t("tool.walletsList.withoutLabels", { wallets: walletsLabel });
    }
    case "ui.backends.list": {
      const backends = Array.isArray(data.backends) ? data.backends : [];
      const defaultBackend = asRecord(data.summary)?.default_backend;
      const backendsLabel = t("tool.backendsList.backends", {
        count: backends.length,
      });
      return typeof defaultBackend === "string"
        ? t("tool.backendsList.withDefault", {
            backends: backendsLabel,
            default: defaultBackend,
          })
        : t("tool.backendsList.withoutDefault", { backends: backendsLabel });
    }
    case "ui.journals.quarantine": {
      const summary = asRecord(data.summary);
      return t("tool.quarantine.summary", {
        count: Number(summary?.count ?? 0),
      });
    }
    case "ui.journals.transfers.list": {
      const summary = asRecord(data.summary);
      const transferEntries = Number(summary?.journal_transfer_entries ?? 0);
      const pairs = t("tool.transfersList.pairs", {
        count: Number(summary?.manual_pairs ?? 0),
      });
      return t("tool.transfersList.summary", {
        count: transferEntries,
        pairs,
        transferEntries,
      });
    }
    case "ui.rates.summary": {
      const pairs = Array.isArray(data.pairs) ? data.pairs : [];
      return t("tool.ratesSummary.pairs", { count: pairs.length });
    }
    case "ui.rates.coverage": {
      const summary = asRecord(data.summary);
      const missing = Number(summary?.missing_price_transactions ?? 0);
      return t("tool.ratesCoverage.summary", {
        count: missing,
        missing,
        coverable: Number(summary?.cache_coverable_missing ?? 0),
      });
    }
    case "ui.report.blockers": {
      const blockers = Array.isArray(data.blockers) ? data.blockers : [];
      return data.ready
        ? t("tool.reportBlockers.ready")
        : t("tool.reportBlockers.blockers", { count: blockers.length });
    }
    case "ui.audit.changes_since_last_answer": {
      const counts = asRecord(data.counts_since);
      if (data.status === "baseline_required") {
        return t("tool.changesSince.baselineRequired");
      }
      const txCount = Number(counts?.transactions ?? 0);
      return t("tool.changesSince.summary", {
        count: txCount,
        state: data.changed
          ? t("tool.changesSince.changed")
          : t("tool.changesSince.unchanged"),
        txCount,
        journalCount: Number(counts?.journal_entries ?? 0),
      });
    }
    case "ui.maintenance.settings": {
      const settings = asRecord(data.settings);
      return settings?.auto_sync_before_report_reads
        ? t("tool.maintenanceSettings.enabled")
        : t("tool.maintenanceSettings.disabled");
    }
    case "ui.maintenance.configure": {
      const settings = asRecord(data.settings);
      return settings?.auto_sync_before_report_reads
        ? t("tool.maintenanceConfigure.enabled")
        : t("tool.maintenanceConfigure.disabled");
    }
    case "ui.maintenance.run": {
      const blockers = Array.isArray(data.blockers) ? data.blockers : [];
      return data.ready
        ? t("tool.maintenanceRun.ready")
        : t("tool.maintenanceRun.blockers", { count: blockers.length });
    }
    case "ui.transactions.list": {
      const txs = Array.isArray(data.txs) ? data.txs : [];
      return t("tool.transactionsList.summary", { count: txs.length });
    }
    case "ui.transactions.extremes": {
      const largest = Array.isArray(data.largest) ? data.largest : [];
      const smallest = Array.isArray(data.smallest) ? data.smallest : [];
      return t("tool.transactionsExtremes.summary", {
        largest: t("tool.transactionsExtremes.largest", {
          count: largest.length,
        }),
        smallest: t("tool.transactionsExtremes.smallest", {
          count: smallest.length,
        }),
      });
    }
    case "ui.transactions.search": {
      const txs = Array.isArray(data.txs) ? data.txs : [];
      return t("tool.transactionsSearch.summary", { count: txs.length });
    }
    case "ui.reports.summary": {
      const metrics = asRecord(data.metrics);
      const assetFlow = Array.isArray(data.asset_flow) ? data.asset_flow : [];
      const wallets = Array.isArray(data.wallet_flow) ? data.wallet_flow : [];
      const pairs = Array.isArray(data.transfer_pairs)
        ? data.transfer_pairs
        : [];
      return t("tool.reportsSummary.summary", {
        activeTransactions: t("tool.reportsSummary.activeTransactions", {
          count: Number(metrics?.active_transactions ?? 0),
        }),
        assetFlowRows: t("tool.reportsSummary.assetFlowRows", {
          count: assetFlow.length,
        }),
        walletFlowRows: t("tool.reportsSummary.walletFlowRows", {
          count: wallets.length,
        }),
        transferPairs: t("tool.reportsSummary.transferPairs", {
          count: pairs.length,
        }),
      });
    }
    case "ui.reports.balance_sheet": {
      const rows = Array.isArray(data.rows) ? data.rows : [];
      const totals = Array.isArray(data.totals_by_asset)
        ? data.totals_by_asset
        : [];
      return t("tool.balanceSheet.summary", {
        balanceRows: t("tool.balanceSheet.balanceRows", { count: rows.length }),
        assetTotals: t("tool.balanceSheet.assetTotals", {
          count: totals.length,
        }),
      });
    }
    case "ui.reports.portfolio_summary": {
      const rows = Array.isArray(data.rows) ? data.rows : [];
      const totals = Array.isArray(data.totals_by_asset)
        ? data.totals_by_asset
        : [];
      return t("tool.portfolioSummary.summary", {
        holdingRows: t("tool.portfolioSummary.holdingRows", {
          count: rows.length,
        }),
        assetTotals: t("tool.portfolioSummary.assetTotals", {
          count: totals.length,
        }),
      });
    }
    case "ui.reports.tax_summary": {
      const rows = Array.isArray(data.rows) ? data.rows : [];
      const years = Array.isArray(data.available_years)
        ? data.available_years
        : [];
      return t("tool.taxSummary.summary", {
        taxRows: t("tool.taxSummary.taxRows", { count: rows.length }),
        years: t("tool.taxSummary.years", { count: years.length }),
      });
    }
    case "ui.reports.balance_history": {
      const rows = Array.isArray(data.rows) ? data.rows : [];
      const filters = asRecord(data.filters);
      return t("tool.balanceHistory.buckets", {
        count: rows.length,
        interval: filters?.interval ?? t("tool.balanceHistory.intervalFallback"),
      });
    }
    default:
      return null;
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function formatMoney(value: number): string {
  return `€${value.toLocaleString("en-US", {
    maximumFractionDigits: 2,
    minimumFractionDigits: 2,
  })}`;
}
