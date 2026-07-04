// The dossier spine: a persistent rail showing the four case stages and the
// live state of the case (target, coverage of the traced amount, gates,
// disclosure posture). The rail is the navigation AND the status display —
// stages are freely clickable, never a locked wizard.

import { Check, CircleDashed, ShieldAlert, ShieldCheck } from "lucide-react";

import {
  formatBtc,
  pretty,
  shortId,
  txAmount,
  txWallet,
} from "./model";
import { type CaseStage, type SourceFundsCaseState } from "./useSourceFundsCase";

function stageStatusLine(
  stage: CaseStage,
  state: SourceFundsCaseState,
): string {
  switch (stage) {
    case "target": {
      if (!state.selectedTx) return "Pick the transaction to explain";
      const amount = state.selectedTargetAmount
        ? `${state.selectedTargetAmount} ${state.selectedTx.asset || "BTC"}`
        : txAmount(state.selectedTx);
      return `${txWallet(state.selectedTx)} · ${amount}`;
    }
    case "trace": {
      const proven = state.links.filter(
        (link) => state.reachableLinkIds.has(link.id) && link.state === "reviewed",
      ).length;
      const open = state.blockers.length;
      if (open > 0) {
        return `${proven} hop${proven === 1 ? "" : "s"} reviewed · ${open} gap${open === 1 ? "" : "s"} open`;
      }
      if (proven > 0) {
        return `${proven} hop${proven === 1 ? "" : "s"} reviewed · no gaps`;
      }
      return state.report ? "No gaps open" : "Not assembled yet";
    }
    case "disclose": {
      const txids = state.report?.disclosure_preview.txids.length ?? 0;
      const recipient = state.selectedRecipient?.label;
      return `${pretty(state.revealMode)}${recipient ? ` → ${recipient}` : ""} · ${txids} txid${txids === 1 ? "" : "s"}`;
    }
    case "export": {
      if (state.savedCase) return `Case ${shortId(state.savedCase.id)}`;
      return state.report?.explain_gates.exportable
        ? "Ready to freeze"
        : "Blocked by open gaps";
    }
  }
}

function stageDone(stage: CaseStage, state: SourceFundsCaseState): boolean {
  switch (stage) {
    case "target":
      return Boolean(state.selectedTx);
    case "trace":
      return Boolean(state.report) && state.blockers.length === 0;
    case "disclose":
      return Boolean(state.report);
    case "export":
      return Boolean(state.savedCase);
  }
}

export function CaseRail({ state }: { state: SourceFundsCaseState }) {
  const exportable = Boolean(state.report?.explain_gates.exportable);
  const overview = state.report?.overview;
  const targetAmount = overview?.target_amount ?? null;

  return (
    <aside className="flex h-fit flex-col gap-4 lg:sticky lg:top-4">
      <div className="rounded-lg border bg-card">
        <div className="border-b px-4 py-3">
          <div className="text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
            Source of funds
          </div>
          <div className="mt-0.5 text-sm font-semibold">
            {state.reportPurpose === "planned_exchange_sale"
              ? "Planned sale dossier"
              : "Completed transaction dossier"}
          </div>
        </div>
        <nav aria-label="Case stages" className="p-2">
          {state.caseStages.map((entry, index) => {
            const active = state.stage === entry.id;
            const done = stageDone(entry.id, state);
            return (
              <button
                key={entry.id}
                type="button"
                onClick={() => state.goToStage(entry.id)}
                aria-current={active ? "step" : undefined}
                className={[
                  "group relative flex w-full items-start gap-3 rounded-md px-2 py-2.5 text-left transition-colors",
                  active ? "bg-muted/70" : "hover:bg-muted/40",
                ].join(" ")}
              >
                {/* timeline spine */}
                {index < state.caseStages.length - 1 && (
                  <span
                    aria-hidden="true"
                    className="absolute left-[19px] top-[34px] h-[calc(100%-26px)] w-px bg-border"
                  />
                )}
                <span
                  aria-hidden="true"
                  className={[
                    "z-10 mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border text-[11px] font-semibold",
                    done
                      ? "border-emerald-600/40 bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300"
                      : active
                        ? "border-[var(--kb-accent)] text-[var(--kb-accent)]"
                        : "border-border text-muted-foreground",
                  ].join(" ")}
                >
                  {done ? <Check className="size-3.5" /> : index + 1}
                </span>
                <span className="min-w-0">
                  <span
                    className={[
                      "block text-sm font-semibold",
                      active ? "" : "text-foreground/90",
                    ].join(" ")}
                  >
                    {entry.label}
                  </span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {stageStatusLine(entry.id, state)}
                  </span>
                </span>
              </button>
            );
          })}
        </nav>
      </div>

      <div className="rounded-lg border bg-card px-4 py-3">
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
            Case state
          </span>
          {state.report ? (
            exportable ? (
              <span className="inline-flex items-center gap-1 rounded-full border border-emerald-600/30 bg-emerald-50 px-2 py-0.5 text-xs font-medium text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300">
                <ShieldCheck className="size-3.5" aria-hidden="true" />
                Exportable
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-full border border-amber-600/30 bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-700 dark:bg-amber-950/50 dark:text-amber-300">
                <ShieldAlert className="size-3.5" aria-hidden="true" />
                {state.blockers.length} blocker
                {state.blockers.length === 1 ? "" : "s"}
              </span>
            )
          ) : (
            <span className="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
              <CircleDashed className="size-3.5" aria-hidden="true" />
              No report yet
            </span>
          )}
        </div>
        {targetAmount !== null && (
          <div className="mt-2 font-mono text-lg font-semibold tabular-nums">
            {formatBtc(targetAmount, overview?.target_asset || "BTC")}
          </div>
        )}
        <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1.5 text-xs">
          <dt className="text-muted-foreground">Transactions</dt>
          <dd className="text-right font-mono tabular-nums">
            {overview?.transaction_count ?? "–"}
          </dd>
          <dt className="text-muted-foreground">Reviewed links</dt>
          <dd className="text-right font-mono tabular-nums">
            {overview?.link_count ?? "–"}
          </dd>
          <dt className="text-muted-foreground">Root sources</dt>
          <dd className="text-right font-mono tabular-nums">
            {overview?.root_source_count ?? "–"}
          </dd>
          <dt className="text-muted-foreground">Evidence items</dt>
          <dd className="text-right font-mono tabular-nums">
            {state.evidence.length}
          </dd>
          <dt className="text-muted-foreground">Warnings</dt>
          <dd className="text-right font-mono tabular-nums">
            {state.warnings.length}
          </dd>
        </dl>
        <p className="mt-3 border-t pt-2 text-[11px] leading-snug text-muted-foreground">
          Reviewed local evidence only. Nothing leaves this device until you
          export a frozen case.
        </p>
      </div>
    </aside>
  );
}
