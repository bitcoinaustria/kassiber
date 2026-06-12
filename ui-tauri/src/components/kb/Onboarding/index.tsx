import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { Wordmark } from "@/components/kb/Wordmark";
import { Button } from "@/components/ui/button";
import { dispatchDaemonAuthRequired, useDaemon } from "@/daemon/client";
import {
  activateImportProject,
  canImportProjects,
  canUseTouchIdPassphraseUnlock,
  clearImportProject,
  getTransport,
  selectImportProjectDirectory,
  storeTouchIdPassphrase,
  type DaemonEnvelope,
  type ImportProjectSelection,
} from "@/daemon/transport";
import { cn } from "@/lib/utils";
import { useUiStore, type DataMode, type Identity } from "@/store/ui";
import {
  clearSessionUnlockPassphrase,
  setSessionUnlockPassphrase,
} from "@/store/sessionLock";
import type { ProfilesSnapshot } from "@/mocks/profiles";

import {
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  DEFAULT_FORM,
  GAINS_ALGORITHM_DEFAULTS,
  electrumEndpointUrl,
  gainsAlgorithmsFor,
  parseTaxLongTermDays,
} from "./constants";
import {
  aiStepComplete,
  essentialsStepComplete,
  reviewStepComplete,
  securityStepComplete,
  syncStepComplete,
} from "./gates";
import { AiStep } from "./steps/AiStep";
import { EssentialsStep } from "./steps/EssentialsStep";
import { ReviewStep } from "./steps/ReviewStep";
import { SecurityStep } from "./steps/SecurityStep";
import { SyncStep } from "./steps/SyncStep";
import { ImportProjectPanel } from "./ImportProjectPanel";
import { StartChoicePanel } from "./StartChoicePanel";
import { OnboardingStepper } from "./stepper";
import type { BackendPreviewRow, OnboardingForm, OnboardingStep } from "./types";

interface OnboardingProps {
  className?: string;
  steps?: OnboardingStep[];
}

const DEFAULT_STEPS: OnboardingStep[] = [
  {
    component: EssentialsStep,
    label: "essentials",
    isComplete: essentialsStepComplete,
  },
  {
    component: SyncStep,
    label: "sync",
    isComplete: syncStepComplete,
  },
  {
    component: AiStep,
    label: "ai",
    isComplete: aiStepComplete,
  },
  {
    component: SecurityStep,
    label: "security",
    isComplete: securityStepComplete,
  },
  {
    component: ReviewStep,
    label: "review",
    isComplete: reviewStepComplete,
  },
];

/** Stable step ids mapped to their `steps.*` label key in the onboarding bundle. */
const DEFAULT_STEP_LABEL_KEYS: Record<
  string,
  "steps.essentials" | "steps.sync" | "steps.ai" | "steps.security" | "steps.review"
> = {
  essentials: "steps.essentials",
  sync: "steps.sync",
  ai: "steps.ai",
  security: "steps.security",
  review: "steps.review",
};

const SECURITY_STEP_INDEX = DEFAULT_STEPS.findIndex(
  (entry) => entry.component === SecurityStep,
);

