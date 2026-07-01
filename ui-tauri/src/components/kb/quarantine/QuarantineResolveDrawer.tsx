import { ArrowRight, CheckCircle2, Loader2, ListChecks, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { cn } from "@/lib/utils";

import type { QuarantineResolvePlan, QuarantineResolveStep } from "./model";

interface QuarantineResolveDrawerProps {
  open: boolean;
  plan: QuarantineResolvePlan;
  isProcessingJournals: boolean;
  onOpenChange: (open: boolean) => void;
  onRunStep: (step: QuarantineResolveStep) => void;
}

export function QuarantineResolveDrawer({
  open,
  plan,
  isProcessingJournals,
  onOpenChange,
  onRunStep,
}: QuarantineResolveDrawerProps) {
  const { t } = useTranslation(["journals", "common"]);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-[92vw] gap-0 p-0 sm:max-w-xl">
        <SheetHeader className="border-b px-5 py-4">
          <div className="flex items-center gap-2 text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">
            <Sparkles className="size-3.5" aria-hidden="true" />
            {t("quarantine.resolvePlan.eyebrow")}
          </div>
          <SheetTitle className="text-xl">
            {t("quarantine.resolvePlan.title")}
          </SheetTitle>
          <SheetDescription>{t("quarantine.resolvePlan.description")}</SheetDescription>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Badge variant="outline" className="rounded-md">
              {plan.summary}
            </Badge>
            {plan.blockedCount ? (
              <Badge
                variant="outline"
                className="rounded-md border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300"
              >
                {t("journals:quarantine.resolvePlan.blocked", {
                  count: plan.blockedCount,
                })}
              </Badge>
            ) : null}
            {plan.actionableCount ? (
              <Badge
                variant="outline"
                className="rounded-md border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
              >
                {t("journals:quarantine.resolvePlan.actionable", {
                  count: plan.actionableCount,
                })}
              </Badge>
            ) : null}
          </div>
        </SheetHeader>

        <ScrollArea className="min-h-0 flex-1">
          <div className="space-y-3 p-5">
            {plan.steps.length ? (
              plan.steps.map((step, index) => (
                <ResolveStepCard
                  key={step.id}
                  step={step}
                  index={index}
                  isProcessingJournals={isProcessingJournals}
                  onRunStep={onRunStep}
                />
              ))
            ) : (
              <div className="rounded-md border border-dashed border-muted-foreground/40 px-4 py-8 text-center text-sm text-muted-foreground">
                {t("journals:quarantine.resolvePlan.empty")}
              </div>
            )}
          </div>
        </ScrollArea>

        <SheetFooter className="border-t px-5 py-4">
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:actions.close")}
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}

function ResolveStepCard({
  step,
  index,
  isProcessingJournals,
  onRunStep,
}: {
  step: QuarantineResolveStep;
  index: number;
  isProcessingJournals: boolean;
  onRunStep: (step: QuarantineResolveStep) => void;
}) {
  const { t } = useTranslation("journals");
  const processStep = step.actionKind === "process-journals";
  const disabled =
    step.actionKind === "none" || (processStep && isProcessingJournals);
  return (
    <section
      className={cn(
        "rounded-lg border bg-card p-4",
        step.tone === "alert" && "border-red-500/25 bg-red-500/[0.04]",
        step.tone === "warning" && "border-amber-500/25 bg-amber-500/[0.05]",
        step.tone === "good" && "border-emerald-500/25 bg-emerald-500/[0.04]",
      )}
    >
      <div className="flex items-start gap-3">
        <span
          className={cn(
            "flex size-7 shrink-0 items-center justify-center rounded-md text-xs font-semibold ring-1 ring-inset",
            step.tone === "alert" &&
              "bg-red-500/10 text-red-700 ring-red-500/20 dark:text-red-300",
            step.tone === "warning" &&
              "bg-amber-500/10 text-amber-700 ring-amber-500/20 dark:text-amber-300",
            step.tone === "good" &&
              "bg-emerald-500/10 text-emerald-700 ring-emerald-500/20 dark:text-emerald-300",
            step.tone === "neutral" && "bg-muted text-muted-foreground",
          )}
          aria-hidden="true"
        >
          {processStep ? <CheckCircle2 className="size-4" /> : index + 1}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 className="text-sm font-semibold">{step.title}</h3>
            <Badge variant="outline" className="rounded-md">
              {step.count}
            </Badge>
          </div>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            {step.detail}
          </p>

          {step.previewRows.length ? (
            <div className="mt-3 space-y-1.5">
              <p className="text-[10px] font-medium tracking-[0.14em] text-muted-foreground uppercase">
                {t("quarantine.resolvePlan.affectedRows")}
              </p>
              {step.previewRows.map((row) => (
                <div
                  key={row.rowKey}
                  className="grid grid-cols-[1fr_auto] gap-3 rounded-md border bg-background/60 px-2.5 py-2 text-xs"
                >
                  <div className="min-w-0">
                    <p className="truncate font-medium">{row.event}</p>
                    <p className="truncate text-[11px] text-muted-foreground">
                      {row.id} · {row.account}
                    </p>
                  </div>
                  <span className="self-center font-mono text-[11px] text-muted-foreground">
                    {row.amount}
                  </span>
                </div>
              ))}
              {step.count > step.previewRows.length ? (
                <p className="text-[11px] text-muted-foreground">
                  {t("quarantine.resolvePlan.moreRows", {
                    count: step.count - step.previewRows.length,
                  })}
                </p>
              ) : null}
            </div>
          ) : null}

          <Button
            type="button"
            size="sm"
            className="mt-3 h-8 gap-1.5"
            variant={processStep ? "default" : "outline"}
            disabled={disabled}
            onClick={() => onRunStep(step)}
          >
            {isProcessingJournals && processStep ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : processStep ? (
              <ListChecks className="size-3.5" aria-hidden="true" />
            ) : null}
            {step.actionLabel}
            {!processStep ? <ArrowRight className="size-3.5" aria-hidden="true" /> : null}
          </Button>
        </div>
      </div>
    </section>
  );
}
