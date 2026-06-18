import { ArrowRight } from "lucide-react";
import * as React from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useDaemonMutation } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useUiStore } from "@/store/ui";

import {
  buildSplitPayoutArgs,
  defaultPayoutAsset,
  PAYOUT_ASSETS,
  splitPayoutValidation,
  type SplitPayoutPolicy,
} from "./transactionSplitPayout";

export interface TransactionSplitPayoutCardProps {
  transactionId: string;
  sourceAsset: string;
  outboundBtc: number;
}

/**
 * Resolves a `transfer_fee_implausible` quarantine: one on-chain spend that
 * partly returned to an owned wallet (self-transfer) and partly went to a swap
 * or direct payout. Records the payout portion via `ui.transfers.payouts.create`
 * with `out_amount`, then reprocesses journals so the remainder books as a
 * same-transaction self-transfer instead of a giant implausible fee.
 */
export function TransactionSplitPayoutCard({
  transactionId,
  sourceAsset,
  outboundBtc,
}: TransactionSplitPayoutCardProps) {
  const addNotification = useUiStore((state) => state.addNotification);
  const createPayout = useDaemonMutation("ui.transfers.payouts.create");
  const { runJournalProcessing } = useJournalProcessingAction();

  const [outAmount, setOutAmount] = React.useState("");
  const [payoutAsset, setPayoutAsset] = React.useState<string>(
    defaultPayoutAsset(sourceAsset),
  );
  const [payoutAmount, setPayoutAmount] = React.useState("");
  const [policy, setPolicy] = React.useState<SplitPayoutPolicy>(
    "carrying-value",
  );
  const [counterparty, setCounterparty] = React.useState("");
  const [notes, setNotes] = React.useState("");

  const { outError, payoutError, ready } = splitPayoutValidation({
    outAmount,
    payoutAmount,
    outboundBtc,
  });
  const canSubmit = ready && !createPayout.isPending;

  const submit = async () => {
    if (!canSubmit) return;
    try {
      await createPayout.mutateAsync(
        buildSplitPayoutArgs({
          transactionId,
          outAmount,
          payoutAsset,
          payoutAmount,
          policy,
          counterparty,
          notes,
        }),
      );
      addNotification({
        title: "Split payout recorded",
        body: "Saved the payout portion; reprocessing journals to apply.",
        tone: "success",
      });
      runJournalProcessing();
    } catch (error) {
      addNotification({
        title: "Could not record split payout",
        body:
          error instanceof Error
            ? error.message
            : "Kassiber could not save the split transfer/swap resolution.",
        tone: "error",
      });
    }
  };

  return (
    <div className="space-y-3 rounded-md border bg-background p-4">
      <div className="space-y-1">
        <p className="text-sm font-medium">Resolve split transfer / swap</p>
        <p className="text-xs text-muted-foreground">
          Part of this {sourceAsset} spend returned to an owned wallet and part
          went to a swap or direct payout. Record the payout portion; the
          remainder resolves as a same-transaction self-transfer instead of an
          implausible fee.
        </p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="grid gap-1.5">
          <Label htmlFor="split-out-amount">
            Swap/payout portion ({sourceAsset})
          </Label>
          <Input
            id="split-out-amount"
            inputMode="decimal"
            placeholder={outboundBtc ? String(outboundBtc) : "0.00000000"}
            value={outAmount}
            onChange={(event) => setOutAmount(event.target.value)}
          />
          <span className="text-[11px] text-muted-foreground">
            Outbound total: {outboundBtc} {sourceAsset}
          </span>
          {outError ? (
            <span className="text-[11px] text-destructive">{outError}</span>
          ) : null}
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="split-payout-asset">Payout asset</Label>
          <Select value={payoutAsset} onValueChange={setPayoutAsset}>
            <SelectTrigger id="split-payout-asset">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PAYOUT_ASSETS.map((asset) => (
                <SelectItem key={asset} value={asset}>
                  {asset}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="split-payout-amount">
            Payout received ({payoutAsset})
          </Label>
          <Input
            id="split-payout-amount"
            inputMode="decimal"
            placeholder="0.00000000"
            value={payoutAmount}
            onChange={(event) => setPayoutAmount(event.target.value)}
          />
          {payoutError ? (
            <span className="text-[11px] text-destructive">{payoutError}</span>
          ) : null}
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="split-policy">Tax policy</Label>
          <Select
            value={policy}
            onValueChange={(value) => setPolicy(value as SplitPayoutPolicy)}
          >
            <SelectTrigger id="split-policy">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="carrying-value">
                Carrying value (no disposal)
              </SelectItem>
              <SelectItem value="taxable">Taxable disposal</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="grid gap-1.5">
        <Label htmlFor="split-counterparty">Counterparty (optional)</Label>
        <Input
          id="split-counterparty"
          placeholder="recipient or exchange"
          value={counterparty}
          onChange={(event) => setCounterparty(event.target.value)}
        />
      </div>

      <div className="grid gap-1.5">
        <Label htmlFor="split-notes">Notes (optional)</Label>
        <Textarea
          id="split-notes"
          rows={2}
          value={notes}
          onChange={(event) => setNotes(event.target.value)}
        />
      </div>

      <div className="flex justify-end">
        <Button size="sm" disabled={!canSubmit} onClick={() => void submit()}>
          {createPayout.isPending ? "Saving…" : "Record payout & reprocess"}
          <ArrowRight className="size-3.5" aria-hidden="true" />
        </Button>
      </div>

      <p className="text-[11px] text-muted-foreground">
        Cross-asset carrying value is supported for Austrian books; otherwise
        choose taxable.
      </p>
    </div>
  );
}
