import { useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ShieldCheck } from "lucide-react";

import { Wordmark } from "@/components/kb/Wordmark";
import { Button } from "@/components/ui/button";
import {
  activateImportProject,
  canImportProjects,
  clearImportProject,
  getTransport,
  selectImportProjectDirectory,
  type ImportProjectSelection,
} from "@/daemon/transport";
import { cn } from "@/lib/utils";
import { useUiStore, type DataMode, type Identity } from "@/store/ui";
import { setSessionUnlockPassphrase } from "@/store/sessionLock";
import type { ProfilesSnapshot } from "@/mocks/profiles";

import {
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  DEFAULT_FORM,
  GAINS_ALGORITHM_DEFAULTS,
  aiBaseUrlHint,
  backendEndpointHint,
  databasePassphraseHint,
  gainsAlgorithmsFor,
  parseTaxLongTermDays,
} from "./constants";
import { AiStep } from "./steps/AiStep";
import { ConnectionsStep } from "./steps/ConnectionsStep";
import { DatabaseStep } from "./steps/DatabaseStep";
import { IdentityStep } from "./steps/IdentityStep";
import { ImportProjectPanel } from "./ImportProjectPanel";
import { StartChoicePanel } from "./StartChoicePanel";
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
      Boolean(form.workspace.trim() && form.profile.trim()),
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
        return Boolean(
          form.backendName.trim() &&
            backendEndpointHint(form.backendKind, form.backendUrl) === null,
        );
      }
      return true;
    },
  },
  {
    component: AiStep,
    isComplete: (form) => {
      if (form.aiSetupMode !== "disabled") {
        if (aiBaseUrlHint(form.aiBaseUrl) !== null) return false;
      }
      if (form.aiSetupMode === "remote") {
        return Boolean(
          form.aiProviderName.trim() &&
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
  name: "mock books",
  workspace: "Demo Ledger",
  country: "AT",
  encrypted: false,
  profile: "mock books",
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
  const dataMode = useUiStore((state) => state.dataMode);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const preImportDataModeRef = useRef<DataMode | null>(null);
  const [flowMode, setFlowMode] = useState<"start" | "setup">(
    customSteps ? "setup" : "start",
  );
  const [currentStep, setCurrentStep] = useState(0);
  const [form, setForm] = useState<OnboardingForm>(DEFAULT_FORM);
  const [submitting, setSubmitting] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);
  const [importSelection, setImportSelection] =
    useState<ImportProjectSelection | null>(null);
  const [importSnapshot, setImportSnapshot] =
    useState<ProfilesSnapshot | null>(null);
  const [importing, setImporting] = useState(false);
  const [loadingImportProfiles, setLoadingImportProfiles] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const activeSteps = customSteps ?? DEFAULT_STEPS;
  const step = activeSteps[currentStep];
  const importAvailable = canImportProjects();

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
    if (form.databaseMode === "sqlcipher") {
      const envelope = await getTransport("real").invoke({
        kind: "ui.secrets.init",
        args: {
          auth_response: { passphrase_secret: form.databasePassphrase },
          migrate_credentials: form.migrateCredentials,
        },
      });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(
          envelope.error?.message ?? "Could not initialize SQLCipher database.",
        );
      }
      if (envelope.kind === "auth_required") {
        throw new Error("Database passphrase is required.");
      }
    }

    const identity: Identity = {
      name: form.profile.trim() || "main",
      workspace: form.workspace.trim() || "Personal",
      // Legacy field. Today rp2 only ships `at` + `generic` country plugins,
      // so all non-AT picks collapse to "Generic". Prefer `taxCountry` for
      // new callers.
      country: form.taxCountry === "at" ? "AT" : "Generic",
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
    if (submitting) return;
    setFinishError(null);
    if (!step.isComplete(form)) return;
    if (currentStep !== activeSteps.length - 1) {
      setCurrentStep(currentStep + 1);
      return;
    }
    setSubmitting(true);
    void finish()
      .catch((error: unknown) => {
        setFinishError(
          error instanceof Error ? error.message : "Could not finish onboarding.",
        );
      })
      .finally(() => setSubmitting(false));
  };

  const handleGoBack = () => {
    if (currentStep > 0) {
      setCurrentStep(currentStep - 1);
      return;
    }
    if (!customSteps) setFlowMode("start");
  };

  const beginSetup = () => {
    setDataMode("real");
    setFinishError(null);
    setImportError(null);
    setFlowMode("setup");
  };

  const refreshImportedProfiles = async () => {
    setLoadingImportProfiles(true);
    setImportError(null);
    try {
      const envelope = await getTransport("real").invoke<ProfilesSnapshot>({
        kind: "ui.profiles.snapshot",
      });
      if (envelope.kind === "auth_required") {
        throw new Error("Database passphrase is required.");
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? "Could not load books.");
      }
      setImportSnapshot(
        envelope.data ?? { workspaces: [], activeProfileId: "" },
      );
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Could not load books.";
      setImportError(message);
      throw error;
    } finally {
      setLoadingImportProfiles(false);
    }
  };

  const unlockAndLoadImportedProfiles = async (
    selection: ImportProjectSelection,
    passphrase: string | null,
  ) => {
    setLoadingImportProfiles(true);
    setImportError(null);
    try {
      const envelope = await getTransport("real").invoke({
        kind: "daemon.unlock",
        args: {
          require_existing_project: true,
          ...(passphrase
            ? { auth_response: { passphrase_secret: passphrase } }
            : {}),
        },
      });
      if (envelope.kind === "auth_required") {
        return;
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? "Could not open project.");
      }
      await setSessionUnlockPassphrase(selection.encrypted ? passphrase : null);
      await refreshImportedProfiles();
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Could not open project.";
      setImportError(message);
      throw error;
    } finally {
      setLoadingImportProfiles(false);
    }
  };

  const beginImport = () => {
    if (importing || submitting) return;
    setFinishError(null);
    setImportError(null);
    setImportSnapshot(null);
    setImporting(true);
    preImportDataModeRef.current = dataMode;
    let activatedImport = false;
    void (async () => {
      const picked = await selectImportProjectDirectory();
      if (!picked) {
        preImportDataModeRef.current = null;
        return;
      }
      setDataMode("real");
      const activated = await activateImportProject(picked.dataRoot);
      setImportSelection(activated);
      activatedImport = true;
      if (!activated.encrypted) {
        await unlockAndLoadImportedProfiles(activated, null);
      }
    })()
      .catch((error: unknown) => {
        if (!activatedImport) {
          const previousDataMode = preImportDataModeRef.current;
          if (previousDataMode) {
            setDataMode(previousDataMode);
          }
          preImportDataModeRef.current = null;
          void setSessionUnlockPassphrase(null);
        }
        setImportError(
          error instanceof Error ? error.message : "Could not import project.",
        );
      })
      .finally(() => setImporting(false));
  };

  const cancelImport = () => {
    setImportError(null);
    setLoadingImportProfiles(true);
    void clearImportProject()
      .then(async () => {
        const previousDataMode = preImportDataModeRef.current;
        await setSessionUnlockPassphrase(null);
        if (previousDataMode) {
          setDataMode(previousDataMode);
        }
        preImportDataModeRef.current = null;
        setImportSelection(null);
        setImportSnapshot(null);
        setFlowMode("start");
      })
      .catch((error: unknown) => {
        void setSessionUnlockPassphrase(null);
        setImportError(
          error instanceof Error
            ? error.message
            : "Could not return to the default project root.",
        );
      })
      .finally(() => setLoadingImportProfiles(false));
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

        {importSelection ? (
          <ImportProjectPanel
            selection={importSelection}
            encrypted={importSelection.encrypted}
            snapshot={importSnapshot}
            loadingProfiles={loadingImportProfiles}
            error={importError}
            onCancel={cancelImport}
            onRefreshProfiles={refreshImportedProfiles}
            onUnlock={(passphrase) =>
              unlockAndLoadImportedProfiles(importSelection, passphrase)
            }
          />
        ) : flowMode === "start" ? (
          <StartChoicePanel
            importAvailable={importAvailable}
            importing={importing}
            onSetup={beginSetup}
            onImport={beginImport}
          />
        ) : (
          <step.component
            form={form}
            update={update}
            onSubmit={handleSubmit}
            canContinue={step.isComplete(form) && !submitting}
            currentStep={currentStep}
            totalSteps={activeSteps.length}
            goBack={handleGoBack}
          />
        )}

        {(finishError || (!importSelection && importError)) && (
          <div className="max-w-2xl rounded-md border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {finishError ?? importError}
          </div>
        )}

        <div className="flex flex-wrap items-center justify-center gap-4 text-xs text-ink-3">
          <span>Private keys never enter Kassiber.</span>
          <span>State stays under ~/.kassiber unless overridden.</span>
          <span>Run backups before tracking real funds.</span>
        </div>
      </div>
    </section>
  );
};
