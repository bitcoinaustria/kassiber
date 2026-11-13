import type { NotificationDetail } from "@/store/notifications";

/**
 * Why a source or run failed, in a closed set of plain causes. The daemon's
 * error code decides where it is specific; otherwise the primary message
 * does. The remediation hint never does: its advice ("check the tcp/ssl
 * port") says nothing about what happened. Anything that does not match
 * stays unclassified and keeps its own message.
 *
 * `connection` is deliberately neutral ("connection failed", not "server
 * unreachable"): the network layer reports refused connections, timeouts,
 * and TLS failures alike as `network I/O failed`.
 */
export const NOTIFICATION_CAUSES = [
  "offline",
  "rateLimited",
  "unavailable",
  "certificate",
  "credentials",
  "noBackend",
  "connection",
  "cancelled",
] as const;
export type NotificationCause = (typeof NOTIFICATION_CAUSES)[number];

/** Codes that say what happened on their own. */
const CODE_CAUSES: Readonly<Record<string, NotificationCause>> = {
  network_egress_disabled: "offline",
  network_error: "connection",
  backend_timeout: "connection",
  backend_tls_config_invalid: "certificate",
  backend_not_configured: "noBackend",
  cancelled: "cancelled",
};

/**
 * Fixed phrases from the daemon's message templates and common HTTP status
 * text. Matched only after quoted names and the source's own label are
 * removed, so a backend called "tls-node" is not read as a TLS failure.
 */
const MESSAGE_CAUSES: ReadonlyArray<[NotificationCause, RegExp]> = [
  ["offline", /kassiber is in offline mode|outbound requests are disabled/i],
  ["rateLimited", /rate limited the request|too many requests|\(http 429\)|\bhttp 429\b|cooling down/i],
  ["unavailable", /temporarily unavailable|service unavailable|\(http 50[234]\)|\bhttp 50[234]\b/i],
  ["credentials", /\bhttp 40[13]\b|\(http 40[13]\)|\b40[13] (unauthori[sz]ed|forbidden)\b|invalid api key|authentication failed/i],
  ["certificate", /certificate (verify|is not trusted|has expired|is invalid)|tls handshake failed|custom ca bundle/i],
  ["noBackend", /no sync backend is configured/i],
  [
    "connection",
    /network i\/o failed|connection (refused|reset|closed)|could not connect|timed out|\btimeout\b|request budget exhausted/i,
  ],
];

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The message with interpolated names removed: quoted text and the label. */
function templateText(message: string, label: string | undefined): string {
  let text = message.replace(/'[^']*'|"[^"]*"|`[^`]*`/g, "''");
  const trimmed = label?.trim();
  if (trimmed) text = text.replace(new RegExp(escapeRegExp(trimmed), "gi"), "''");
  return text;
}

export function classifyCause(
  detail: Pick<NotificationDetail, "message" | "phraseKey" | "code" | "label">,
): NotificationCause | null {
  if (detail.phraseKey === "coolingDown") return "rateLimited";
  // `rate_limited` covers both 429 and 5xx, so its message decides.
  const byCode =
    detail.code && detail.code !== "rate_limited" ? CODE_CAUSES[detail.code] : undefined;
  if (byCode) return byCode;
  const message = templateText(detail.message ?? "", detail.label);
  for (const [cause, pattern] of MESSAGE_CAUSES) {
    if (pattern.test(message)) return cause;
  }
  if (detail.code === "rate_limited") return "rateLimited";
  if (detail.phraseKey === "cancelled") return "cancelled";
  return null;
}

/**
 * Detail lines folded by what went wrong. One group per cause; details
 * nothing classifies are grouped by their exact message, so twelve wallets
 * with the same unknown error still make one line.
 */
export interface DetailGroup {
  cause: NotificationCause | null;
  /** The shared message of an unclassified group; unset for a cause. */
  message?: string;
  phraseKey?: NotificationDetail["phraseKey"];
  labels: string[];
  /** Details in this group, with or without a label. */
  count: number;
  /**
   * Distinct raw texts behind the group (message, then hint), with how
   * often each came. Technical only: copied on request, never shown inline
   * for a classified cause, never sent to the assistant.
   */
  messages: Array<{ text: string; count: number }>;
}

export function groupDetails(
  details: readonly NotificationDetail[],
): DetailGroup[] {
  const groups = new Map<string, DetailGroup>();
  for (const detail of details) {
    const cause = classifyCause(detail);
    const message = detail.message?.trim() || undefined;
    // Phrases that only restate the entry's body ("failed", "blocks
    // reports") do not split a group.
    const phraseKey =
      cause === null &&
      !message &&
      detail.phraseKey !== "failed" &&
      detail.phraseKey !== "blocksReports"
        ? detail.phraseKey
        : undefined;
    const key = cause
      ? `cause:${cause}`
      : `message:${message ?? ""}:${phraseKey ?? ""}`;
    let group = groups.get(key);
    if (!group) {
      group = {
        cause,
        ...(cause ? {} : { message, phraseKey }),
        labels: [],
        count: 0,
        messages: [],
      };
      groups.set(key, group);
    }
    group.count += 1;
    const label = detail.label?.trim();
    if (label && !group.labels.includes(label)) group.labels.push(label);
    const raw = [message, detail.hint?.trim()].filter(Boolean).join(" — ");
    if (raw) {
      const seen = group.messages.find((item) => item.text === raw);
      if (seen) seen.count += 1;
      else group.messages.push({ text: raw, count: 1 });
    }
  }
  // Biggest group first: the cause that explains most of the failures.
  return [...groups.values()].sort((a, b) => b.count - a.count);
}
