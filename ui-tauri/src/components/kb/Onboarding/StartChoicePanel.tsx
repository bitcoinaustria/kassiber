import {
  ArrowRight,
  BookOpen,
  Database,
  FolderOpen,
  LockKeyhole,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";

import {
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "./frame";

interface StartChoicePanelProps {
  importAvailable: boolean;
  importing: boolean;
  onSetup: () => void;
  onImport: () => void;
}

const START_HIGHLIGHTS: Array<{
  icon: LucideIcon;
  title: string;
  body: string;
}> = [
  {
    icon: Database,
    title: "One local database",
    body: "~/.kassiber by default, or an imported ledger root.",
  },
  {
    icon: LockKeyhole,
    title: "Encrypted by default",
    body: "SQLCipher protects real ledgers at rest.",
  },
  {
    icon: FolderOpen,
    title: "Import stays local",
    body: "Existing books are listed after the selected root opens.",
  },
];

export function StartChoicePanel({
  importAvailable,
  importing,
  onSetup,
  onImport,
}: StartChoicePanelProps) {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Open a local ledger"
        eyebrow="Start"
        currentStep={0}
        totalSteps={5}
        showProgress={false}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-3">
            <button
              type="button"
              onClick={onSetup}
              className="group flex min-h-[132px] w-full items-start gap-4 rounded-lg border border-ink bg-ink p-4 text-left text-paper transition hover:bg-ink/95"
            >
              <span className="flex size-10 shrink-0 items-center justify-center rounded-md border border-paper/20 bg-paper text-ink">
                <BookOpen className="size-5" aria-hidden="true" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-base font-semibold">
                  Set up new ledger
                </span>
                <span className="mt-2 block text-sm leading-6 text-paper/75">
                  Create a new local Kassiber database with sensible defaults.
                </span>
              </span>
              <ArrowRight
                className="mt-1 size-5 shrink-0 transition-transform group-hover:translate-x-0.5"
                aria-hidden="true"
              />
            </button>

            <button
              type="button"
              disabled={!importAvailable || importing}
              onClick={onImport}
              title={
                importAvailable
                  ? "Import an existing local Kassiber ledger"
                  : "Ledger import is available in the desktop app"
              }
              className={cn(
                "group flex min-h-[132px] w-full items-start gap-4 rounded-lg border p-4 text-left transition",
                importAvailable
                  ? "border-line bg-paper hover:bg-paper-2"
                  : "cursor-not-allowed border-line bg-paper-2 opacity-70",
              )}
            >
              <span className="flex size-10 shrink-0 items-center justify-center rounded-md border border-line bg-paper">
                <FolderOpen className="size-5 text-ink" aria-hidden="true" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-base font-semibold text-ink">
                  {importing ? "Opening ledger..." : "Import existing ledger"}
                </span>
                <span className="mt-2 block text-sm leading-6 text-ink-2">
                  Open a Kassiber state root or data folder and choose local
                  books.
                </span>
                {!importAvailable && (
                  <span className="mt-3 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-ink-3">
                    desktop app only
                  </span>
                )}
              </span>
              <ArrowRight
                className={cn(
                  "mt-1 size-5 shrink-0 text-ink transition-transform",
                  importAvailable && "group-hover:translate-x-0.5",
                )}
                aria-hidden="true"
              />
            </button>
          </div>

          <div className="flex items-start gap-3 rounded-lg border border-line bg-paper-2 p-3 text-xs leading-5 text-ink-2">
            <ShieldCheck className="mt-0.5 size-4 shrink-0 text-ink" />
            <p className="m-0">
              Kassiber stores watch-only accounting data locally. Private keys
              never enter the app.
            </p>
          </div>
        </div>
      </OnboardingStepLeftWrapper>

      <OnboardingStepRightWrapper className="p-6">
        <div className="flex h-full flex-col justify-center gap-4">
          {START_HIGHLIGHTS.map(({ icon: Icon, title, body }) => (
            <div
              key={title}
              className="flex items-start gap-3 rounded-lg border border-line bg-paper p-4"
            >
              <span className="flex size-9 shrink-0 items-center justify-center rounded-md bg-ink text-paper">
                <Icon className="size-4" aria-hidden="true" />
              </span>
              <div>
                <p className="m-0 text-sm font-semibold text-ink">{title}</p>
                <p className="m-0 mt-1 text-xs leading-5 text-ink-2">{body}</p>
              </div>
            </div>
          ))}
        </div>
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
}
