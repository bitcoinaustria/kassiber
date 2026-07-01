// Source of Funds workstation — a case dossier, not a wizard.
//
// Layout: a persistent rail (stage spine + live case state) beside one work
// surface per stage. Target → Trace → Disclose → Export, freely navigable;
// the rail always answers "where does my case stand?".

import { TransactionDetailController } from "@/components/transactions/dashboard/TransactionDetailController";
import { screenShellClassName } from "@/lib/screen-layout";

import { CaseRail } from "./CaseRail";
import {
  DiscloseStage,
  ExportStage,
  TargetStage,
  TraceStage,
} from "./stages";
import { useSourceFundsCase } from "./useSourceFundsCase";

export function SourceFunds() {
  const state = useSourceFundsCase();

  return (
    <div className={screenShellClassName}>
      <div className="grid items-start gap-5 lg:grid-cols-[270px_minmax(0,1fr)]">
        <CaseRail state={state} />
        <main className="min-w-0">
          {state.stage === "target" && <TargetStage state={state} />}
          {state.stage === "trace" && <TraceStage state={state} />}
          {state.stage === "disclose" && <DiscloseStage state={state} />}
          {state.stage === "export" && <ExportStage state={state} />}
        </main>
      </div>
      <TransactionDetailController
        transaction={state.detailTransaction}
        hideSensitive={state.hideSensitive}
        currency={state.currency}
        explorerSettings={state.explorerSettings}
        onOpenChange={(open) => {
          if (!open) state.setDetailTransaction(null);
        }}
      />
    </div>
  );
}
