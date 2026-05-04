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
  const hasArguments = Object.keys(toolCall.arguments).length > 0;
  const hasResult = toolCall.result !== undefined && toolCall.result !== null;
  const summary = summarizeToolResult(toolCall.result);
  const errorText =
    toolCall.status === "error" || toolCall.status === "denied"
      ? toolCall.reason
      : undefined;
  return (
    <Tool
      defaultOpen={
        toolCall.status === "awaiting_consent" ||
        toolCall.status === "running" ||
        toolCall.status === "error"
      }
      className={
        toolCall.status === "error"
          ? "border-destructive/35 bg-destructive/5"
          : undefined
      }
    >
      <ToolHeader name={toolCall.name} state={toolCall.status} />
      <ToolContent>
        {hasArguments ? <ToolInput input={toolCall.arguments} /> : null}
        {summary ? <ToolOutput output={summary} label="Summary" /> : null}
        {hasResult && !summary ? (
          <ToolOutput output={toolCall.result} label="Details" />
        ) : null}
        {errorText ? (
          <ToolOutput error={errorText} />
        ) : toolCall.reason ? (
          <ToolOutput output={toolCall.reason} label="Reason" />
        ) : null}
      </ToolContent>
    </Tool>
  );
}

function summarizeToolResult(result: unknown): string | null {
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
          ? `BTC/EUR ${formatMoney(data.priceEur)}`
          : null;
      const realizedYtd =
        typeof fiat?.eurRealizedYTD === "number"
          ? `realized YTD ${formatMoney(fiat.eurRealizedYTD)}`
          : null;
      return [
        `${connections.length} connection(s)`,
        `${txs.length} recent transaction(s)`,
        priceEur,
        realizedYtd,
      ]
        .filter((part): part is string => Boolean(part))
        .join("; ");
    }
    case "ui.workspace.health": {
      const booksSet = asRecord(data.workspace)?.label ?? "No books set";
      const books = asRecord(data.profile)?.label ?? "No book";
      const journals = asRecord(data.journals);
      const reports = asRecord(data.reports);
      return `${booksSet} / ${books}: journals ${journals?.status ?? "unknown"}, reports ${
        reports?.ready ? "ready" : "not ready"
      }.`;
    }
    case "ui.next_actions": {
      const suggestions = Array.isArray(data.suggestions)
        ? data.suggestions
        : [];
      const titles = suggestions
        .map((item) => asRecord(item)?.title)
        .filter((title): title is string => typeof title === "string");
      return titles.length
        ? `${titles.length} suggestion(s): ${titles.join(", ")}.`
        : "No next action suggestions.";
    }
    case "ui.wallets.list": {
      const wallets = Array.isArray(data.wallets) ? data.wallets : [];
      const labels = wallets
        .map((item) => asRecord(item)?.label)
        .filter((label): label is string => typeof label === "string")
        .slice(0, 4);
      return `${wallets.length} wallet(s)${labels.length ? `: ${labels.join(", ")}` : ""}.`;
    }
    case "ui.backends.list": {
      const backends = Array.isArray(data.backends) ? data.backends : [];
      const defaultBackend = asRecord(data.summary)?.default_backend;
      return `${backends.length} backend(s)${
        typeof defaultBackend === "string" ? `; default ${defaultBackend}` : ""
      }.`;
    }
    case "ui.journals.quarantine": {
      const summary = asRecord(data.summary);
      return `${summary?.count ?? 0} quarantined transaction(s).`;
    }
    case "ui.journals.transfers.list": {
      const summary = asRecord(data.summary);
      const transferEntries = Number(summary?.journal_transfer_entries ?? 0);
      return `${summary?.manual_pairs ?? 0} pair(s), ${transferEntries} journal transfer ${
        transferEntries === 1 ? "entry" : "entries"
      }.`;
    }
    case "ui.rates.summary": {
      const pairs = Array.isArray(data.pairs) ? data.pairs : [];
      return `${pairs.length} cached rate pair(s).`;
    }
    case "ui.transactions.list": {
      const txs = Array.isArray(data.txs) ? data.txs : [];
      return `${txs.length} transaction(s).`;
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
