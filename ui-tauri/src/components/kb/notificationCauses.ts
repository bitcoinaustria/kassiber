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

const CODE_CAUSES: Readonly<Record<string, NotificationCause>> = {
  network_error: "connection",
  backend_timeout: "connection",
  backend_tls_config_invalid: "certificate",
  backend_not_configured: "noBackend",
  cancelled: "cancelled",
};

const MESSAGE_CAUSES: ReadonlyArray<[NotificationCause, RegExp]> = [
  ["offline", /offline mode|outbound requests are disabled/i],
  ["rateLimited", /rate[ -]?limit|too many requests|\b429\b|cooling down/i],
  ["unavailable", /temporarily unavailable|service unavailable|\b50[234]\b/i],
  ["certificate", /certificate|\btls\b|handshake|\bca bundle/i],
  ["credentials", /\b401\b|\b403\b|unauthori[sz]ed|forbidden|api key|authenticat/i],
  ["noBackend", /no sync backend/i],
  [
    "connection",
    /network i\/o|connection (refused|reset|closed)|could not connect|unreachable|timed? ?out|timeout|dns|budget exhausted/i,
  ],
  ["cancelled", /\bcancel+ed\b/i],
];

export function classifyCause(
  detail: Pick<NotificationDetail, "message" | "phraseKey" | "code">,
): NotificationCause | null {
  if (detail.phraseKey === "coolingDown") return "rateLimited";
  const message = detail.message ?? "";
  // The text tells offline and 429-vs-503 apart where one code covers both.
  for (const [cause, pattern] of MESSAGE_CAUSES) {
    if (pattern.test(message)) return cause;
  }
  const byCode = detail.code ? CODE_CAUSES[detail.code] : undefined;
  if (byCode) return byCode;
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
