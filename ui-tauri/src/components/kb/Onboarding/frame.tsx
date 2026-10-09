import type { ReactNode } from "react";
import { ArrowLeft, ArrowRight, Loader2, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * One step's page: an eyebrow with the position, a large title, an optional
 * lead that explains what the decision affects, then the step's own content.
 * Every step renders through this, so the flow keeps one rhythm from start to
 * review instead of switching layouts between steps.
 */
export const OnboardingStepPage = ({
  eyebrow,
  title,
  lead,
  className,
  children,
}: {
  eyebrow?: string;
  title: string;
  lead?: ReactNode;
  className?: string;
  children: ReactNode;
}) => (
  <div
    className={cn(
      "mx-auto flex w-full max-w-xl flex-col gap-7 animate-in fade-in-0 slide-in-from-bottom-2 duration-300",
      className,
    )}
  >
    <header className="space-y-2.5">
      {/* Below `md` the setup's compact progress bar already names the
          position, so the eyebrow would repeat it. */}
      {eyebrow && (
        <p className="hidden font-mono text-2xs font-medium uppercase tracking-[0.14em] text-ink-3 md:block">
          {eyebrow}
        </p>
      )}
      <h1 className="text-3xl font-semibold tracking-tight text-balance text-ink">
        {title}
      </h1>
      {lead && (
        <p className="text-sm leading-6 text-pretty text-ink-2">{lead}</p>
      )}
    </header>
    {children}
  </div>
);

/** A labelled group of fields inside a step. */
export const OnboardingSection = ({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) => (
  <section className={cn("space-y-3", className)}>
    <h2 className="font-mono text-2xs font-medium uppercase tracking-[0.14em] text-ink-3">
      {title}
    </h2>
    {children}
  </section>
);

/** A quiet footnote with a leading icon, for what applies whatever is picked. */
export const OnboardingNote = ({
  icon: Icon,
  children,
}: {
  icon: LucideIcon;
  children: ReactNode;
}) => (
  <div className="flex items-start gap-3 text-xs leading-5 text-ink-2">
    <Icon className="mt-0.5 size-4 shrink-0 text-ink-3" aria-hidden="true" />
    <p>{children}</p>
  </div>
);

/**
 * The step's floating action bar: Back on the left, the primary action on the
 * right. It sticks to the bottom of the scroll pane as a raised pill, so the
 * primary action stays reachable on tall steps while the fields scroll by.
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
    <div className="sticky bottom-0 z-10 -mx-2 mt-1 pb-(--kb-page-gutter)">
      {/* Opaque, not glass: fields scroll beneath this bar, and any frosting
          let their text show through behind its labels. */}
      <div className="flex items-center justify-between gap-3 rounded-(--kb-radius-card) border bg-popover p-2 shadow-lg">
        {goBack ? (
          <Button type="button" variant="ghost" onClick={goBack}>
            <ArrowLeft aria-hidden="true" />
            {t("common:actions.back")}
          </Button>
        ) : (
          <span />
        )}
        <Button
          type="submit"
          className="min-w-36"
          disabled={disabled || busy}
        >
          {busy ? (
            <>
              <Loader2 className="animate-spin" aria-hidden="true" />
              {busyLabel ?? label}
            </>
          ) : (
            <>
              {label ?? t("common:actions.continue")}
              <ArrowRight aria-hidden="true" />
            </>
          )}
        </Button>
      </div>
    </div>
  );
};
