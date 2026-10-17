import type { ReactNode } from "react";
import { ArrowLeft, ArrowRight, Loader2, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * One step's page: a large title, an optional lead that explains what the
 * decision affects, then the step's own content. The rail's stepper (or the
 * compact progress bar) already names the position, so the page doesn't.
 * Every step renders through this, so the flow keeps one rhythm from start to
 * review instead of switching layouts between steps.
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
      "mx-auto flex w-full max-w-xl flex-col gap-8 animate-in fade-in-0 slide-in-from-bottom-2 duration-300",
      className,
    )}
  >
    <header className="space-y-3">
      <h1 className="text-3xl font-semibold tracking-tight text-balance text-ink">
        {title}
      </h1>
      {lead && (
        <p className="text-base leading-7 text-pretty text-ink-2">{lead}</p>
      )}
    </header>
    {children}
  </div>
);

/**
 * A secondary group of fields below the step's main decision: a hairline and
 * a plain heading rather than a card, so it reads as less important.
 */
export const OnboardingSection = ({
  title,
  description,
  children,
  className,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) => (
  <section
    className={cn("space-y-4 border-t border-border pt-7", className)}
  >
    <header className="space-y-1">
      <h2 className="text-base font-semibold text-ink">{title}</h2>
      {description && (
        <p className="text-sm leading-6 text-ink-2">{description}</p>
      )}
    </header>
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
  <div className="flex items-start gap-3 text-xs leading-5 text-ink-3">
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
