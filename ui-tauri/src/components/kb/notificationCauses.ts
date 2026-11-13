import type { NotificationDetail } from "@/store/notifications";

/**
 * Why a source or run failed, in a closed set of plain causes. The daemon
 * reports a source's last error as text only, so the cause is read from the
 * message and the detail's fixed phrase. Anything that does not match stays
 * unclassified and keeps its own message.
 */
export const NOTIFICATION_CAUSES = [
  "offline",
  "rateLimited",
  "certificate",
  "credentials",
  "noBackend",
  "unreachable",
  "cancelled",
] as const;
export type NotificationCause = (typeof NOTIFICATION_CAUSES)[number];

const CAUSE_PATTERNS: ReadonlyArray<[NotificationCause, RegExp]> = [
  ["offline", /offline mode|outbound requests are disabled/i],
  ["rateLimited", /rate[ -]?limit|too many requests|\b429\b|\b503\b|cooling down/i],
  ["certificate", /certificate|\btls\b|\bssl\b|handshake|\bca bundle/i],
  ["credentials", /\b401\b|\b403\b|unauthori[sz]ed|forbidden|api key|credential|authenticat/i],
  ["noBackend", /no sync backend|backend_not_configured/i],
  [
    "unreachable",
    /network i\/o|connection (refused|reset|closed)|could not connect|unreachable|timed? ?out|timeout|dns|resolve host|budget exhausted|network_error|backend_timeout/i,
  ],
  ["cancelled", /\bcancel+ed\b/i],
];

export function classifyCause(
  detail: Pick<NotificationDetail, "message" | "phraseKey">,
): NotificationCause | null {
  if (detail.phraseKey === "coolingDown") return "rateLimited";
  const message = detail.message ?? "";
  for (const [cause, pattern] of CAUSE_PATTERNS) {
    if (pattern.test(message)) return cause;
  }
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
  /** Distinct raw messages behind the group, with how often each came. */
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
    if (message) {
      const seen = group.messages.find((item) => item.text === message);
      if (seen) seen.count += 1;
      else group.messages.push({ text: message, count: 1 });
    }
  }
  // Biggest group first: the cause that explains most of the failures.
  return [...groups.values()].sort((a, b) => b.count - a.count);
}
