/**
 * Decision card for one pairing candidate ("is this your own transfer?" /
 * "is this one swap?"). Confirms via ``ui.transfers.pair`` with the engine's
 * default kind/policy; both stay editable behind the Details disclosure.
 */

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemonMutation } from "@/daemon/client";
import { currentUiLocale } from "@/lib/localeFormat";
import { useUiStore } from "@/store/ui";
import { formatCustodyMsat } from "../custodyGapsModel";
import { FlowDiagram } from "./FlowDiagram";
import {
  CANDIDATE_WHY_KEYS,
  candidatePresentation,
  walletDisplayName,
  type InboxCandidate,
} from "./inboxModel";

const PAIR_KIND_VALUES = [
  "manual",
  "coinjoin",
  "whirlpool",
  "chain-swap",
  "peg-in",
  "peg-out",
  "reverse-submarine-swap",
  "submarine-swap",
  "swap-refund",
] as const;
const PAIR_POLICY_VALUES = ["carrying-value", "taxable"] as const;

function formatWhen(value: string): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function PairDecisionCard({
  candidate,
  onSettled,
}: {
  candidate: InboxCandidate;
  onSettled?: () => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const addNotification = useUiStore((state) => state.addNotification);
  const [kind, setKind] = useState(candidate.default_kind);
  const [policy, setPolicy] = useState(candidate.default_policy);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const pairMutation = useDaemonMutation("ui.transfers.pair");
  const dismissMutation = useDaemonMutation("ui.transfers.dismiss");
  const busy = pairMutation.isPending || dismissMutation.isPending;

  const presentation = candidatePresentation(candidate);
  const questionKey =
    presentation === "transfer"
      ? "swap.inbox.candidate.questionTransfer"
      : presentation === "layer-transition"
        ? "swap.inbox.candidate.questionMove"
        : "swap.inbox.candidate.questionSwap";
  const yesKey =
    presentation === "transfer"
      ? "swap.inbox.candidate.yesTransfer"
      : presentation === "layer-transition"
        ? "swap.inbox.candidate.yesMove"
        : "swap.inbox.candidate.yesSwap";
  const typeKey =
    presentation === "transfer"
      ? "swap.inbox.type.transfer"
      : presentation === "layer-transition"
        ? "swap.inbox.type.layerTransition"
        : "swap.inbox.type.swap";

  const pair = async () => {
    setActionError(null);
    try {
      await pairMutation.mutateAsync({
        tx_out: candidate.out_id,
        tx_in: candidate.in_id,
        kind,
        policy,
        pair_source: "manual",
        confidence_at_pair: candidate.confidence,
      });
      addNotification({
        title: t("swap.inbox.candidate.pairedTitle"),
        body: t("swap.inbox.candidate.pairedBody"),
        tone: "success",
      });
      onSettled?.();
    } catch (cause) {
      setActionError(
        cause instanceof Error ? cause.message : t("swap.inbox.actionFailed"),
      );
    }
  };

  const dismiss = async () => {
    setActionError(null);
    try {
      await dismissMutation.mutateAsync({
        tx_out: candidate.out_id,
        tx_in: candidate.in_id,
        reason: t("swap.dismissReason"),
      });
      addNotification({
        title: t("swap.inbox.candidate.dismissedTitle"),
        body: t("swap.inbox.candidate.dismissedBody"),
        tone: "info",
      });
      onSettled?.();
    } catch (cause) {
      setActionError(
        cause instanceof Error ? cause.message : t("swap.inbox.actionFailed"),
      );
    }
  };

  return (
    <Card className="gap-4 py-5">
      <CardContent className="space-y-4 px-5">
        <div className="flex flex-wrap items-center gap-1.5">
          <Badge variant="outline">{t(typeKey)}</Badge>
          <Badge variant="outline">
            {candidate.confidence === "exact"
              ? t("swap.confidence.exact")
              : t("swap.confidence.strong")}
          </Badge>
          {candidate.confidence === "exact" && candidate.conflict_size <= 1 ? (
            <Badge>{t("swap.inbox.suggestedBadge")}</Badge>
          ) : null}
          {candidate.conflict_size > 1 ? (
            <Badge variant="destructive">{t("swap.inbox.competingBadge")}</Badge>
          ) : null}
        </div>

        <div>
          <h2 className="text-lg font-semibold">{t(questionKey)}</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {t(CANDIDATE_WHY_KEYS[candidate.method])}
          </p>
        </div>

        <FlowDiagram
          from={{
            label: walletDisplayName(
              candidate.out_wallet_label,
              candidate.out_wallet_kind,
            ),
            sub: formatWhen(candidate.out_occurred_at),
          }}
          to={{
            label: walletDisplayName(
              candidate.in_wallet_label,
              candidate.in_wallet_kind,
            ),
            sub: formatWhen(candidate.in_occurred_at),
          }}
          outAmount={`${formatCustodyMsat(
            candidate.out_amount_msat,
            candidate.out_asset,
          )} → ${formatCustodyMsat(candidate.in_amount_msat, candidate.in_asset)}`}
          fee={
            candidate.swap_fee_msat
              ? t("swap.inbox.candidate.feeNote", {
                  amount: formatCustodyMsat(
                    candidate.swap_fee_msat,
                    candidate.out_asset,
                  ),
                })
              : null
          }
          hideSensitive={hideSensitive}
        />

        {candidate.conflict_size > 1 ? (
          <p className="rounded-md border border-destructive/40 bg-destructive/5 p-2.5 text-sm">
            {t("swap.inbox.candidate.competingNote", {
              count: candidate.conflict_size,
            })}
          </p>
        ) : null}

        <Collapsible open={detailsOpen} onOpenChange={setDetailsOpen}>
          <CollapsibleTrigger className="flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:underline">
            {detailsOpen ? (
              <ChevronDown className="size-3.5" aria-hidden="true" />
            ) : (
              <ChevronRight className="size-3.5" aria-hidden="true" />
            )}
            {t("swap.inbox.candidate.detailsToggle")}
          </CollapsibleTrigger>
          <CollapsibleContent className="mt-2 space-y-3 border-l-2 pl-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label className="text-xs" htmlFor={`kind-${candidate.out_id}`}>
                  {t("swap.detail.pairKind")}
                </Label>
                <Select value={kind} onValueChange={setKind}>
                  <SelectTrigger
                    id={`kind-${candidate.out_id}`}
                    className="h-8 w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PAIR_KIND_VALUES.map((value) => (
                      <SelectItem key={value} value={value}>
                        {value}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1.5">
                <Label
                  className="text-xs"
                  htmlFor={`policy-${candidate.out_id}`}
                >
                  {t("swap.detail.policy")}
                </Label>
                <Select value={policy} onValueChange={setPolicy}>
                  <SelectTrigger
                    id={`policy-${candidate.out_id}`}
                    className="h-8 w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PAIR_POLICY_VALUES.map((value) => (
                      <SelectItem key={value} value={value}>
                        {value}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="space-y-1 text-xs text-muted-foreground">
              <p>
                {t("swap.detail.recordId")}: {candidate.out_id} →{" "}
                {candidate.in_id}
              </p>
            </div>
          </CollapsibleContent>
        </Collapsible>

        {actionError ? (
          <p className="text-sm text-destructive">{actionError}</p>
        ) : null}

        <div className="flex flex-wrap gap-2">
          <Button type="button" onClick={pair} disabled={busy}>
            {pairMutation.isPending
              ? t("swap.inbox.candidate.pairing")
              : t(yesKey)}
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={dismiss}
            disabled={busy}
          >
            {t("swap.inbox.candidate.no")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
