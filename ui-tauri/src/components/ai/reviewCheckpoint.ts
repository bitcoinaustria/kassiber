const MAX_CHECKPOINT_LENGTH = 16_384;
const STRING_FIELDS = new Set(["workspace_id", "profile_id"]);
const NULLABLE_STRING_FIELDS = new Set(["next_cursor", "unapplied_plan_digest"]);

function validCheckpoint(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const fields = Object.entries(value);
  return fields.length > 0 && fields.every(([key, field]) => {
    if (STRING_FIELDS.has(key)) return typeof field === "string" && field.length > 0;
    if (NULLABLE_STRING_FIELDS.has(key)) return field === null || typeof field === "string";
    if (key === "input_version") return Number.isSafeInteger(field) && Number(field) >= 0;
    const limit = key === "receipt_ids" ? 32 : key === "last_page_transaction_ids" ? 100 : 0;
    return limit > 0 && Array.isArray(field) && field.length <= limit &&
      field.every((item) => typeof item === "string" && item.length > 0);
  });
}

/** A packet printed inside another code example is still ordinary message content. */
function insideFence(prefix: string): boolean {
  let fence: { marker: string; length: number } | null = null;
  for (const match of prefix.matchAll(/^ {0,3}(`{3,}|~{3,})([^\r\n]*)/gm)) {
    const marker = match[1][0];
    if (fence) {
      if (marker === fence.marker && match[1].length >= fence.length && !match[2].trim()) fence = null;
    } else if (marker !== "`" || !match[2].includes("`")) {
      fence = { marker, length: match[1].length };
    }
  }
  return fence !== null;
}

/** Display projection only: the caller retains the original message for replay/copy. */
export function splitReviewCheckpoint(content: string): { prose: string; packet: string | null } {
  const unchanged = { prose: content, packet: null };
  const opening = content.lastIndexOf("\n```json");
  const start = opening >= 0 ? opening + 1 : content.startsWith("```json") ? 0 : -1;
  if (start < 0 || content.length - start > MAX_CHECKPOINT_LENGTH) return unchanged;
  const match = /^```json[ \t]*\r?\n([\s\S]*?)\r?\n```[ \t]*(?:\r?\n[ \t]*)*$/.exec(content.slice(start));
  if (!match || insideFence(content.slice(0, start))) return unchanged;
  try {
    const payload: unknown = JSON.parse(match[1]);
    if (!payload || typeof payload !== "object" || Array.isArray(payload) ||
        Object.keys(payload).length !== 1 || !("review_checkpoint" in payload) ||
        !validCheckpoint(payload.review_checkpoint)) return unchanged;
    return { prose: content.slice(0, start), packet: match[1] };
  } catch {
    return unchanged;
  }
}
