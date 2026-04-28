import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ShieldCheck } from "lucide-react";

import { Wordmark } from "@/components/kb/Wordmark";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useUiStore, type Identity } from "@/store/ui";
import { setSessionUnlockPassphrase } from "@/store/sessionLock";

import {
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  DEFAULT_FORM,
  GAINS_ALGORITHM_DEFAULTS,
  databasePassphraseHint,
  gainsAlgorithmsFor,
  parseTaxLongTermDays,
} from "./constants";
import { AiStep } from "./steps/AiStep";
import { ConnectionsStep } from "./steps/ConnectionsStep";
import { DatabaseStep } from "./steps/DatabaseStep";
import { IdentityStep } from "./steps/IdentityStep";
import { TaxStep } from "./steps/TaxStep";
import type { OnboardingForm, OnboardingStep } from "./types";

interface OnboardingProps {
  className?: string;
  steps?: OnboardingStep[];
}

const DEFAULT_STEPS: OnboardingStep[] = [
  {
    component: IdentityStep,
    isComplete: (form) =>
      Boolean(form.name.trim() && form.workspace.trim() && form.profile.trim()),
  },
  {
    component: TaxStep,
    isComplete: (form) =>
      form.taxCountry === "at" ||
      parseTaxLongTermDays(form.taxLongTermDays) !== null,
  },
  {
    component: ConnectionsStep,
    isComplete: (form) => {
      if (form.backendSetupMode === "skip") {
        return form.skipBackendsAcknowledged;
      }
      if (form.backendSetupMode === "custom") {
        return Boolean(form.backendName.trim() && form.backendUrl.trim());
      }
      return true;
    },
  },
  {
    component: AiStep,
    isComplete: (form) => {
      if (form.aiSetupMode === "remote") {
        return Boolean(
          form.aiProviderName.trim() &&
            form.aiBaseUrl.trim() &&
            form.aiRemoteAcknowledged,
        );
      }
      return true;
    },
  },
  {
    component: DatabaseStep,
    isComplete: (form) =>
      form.databaseMode === "plaintext"
        ? form.plaintextAcknowledged
        : form.recoveryAcknowledged &&
          databasePassphraseHint(
            form.databasePassphrase,
            form.databasePassphraseConfirm,
          ) === null,
  },
];

const DEV_MOCK_IDENTITY: Identity = {
  name: "mock profile",
  workspace: "Demo Workspace",
  country: "AT",
  encrypted: false,
  profile: "mock",
  taxCountry: "at",
  fiatCurrency: "EUR",
  taxLongTermDays: 0,
  gainsAlgorithm: "MOVING_AVERAGE_AT",
  databaseMode: "plaintext",
  migrateCredentials: false,
  backendSetupMode: "skip",
  aiSetupMode: "local",
  aiProviderKind: "local",
  aiProviderName: DEFAULT_AI_PROVIDER_NAME,
  aiBaseUrl: DEFAULT_AI_BASE_URL,
};

export const Onboarding = ({ className, steps: customSteps }: OnboardingProps) => {
  const navigate = useNavigate();
  const setIdentity = useUiStore((state) => state.setIdentity);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const [currentStep, setCurrentStep] = useState(0);
  const [form, setForm] = useState<OnboardingForm>(DEFAULT_FORM);
  const activeSteps = customSteps ?? DEFAULT_STEPS;
  const step = activeSteps[currentStep];

  const update = <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const finish = async () => {
    // Step gates already enforce these — clamp defensively in case state
    // arrives via an injected `customSteps` override (used by tests).
    const allowedAlgorithms = gainsAlgorithmsFor(form.taxCountry);
    const gainsAlgorithm = allowedAlgorithms.includes(form.gainsAlgorithm)
      ? form.gainsAlgorithm
      : GAINS_ALGORITHM_DEFAULTS[form.taxCountry];
    const taxLongTermDays =
      form.taxCountry === "at"
        ? 0
        : (parseTaxLongTermDays(form.taxLongTermDays) ?? 365);
    const identity: Identity = {
      name: form.name.trim(),
      workspace: form.workspace.trim() || "Personal",
      // Legacy field. Today rp2 only ships `at` + `generic` country plugins,
      // so all non-AT picks collapse to "Generic". Prefer `taxCountry` for
      // new callers.
      country: form.taxCountry === "at" ? "AT" : "Generic",
      // Intent only until the native SQLCipher passphrase handoff lands; see
      // `Identity` JSDoc in store/ui.ts.
      encrypted: form.databaseMode === "sqlcipher",
      profile: form.profile.trim() || "main",
      taxCountry: form.taxCountry,
      fiatCurrency: form.fiatCurrency,
      taxLongTermDays,
      gainsAlgorithm,
      databaseMode: form.databaseMode,
      migrateCredentials: form.migrateCredentials,
      backendSetupMode: form.backendSetupMode,
      backendKind:
        form.backendSetupMode === "custom" ? form.backendKind : undefined,
      backendName:
        form.backendSetupMode === "custom"
          ? form.backendName.trim() || "custom"
          : undefined,
      backendUrl:
        form.backendSetupMode === "custom"
          ? form.backendUrl.trim()
          : undefined,
      aiSetupMode: form.aiSetupMode,
      aiProviderKind:
        form.aiSetupMode === "disabled" ? undefined : form.aiProviderKind,
      aiProviderName:
        form.aiSetupMode === "disabled"
          ? undefined
          : form.aiProviderName.trim() || DEFAULT_AI_PROVIDER_NAME,
      aiBaseUrl:
        form.aiSetupMode === "disabled"
          ? undefined
          : form.aiBaseUrl.trim() || DEFAULT_AI_BASE_URL,
    };
    await setSessionUnlockPassphrase(
      form.databaseMode === "sqlcipher" ? form.databasePassphrase : null,
    );
    setIdentity(identity);
    void navigate({ to: "/overview" });
  };

  const handleSubmit = () => {
    if (!step.isComplete(form)) return;
    if (currentStep !== activeSteps.length - 1) {
      setCurrentStep(currentStep + 1);
      return;
    }
    void finish();
  };

  const handleGoBack = () => {
    if (currentStep > 0) setCurrentStep(currentStep - 1);
  };

  const skipToMockPreview = () => {
    setDataMode("mock");
    void setSessionUnlockPassphrase(null);
    setIdentity(DEV_MOCK_IDENTITY);
    void navigate({ to: "/overview" });
  };

  return (
    <section className="min-h-screen bg-paper px-4 py-6 text-ink sm:px-8 lg:px-10">
      <div
        className={cn(
          "mx-auto flex max-w-7xl flex-col items-center gap-8",
          className,
        )}
      >
        <div className="flex w-full items-center justify-between gap-4">
          <Wordmark size={22} />
          <div className="flex items-center gap-3">
            <div className="hidden items-center gap-2 text-xs text-ink-2 sm:flex">
              <ShieldCheck className="size-4" />
              Local-first · watch-only · SQLCipher-aware
            </div>
            {import.meta.env.DEV && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={skipToMockPreview}
              >
                Mock-only preview
              </Button>
            )}
          </div>
        </div>

        <step.component
          form={form}
          update={update}
          onSubmit={handleSubmit}
          currentStep={currentStep}
          totalSteps={activeSteps.length}
          goBack={handleGoBack}
        />

        <div className="flex flex-wrap items-center justify-center gap-4 text-xs text-ink-3">
          <span>Private keys never enter Kassiber.</span>
          <span>State stays under ~/.kassiber unless overridden.</span>
          <span>Run backups before tracking real funds.</span>
        </div>
      </div>
    </section>
  );
};
