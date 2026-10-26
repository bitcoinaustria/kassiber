/*
 * Setup's page primitives, after T3 Code's welcome wizard and settings layout
 * (apps/web/src/components/onboarding/WelcomeWizard.tsx and
 * settings/settingsLayout.tsx, MIT, Copyright (c) 2026 T3 Tools Inc.): a
 * plain title and muted lead, settings groups of title-left/control-right
 * rows, and a footer that pairs a quiet note with the primary action.
 */
import type { ReactNode } from "react";
import { ArrowLeft, ArrowRight, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * One step: a title, an optional lead that explains what the decision
 * affects, then the step's own content. The wizard's step track already names
 * the position, so the page doesn't.
 */
export const OnboardingStepPage = ({
  title,
  lead,
  className,
  children,
}: {
  title: string;
  lead?: ReactNode;
  className?: string;
  children: ReactNode;
}) => (
  <div
    className={cn(
      "flex w-full flex-col gap-6 animate-in fade-in-0 duration-200",
      className,
    )}
  >
    <header>
      <h1 className="text-2xl font-semibold tracking-tight text-balance text-ink">
        {title}
      </h1>
      {lead && (
        <p className="mt-2.5 text-sm leading-relaxed text-pretty text-ink-2">
          {lead}
        </p>
      )}
    </header>
    {children}
  </div>
);

/**
 * A secondary group below the step's main decision: a dimmed heading over a
 * settings group, so it reads as less important than the options above.
 */
export const OnboardingSection = ({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) => (
  <section className={cn("space-y-2.5", className)}>
    <h2 className="px-1 text-sm font-normal text-ink/70">{title}</h2>
    <OnboardingGroup>{children}</OnboardingGroup>
  </section>
);

/** Rows in one outlined group, divided by hairlines. */
export const OnboardingGroup = ({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) => (
  <div
    className={cn(
      "rounded-(--kb-radius-card) border border-border/70 bg-card/40 [&>*+*]:border-t [&>*+*]:border-border/50",
      className,
    )}
  >
    {children}
  </div>
);

/**
 * A settings row: title, description and status on the left, the control on
 * the right; stacked on narrow widths.
 */
export const OnboardingRow = ({
  title,
  description,
  status,
  control,
  disabled = false,
}: {
  title: ReactNode;
  description?: ReactNode;
  status?: ReactNode;
  control?: ReactNode;
  disabled?: boolean;
}) => (
  <div
    aria-disabled={disabled || undefined}
    className="px-4 py-3 aria-disabled:opacity-60"
  >
    <div className="flex flex-col gap-3 sm:grid sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-8">
      <div className="min-w-0 space-y-1">
        <h3 className="text-sm font-medium text-ink">{title}</h3>
        {description && (
          <div className="max-w-xl text-xs leading-normal text-ink-3">
            {description}
          </div>
        )}
        {status && <div className="pt-0.5 text-xs text-ink-3">{status}</div>}
      </div>
      {control && (
        <div className="flex shrink-0 items-center gap-2 sm:justify-end">
          {control}
        </div>
      )}
    </div>
  </div>
);

/**
 * The step's footer: what setup itself does on the network on the left, Back
 * and the primary action on the right.
 */
export const OnboardingStepActions = ({
  goBack,
  label,
  busyLabel,
  busy = false,
  disabled = false,
}: {
  goBack?: () => void;
  label?: string;
  busyLabel?: string;
  busy?: boolean;
  disabled?: boolean;
}) => {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <div className="mt-2 flex flex-col-reverse gap-4 border-t border-border/60 pt-5 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
      <p className="min-w-0 text-xs leading-relaxed text-ink-3">
        {t("frame.setupConnects")}
      </p>
      <div className="flex shrink-0 items-center justify-end gap-2">
        {goBack && (
          <Button
            type="button"
            variant="ghost"
            className="text-ink-2"
            onClick={goBack}
          >
            <ArrowLeft className="size-3.5" aria-hidden="true" />
            {t("common:actions.back")}
          </Button>
        )}
        <Button type="submit" disabled={disabled || busy}>
          {busy ? (
            <>
              <Loader2 className="animate-spin" aria-hidden="true" />
              {busyLabel ?? label}
            </>
          ) : (
            <>
              {label ?? t("common:actions.continue")}
              <ArrowRight className="size-3.5" aria-hidden="true" />
            </>
          )}
        </Button>
      </div>
    </div>
  );
};
