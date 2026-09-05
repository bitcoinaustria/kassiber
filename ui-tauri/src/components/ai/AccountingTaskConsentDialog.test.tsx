import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiToolConsentRequest } from "@/daemon/stream";
import { AccountingTaskConsentDialog } from "./AccountingTaskConsentDialog";
import { ToolConsentDialog } from "./ToolConsentDialog";
import { TaskPreviewCard } from "./accounting/TaskPreviewCard";

// Node-only hook slots exercise the actual event handlers; layout is not mocked
// into a claim about native focus trapping or a full browser interaction.
const mock = vi.hoisted(() => ({ hidden: false, cursor: 0,
  slots: [] as { value?: unknown; current?: unknown; dependencies?: unknown[] }[] }));
vi.mock("react", async (original) => ({ ...await original<typeof import("react")>(),
  useId: () => "consent-id",
  useState: (initial: unknown) => { const index = mock.cursor++; mock.slots[index] ??= { value: initial };
    return [mock.slots[index].value, (value: unknown) => { mock.slots[index].value = value; }]; },
  useRef: (initial: unknown) => { const index = mock.cursor++; return mock.slots[index] ??= { current: initial }; },
  useEffect: (effect: () => void, dependencies: unknown[]) => {
    const index = mock.cursor++; const old = mock.slots[index];
    if (!old || dependencies.some((value, i) => value !== old.dependencies?.[i])) effect();
    mock.slots[index] = { dependencies };
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/store/ui", () => ({ useUiStore: Object.assign(
  (select: (state: { hideSensitive: boolean }) => unknown) => select({ hideSensitive: mock.hidden }),
  { getState: () => ({ hideSensitive: mock.hidden }) },
) }));

type Node = ReactElement<Record<string, unknown>>;
function nodes(value: ReactNode): Node[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  if (!isValidElement<Record<string, unknown>>(value)) return [];
  return [value, ...nodes(value.props.children as ReactNode)];
}
const exact = "9007199254740997";
function request(step: "prepare" | "post" = "prepare"): AiToolConsentRequest {
  const entry = { description: "LOCAL-ONLY-ENTRY", entry_date: "2025-01-02", lines: [
    { account_code: "bank", debit_minor: exact, credit_minor: "0" },
    { account_code: "sales", debit_minor: "0", credit_minor: exact },
  ] };
  return { targetRequestId: "chat", callId: "call", name: "ui.accounting.task_apply", summary: "FORGED-SUMMARY",
    argumentsPreview: { task_id: "task", approval_id: "opaque", idempotency_key: "once",
      accounting_task_preview: { status: "ready", preview: "FORGED-FINANCIALS" } },
    accountingTaskPreview: { status: "ready", step, book: { currency: "EUR", minor_unit_exponent: 2 }, preview: {
      id: "task", period_id: "2025", state: "active", source_count: 1, step, ready: true,
      expected_revision: 7, expected_digest: "a".repeat(64), blockers: [],
      proposals: step === "prepare" ? [{ source_kind: "bank_row", source_id: "row", payload: entry }] : [],
      detail: step === "post" ? { entries: [entry], draft_ids: ["draft"] } : {},
    } },
  };
}

describe("desktop-only accounting task consent", () => {
  let value: AiToolConsentRequest;
  const decide = vi.fn();
  beforeEach(() => { mock.hidden = false; mock.slots = []; mock.cursor = 0; decide.mockReset(); value = request(); });
  function render() { mock.cursor = 0; return AccountingTaskConsentDialog({ request: value, onDecision: decide }); }
  function find(test: (node: Node) => boolean) { const result = nodes(render()).find(test); if (!result) throw new Error("Missing control"); return result; }
  function button(label: string) { return find((node) => node.props.children === label); }
  function acknowledge() { (find((node) => node.props.type === "checkbox").props.onChange as (event: unknown) => void)({ target: { checked: true } }); }

  it("routes task apply to the dedicated dialog and renders the exact server-computed entries", () => {
    expect(ToolConsentDialog({ request: value, onDecision: decide }).type).toBe(AccountingTaskConsentDialog);
    const preview = find((node) => node.type === TaskPreviewCard);
    const markup = renderToStaticMarkup(preview);
    expect(markup).toContain("LOCAL-ONLY-ENTRY");
    expect(markup).toContain("90.071.992.547.409,97");
    expect(markup).not.toContain("FORGED");
  });

  it("requires acknowledgement and approves exactly one distinct step", async () => {
    for (const step of ["prepare", "post"] as const) {
      mock.slots = []; value = request(step);
      const label = `consent.accountingTask.${step}`;
      expect(button(label).props.disabled).toBe(true);
      await (button(label).props.onClick as () => Promise<void>)();
      expect(decide).not.toHaveBeenCalled();
      acknowledge();
      expect(button(label).props.disabled).toBe(false);
      await (button(label).props.onClick as () => Promise<void>)();
      expect(decide).toHaveBeenCalledExactlyOnceWith("allow_once");
      expect(nodes(render()).some((node) => node.props.children === "consent.allowSession")).toBe(false);
      decide.mockClear();
    }
  });

  it("never authorizes missing, unavailable, mismatched or malformed daemon previews", async () => {
    for (const preview of [undefined, { status: "unavailable" as const, code: "stale" },
      { status: "ready" as const, step: "post", book: { currency: "EUR", minor_unit_exponent: 2 }, preview: {} },
      { ...request().accountingTaskPreview!, step: "close" },
    ]) {
      mock.slots = []; value = { ...request(), accountingTaskPreview: preview };
      expect(nodes(render()).some((node) => node.type === TaskPreviewCard)).toBe(false);
      acknowledge();
      expect(button("consent.allowOnce").props.disabled).toBe(true);
      await (button("consent.allowOnce").props.onClick as () => Promise<void>)();
      expect(decide).not.toHaveBeenCalled();
    }
  });

  it("suppresses hidden financials and rejects even a previously captured approve callback", async () => {
    render(); acknowledge();
    const approve = button("consent.accountingTask.prepare").props.onClick as () => Promise<void>;
    mock.hidden = true;
    await approve();
    expect(decide).not.toHaveBeenCalled();
    expect(nodes(render()).some((node) => node.type === TaskPreviewCard)).toBe(false);
    expect(button("consent.accountingTask.prepare").props.disabled).toBe(true);
    mock.hidden = false;
    render();
    expect(button("consent.accountingTask.prepare").props.disabled).toBe(true);
  });

  it("defaults focus to cancel and treats Escape as denial", () => {
    const cancel = button("consent.apply.cancel");
    const focus = vi.fn();
    (cancel.props.ref as { current: unknown }).current = { focus };
    const dialog = find((node) => node.props.role === "alertdialog");
    const event = { preventDefault: vi.fn() };
    (dialog.props.onOpenAutoFocus as (event: unknown) => void)(event);
    expect(focus).toHaveBeenCalledOnce();
    (dialog.props.onEscapeKeyDown as (event: unknown) => void)(event);
    expect(decide).toHaveBeenCalledExactlyOnceWith("deny");
  });
});
