import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";

import {
  AI_PROVIDER_KIND_LABELS,
  BACKEND_KIND_LABELS,
  electrumEndpointUrl,
} from "../constants";
import {
  OnboardingSingleColumnFrame,
  OnboardingStepActions,
} from "../frame";
import type { OnboardingForm, StepComponentProps } from "../types";

const STEP_INDEX = {
  books: 0,
  sync: 1,
  ai: 2,
  security: 3,
} as const;

interface ReviewRow {
  area: string;
  value: string;
  note: string;
  step: keyof typeof STEP_INDEX;
  details?: ReviewDetail[];
}

interface ReviewDetail {
  label: string;
  value: string;
}

export const ReviewStep = ({
  form,
  onSubmit,
  onJump,
  goBack,
  currentStep,
  totalSteps,
  backendPreviewRows = [],
  canContinue = true,
  submitting = false,
}: StepComponentProps) => {
  const rows = reviewRows(form, backendPreviewRows);

  return (
    <OnboardingSingleColumnFrame
      title="Review security setup"
      currentStep={currentStep}
      totalSteps={totalSteps}
      goBack={goBack}
      className="max-w-3xl"
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-5"
      >
        <p className="m-0 text-sm leading-6 text-ink-2">
          Confirm the choices that affect local storage, network visibility,
          and whether data can leave this machine.
        </p>

        <div className="overflow-hidden rounded-lg border border-line">
          <table className="w-full border-collapse text-left text-sm">
            <tbody>
              {rows.map((row) => (
                <tr
                  key={row.area}
                  className="border-b border-line last:border-b-0"
                >
                  <th className="w-32 bg-paper-2 px-4 py-3 align-top font-medium text-ink">
                    {row.area}
                  </th>
                  <td className="px-4 py-3 align-top">
                    <div className="flex items-start justify-between gap-4">
                      <div className="min-w-0 space-y-1">
                        <p className="m-0 font-medium text-ink">{row.value}</p>
                        {row.details?.length ? (
                          <ul className="m-0 space-y-1 p-0 text-xs leading-5 text-ink-2">
                            {row.details.map((detail) => (
                              <li
                                key={`${detail.label}-${detail.value}`}
                                className="grid gap-x-3 gap-y-0.5 sm:grid-cols-[8rem_minmax(0,1fr)]"
                              >
                                <span className="font-medium text-ink">
                                  {detail.label}
                                </span>
                                <span className="break-all font-mono text-[11px] leading-5 text-ink-2">
                                  {detail.value}
                                </span>
                              </li>
                            ))}
                          </ul>
                        ) : null}
                        <p className="m-0 text-xs leading-5 text-ink-2">
                          {row.note}
                        </p>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="shrink-0"
                        aria-label={`Change ${row.area}`}
                        onClick={() => onJump?.(STEP_INDEX[row.step])}
                      >
                        Change
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <OnboardingStepActions>
          <Button
            type="submit"
            className="w-full"
            disabled={!canContinue || submitting}
          >
            {submitting ? (
              <>
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                Creating books...
              </>
            ) : (
              "Create local books"
            )}
          </Button>
        </OnboardingStepActions>
      </form>
    </OnboardingSingleColumnFrame>
  );
};

function reviewRows(
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): ReviewRow[] {
  return [
    {
      area: "Books",
      value: `${form.workspace.trim() || "My Books"} / ${
        form.profile.trim() || "Private"
      }`,
      note: "Creates one local books set with one active book.",
      step: "books",
    },
    {
      area: "Tax",
      value:
        form.taxCountry === "at"
          ? "Austria, EUR, moving average"
          : `Generic, ${form.fiatCurrency}, ${form.gainsAlgorithm}`,
      note:
        form.taxCountry === "at"
          ? "Austrian reporting currency and method are jurisdiction defaults."
          : `${form.taxLongTermDays || "365"} long-term holding days.`,
      step: "books",
    },
    {
      area: "Sync",
      value: syncValue(form),
      note: syncNote(form, backendPreviewRows),
      step: "sync",
      details: syncDetails(form, backendPreviewRows),
    },
    {
      area: "AI",
      value: aiValue(form),
      note: aiNote(form),
      step: "ai",
    },
    {
      area: "Storage",
      value:
        form.databaseMode === "sqlcipher"
          ? "SQLCipher encrypted database"
          : "Plaintext database",
      note:
        form.databaseMode === "sqlcipher"
          ? `Passphrase required. Touch ID ${
              form.enableTouchId ? "enabled" : "not enabled"
            }. No recovery path.`
          : "Evaluation only. Anyone with disk access can read wallet data.",
      step: "security",
    },
    {
      area: "Secrets",
      value:
        form.databaseMode === "sqlcipher" && form.migrateCredentials
          ? "Encrypted credential storage"
          : "No credentials collected during setup",
      note:
        form.databaseMode === "sqlcipher" && form.migrateCredentials
          ? "Existing backends.env credentials migrate if present; new secrets are added later."
          : "Backend API keys, cookies, and passwords are added later.",
      step: "security",
    },
  ];
}

function syncValue(form: OnboardingForm): string {
  if (form.backendSetupMode === "skip") return "Manual imports only";
  if (form.backendSetupMode === "default") return "Built-in public backends";
  const kind = BACKEND_KIND_LABELS[form.backendKind] ?? form.backendKind;
  const endpoint =
    form.backendKind === "electrum"
      ? electrumEndpointUrl({
          host: form.backendHost,
          port: form.backendPort,
          useSsl: form.backendUseSsl,
        })
      : form.backendUrl.trim();
  return `${form.backendName.trim() || "Custom backend"} (${kind}${
    endpoint ? `, ${endpoint}` : ""
  })`;
}

interface ReviewDetailSource {
  name: string;
  kind: string;
  url: string;
}

function syncNote(
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): string {
  if (form.backendSetupMode === "skip") {
    return "No address discovery or watch-only refresh until Settings adds a backend.";
  }
  if (form.backendSetupMode === "default") {
    if (backendPreviewRows.length === 0) {
      return "Backend addresses are loading from Kassiber.";
    }
    return "Public operators can see address queries; no no-log promise.";
  }
  if (form.backendKind === "electrum" && form.backendUseProxy) {
    return "Custom Electrum backend routed through the configured proxy.";
  }
  return "Custom backend. Use your own infrastructure or a trusted operator.";
}

function syncDetails(
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): ReviewDetail[] | undefined {
  if (form.backendSetupMode !== "default") return undefined;
  if (backendPreviewRows.length === 0) return undefined;
  return backendPreviewRows.map((backend) => ({
    label: `${backend.name} (${backendKindLabel(backend.kind)})`,
    value: backend.url,
  }));
}

function backendKindLabel(kind: string): string {
  return BACKEND_KIND_LABELS[kind as keyof typeof BACKEND_KIND_LABELS] ?? kind;
}

function aiValue(form: OnboardingForm): string {
  if (form.aiSetupMode === "disabled") return "Disabled";
  const kind =
    AI_PROVIDER_KIND_LABELS[form.aiProviderKind] ?? form.aiProviderKind;
  return `${kind}: ${form.aiProviderName.trim() || "assistant"}`;
}

function aiNote(form: OnboardingForm): string {
  if (form.aiSetupMode === "disabled") {
    return "No assistant calls during normal app use.";
  }
  if (form.aiSetupMode === "remote") {
    return "Remote provider can receive selected book context after consent.";
  }
  return "Local provider endpoint; book context stays on your machine unless configured otherwise.";
}
