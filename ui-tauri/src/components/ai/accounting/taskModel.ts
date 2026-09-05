export const taskSteps = ["prepare", "post", "close", "tax_finalize", "export_close", "export_tax"] as const;
export type TaskStep = typeof taskSteps[number];
export type TaskCoverage = { source_kind: string; source_id: string; status: string; exception?: string; entry_id?: string;
  statement_id?: string; name?: string; account_code?: string; description?: string; amount_minor?: string; occurred_on?: string;
  assignment_id?: string | null; reviews_truncated?: boolean;
  review_options?: { extraction_id: string; review_digest: string; fields: Record<string, unknown> }[] };
export type AccountingTask = { id: string; period_id: string; state: string; source_count: number; coverage?: TaskCoverage[]; exceptions?: TaskCoverage[]; receipts?: { id: string; step: TaskStep; result?: unknown }[]; next_step?: TaskStep };
export type TaskProposal = { source_kind: string; source_id: string; payload: { description: string; entry_date: string; lines: { account_code: string; debit_minor: string; credit_minor: string }[] } };
export type TaskPreview = AccountingTask & { step: TaskStep; expected_digest: string; expected_revision: number; ready: boolean; proposals?: TaskProposal[]; blockers?: { kind: string; code?: string }[]; detail?: unknown };

export function isTaskPreview(value: unknown): value is TaskPreview {
  if (!value || typeof value !== "object") return false;
  const row = value as Record<string, unknown>;
  if (typeof row.id !== "string" || typeof row.period_id !== "string" || typeof row.state !== "string" || !Number.isSafeInteger(row.source_count) || Number(row.source_count) < 0 || !taskSteps.includes(row.step as TaskStep) || typeof row.ready !== "boolean" || typeof row.expected_digest !== "string" || !/^[a-f0-9]{64}$/.test(row.expected_digest) || !Number.isSafeInteger(row.expected_revision) || Number(row.expected_revision) < 0) return false;
  if (row.proposals !== undefined && (!Array.isArray(row.proposals) || !row.proposals.every((proposal: unknown) => {
    if (!proposal || typeof proposal !== "object") return false;
    const item = proposal as Record<string, unknown>;
    return typeof item.source_id === "string" && typeof item.source_kind === "string" && isTaskEntry(item.payload);
  }))) return false;
  if (row.blockers !== undefined && (!Array.isArray(row.blockers) || !row.blockers.every((blocker: unknown) => Boolean(blocker && typeof blocker === "object" && typeof (blocker as Record<string, unknown>).kind === "string")))) return false;
  if (row.ready === false) return true;
  if (!Array.isArray(row.blockers) || row.blockers.length > 0) return false;
  if (row.step === "prepare") return Array.isArray(row.proposals) && row.proposals.length > 0;
  if (!row.detail || typeof row.detail !== "object") return false;
  const detail = row.detail as Record<string, unknown>;
  if (row.step === "post") return Array.isArray(detail.entries) && detail.entries.length > 0 && detail.entries.every(isTaskEntry);
  if (row.step === "close") return detail.period_id === row.period_id && detail.ready === true && Array.isArray(detail.blockers) && detail.blockers.length === 0 && isTaskReports(detail) && detail.trial_balance.balanced && detail.statements.balanced;
  if (row.step === "tax_finalize") return Array.isArray(detail.forms) && detail.forms.length > 0 && detail.forms.every((value: unknown) => {
    if (!value || typeof value !== "object") return false;
    const form = value as Record<string, unknown>;
    return typeof form.form_id === "string" && Boolean(form.fields && typeof form.fields === "object");
  });
  if (row.step === "export_close") return typeof detail.id === "string" && typeof detail.snapshot_digest === "string" && /^[a-f0-9]{64}$/.test(detail.snapshot_digest);
  return typeof detail.final_id === "string" && typeof detail.report_digest === "string" && /^[a-f0-9]{64}$/.test(detail.report_digest);
}

export function isTaskReports(value: unknown): value is import("./model").Reports {
  if (!value || typeof value !== "object") return false;
  const { trial_balance: trial, statements } = value as Record<string, unknown>;
  if (!trial || typeof trial !== "object" || !statements || typeof statements !== "object") return false;
  const a = trial as Record<string, unknown>, b = statements as Record<string, unknown>;
  const money = (v: unknown) => typeof v === "string" && /^-?\d+$/.test(v);
  const rows = (v: unknown) => Array.isArray(v) && v.every((r: unknown) => {
    if (!r || typeof r !== "object") return false;
    const line = r as Record<string, unknown>;
    return typeof line.account_code === "string" && typeof line.name === "string" && [line.debit_minor, line.credit_minor, line.balance_minor].every(money);
  });
  return rows(a.rows) && rows(b.profit_and_loss) && rows(b.balance_sheet) && [a.debit_minor, a.credit_minor, b.profit_minor, b.unappropriated_result_minor].every(money) && typeof a.balanced === "boolean" && typeof b.balanced === "boolean";
}

export function isTaskEntry(value: unknown): value is TaskProposal["payload"] {
  if (!value || typeof value !== "object") return false;
  const entry = value as Record<string, unknown>;
  return typeof entry.description === "string" && typeof entry.entry_date === "string" && Array.isArray(entry.lines) && entry.lines.length > 0 && entry.lines.every((value: unknown) => {
    if (!value || typeof value !== "object") return false;
    const line = value as Record<string, unknown>;
    return typeof line.account_code === "string" && typeof line.debit_minor === "string" && /^\d+$/.test(line.debit_minor) && typeof line.credit_minor === "string" && /^\d+$/.test(line.credit_minor);
  });
}