const DEV_MOCK_IDENTITY: Identity = {
  name: "mock books",
  workspace: "My Books",
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
  const { t } = useTranslation("onboarding");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const setIdentity = useUiStore((state) => state.setIdentity);
  const setAppLockPolicy = useUiStore((state) => state.setAppLockPolicy);
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
  const backendPublicDefaultsQuery = useDaemon<{
    backends: Array<{
      name?: string;
      kind?: string;
      url?: string;
    }>;
  }>(
    "ui.backends.public_defaults",
    undefined,
    {
      enabled: flowMode === "setup" && !customSteps,
      refetchOnMount: "always",
    },
  );
  const backendPreviewRows: BackendPreviewRow[] =
    backendPublicDefaultsQuery.data?.data?.backends
      ?.map((backend) => ({
        name: backend.name?.trim() ?? "",
        kind: backend.kind?.trim() ?? "",
        url: backend.url?.trim() ?? "",
      }))
      .filter(
        (backend) =>
          backend.name.length > 0 &&
          backend.kind.length > 0 &&
          backend.url.length > 0,
      ) ?? [];

  const update = <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const clearDaemonQueryCache = () => {
    void queryClient.cancelQueries({ queryKey: ["daemon"] });
    queryClient.removeQueries({ queryKey: ["daemon"] });
  };

  const handleAuthRequired = (envelope: DaemonEnvelope) => {
    clearSessionUnlockPassphrase();
    clearDaemonQueryCache();
    dispatchDaemonAuthRequired(
      envelope,
      useUiStore.getState().daemonSession,
    );
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
          envelope.error?.message ?? t("shell.errorInitSqlcipher"),
        );
      }
      if (envelope.kind === "auth_required") {
        throw new Error(t("shell.passphraseRequired"));
      }
      // Best-effort: enroll the just-created passphrase for Touch ID unlock.
      // Non-fatal — the encrypted DB already exists, so a Keychain failure
      // shouldn't block onboarding; it can be retried later from Settings.
      if (form.enableTouchId && canUseTouchIdPassphraseUnlock()) {
        try {
          const status = await storeTouchIdPassphrase(form.databasePassphrase);
          if (status.configured) {
            setAppLockPolicy({ touchIdUnlock: true });
          }
        } catch {
          // Swallow: Touch ID stays optional and configurable post-setup.
        }
      }
    }

    const customBackendUrl =
      form.backendSetupMode === "custom" && form.backendKind === "electrum"
        ? electrumEndpointUrl({
            host: form.backendHost,
            port: form.backendPort,
            useSsl: form.backendUseSsl,
          })
        : form.backendUrl.trim();
    const backendProxy =
      form.backendSetupMode === "custom" &&
      form.backendUseProxy &&
      form.backendProxyHost.trim() &&
      form.backendProxyPort.trim()
        ? `${form.backendProxyHost.trim()}:${form.backendProxyPort.trim()}`
        : undefined;
    const onboarding = await getTransport("real").invoke({
      kind: "ui.onboarding.complete",
      args: {
        workspace_label: form.workspace.trim() || "My Books",
        profile_label: form.profile.trim() || "Private",
        tax_country: form.taxCountry,
        fiat_currency: form.fiatCurrency,
        tax_long_term_days: taxLongTermDays,
        gains_algorithm: gainsAlgorithm,
        ...(form.backendSetupMode === "custom"
          ? {
              backend: {
                name: form.backendName.trim() || "custom",
                kind: form.backendKind,
                url: customBackendUrl,
                chain:
                  form.backendKind === "liquid-esplora"
                    ? "liquid"
                    : "bitcoin",
                network:
                  form.backendKind === "liquid-esplora"
                    ? "liquidv1"
                    : "main",
                ...(form.backendKind === "electrum" &&
                form.backendUseSsl &&
                !form.backendTrustSsl &&
                form.backendCertificate.trim()
                  ? { certificate: form.backendCertificate.trim() }
                  : {}),
                ...(backendProxy ? { tor_proxy: backendProxy } : {}),
              },
            }
          : {}),
      },
    });
    if (onboarding.kind === "auth_required") {
      handleAuthRequired(onboarding);
      throw new Error(t("shell.passphraseRequired"));
    }
    if (onboarding.kind === "error" || onboarding.error) {
      throw new Error(onboarding.error?.message ?? t("shell.finishError"));
    }
    // Best-effort: persist the AI provider the assistant step configured.
    // Non-fatal — onboarding already succeeded, and the provider can be
    // added later from Settings -> Assistant; before this call the step's
    // collected config was silently dropped.
    if (form.aiSetupMode !== "disabled" && form.aiBaseUrl.trim()) {
      try {
        await getTransport("real").invoke({
          kind: "ai.providers.create",
          args: {
            name: form.aiProviderName.trim() || form.aiProviderKind,
            base_url: form.aiBaseUrl.trim(),
            kind: form.aiProviderKind,
            acknowledged: form.aiRemoteAcknowledged,
          },
        });
      } catch {
        // Swallow: assistant setup stays optional and repeatable post-setup.
      }
    }
    const identity: Identity = {
      name: form.profile.trim() || "Private",
      workspace: form.workspace.trim() || "My Books",
      // Legacy field. Today rp2 only ships `at` + `generic` country plugins,
      // so all non-AT picks collapse to "Generic". Prefer `taxCountry` for
      // new callers.
      country: form.taxCountry === "at" ? "AT" : "Generic",
      encrypted: form.databaseMode === "sqlcipher",
      profile: form.profile.trim() || "Private",
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
          ? customBackendUrl
          : undefined,
      backendTrustSsl:
        form.backendSetupMode === "custom" &&
        form.backendKind === "electrum" &&
        form.backendUseSsl
          ? form.backendTrustSsl
          : undefined,
      backendCertificate:
        form.backendSetupMode === "custom" &&
        form.backendKind === "electrum" &&
        form.backendUseSsl &&
        !form.backendTrustSsl &&
        form.backendCertificate.trim()
          ? form.backendCertificate.trim()
          : undefined,
      backendProxy:
        form.backendKind === "electrum" && backendProxy
          ? {
              host: form.backendProxyHost.trim(),
              port: form.backendProxyPort.trim(),
            }
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
          error instanceof Error ? error.message : t("shell.finishError"),
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

  // Express path: accept recommended defaults and jump straight to Security,
  // where the user only sets a passphrase. Encryption is never silently
  // skipped, and Back still reaches the Essentials step.
  const beginQuickStart = () => {
    setDataMode("real");
    setFinishError(null);
    setImportError(null);
    setForm(DEFAULT_FORM);
    setCurrentStep(SECURITY_STEP_INDEX);
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
        handleAuthRequired(envelope);
        throw new Error(t("shell.passphraseRequired"));
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? t("shell.errorLoadBooks"));
      }
      setImportSnapshot(
        envelope.data ?? { workspaces: [], activeProfileId: "" },
      );
    } catch (error) {
      const message =
        error instanceof Error ? error.message : t("shell.errorLoadBooks");
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
      if (selection.encrypted) {
        useUiStore.getState().bumpDaemonSession();
      }
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
        handleAuthRequired(envelope);
        throw new Error(t("shell.passphraseRequired"));
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? t("shell.errorOpenProject"));
      }
      await setSessionUnlockPassphrase(selection.encrypted ? passphrase : null);
      await refreshImportedProfiles();
    } catch (error) {
      const message =
        error instanceof Error ? error.message : t("shell.errorOpenProject");
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
      clearDaemonQueryCache();
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
          error instanceof Error ? error.message : t("shell.errorImportProject"),
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
        clearDaemonQueryCache();
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
            : t("shell.errorReturnDefaultRoot"),
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
          {import.meta.env.DEV && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={skipToMockPreview}
            >
              {t("shell.mockPreview")}
            </Button>
          )}
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
            onQuickStart={beginQuickStart}
          />
        ) : (
          <>
            <OnboardingStepper
              labels={activeSteps.map((entry) => {
                const labelKey = entry.label
                  ? DEFAULT_STEP_LABEL_KEYS[entry.label]
                  : undefined;
                return labelKey ? t(labelKey) : entry.label;
              })}
              current={currentStep}
              onJump={(index) => {
                setFinishError(null);
                setCurrentStep(index);
              }}
            />
            <step.component
              form={form}
              update={update}
              onSubmit={handleSubmit}
              canContinue={step.isComplete(form) && !submitting}
              submitting={submitting}
              currentStep={currentStep}
              totalSteps={activeSteps.length}
              backendPreviewRows={backendPreviewRows}
              goBack={handleGoBack}
              onJump={(index) => {
                setFinishError(null);
                setCurrentStep(index);
              }}
            />
          </>
        )}

        {(finishError || (!importSelection && importError)) && (
          <div className="max-w-2xl rounded-md border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {finishError ?? importError}
          </div>
        )}

        <div className="flex flex-wrap items-center justify-center gap-4 text-xs text-ink-3">
          <span>{t("shell.footer.privateKeys")}</span>
          <span>{t("shell.footer.stateLocation")}</span>
          <span>{t("shell.footer.noSaas")}</span>
        </div>
      </div>
    </section>
  );
};
