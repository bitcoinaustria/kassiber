import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

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

type ReviewArea =
  | "books"
  | "tax"
  | "sync"
  | "ai"
  | "updates"
  | "storage"
  | "secrets";

interface ReviewRow {
  areaKey: ReviewArea;
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
  const { t } = useTranslation("onboarding");
  const rows = reviewRows(t, form, backendPreviewRows);

  return (
    <OnboardingSingleColumnFrame
      title={t("review.title")}
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
        <p className="m-0 text-sm leading-6 text-ink-2">{t("review.intro")}</p>

        <div className="overflow-hidden rounded-lg border border-line">
          <table className="w-full border-collapse text-left text-sm">
            <tbody>
              {rows.map((row) => (
                <tr
                  key={row.areaKey}
                  className="border-b border-line last:border-b-0"
                >
                  <th className="w-32 bg-paper-2 px-4 py-3 align-top font-medium text-ink">
                    {t(`review.area.${row.areaKey}`)}
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
                        aria-label={t("review.changeArea", {
                          area: t(`review.area.${row.areaKey}`),
                        })}
                        onClick={() => onJump?.(STEP_INDEX[row.step])}
                      >
                        {t("review.change")}
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
                {t("review.creatingBooks")}
              </>
            ) : (
              t("review.createLocalBooks")
            )}
          </Button>
        </OnboardingStepActions>
      </form>
    </OnboardingSingleColumnFrame>
  );
};

function reviewRows(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): ReviewRow[] {
  return [
    {
      areaKey: "books",
      value: t("review.books.value", {
        workspace: form.workspace.trim() || "My Books",
        profile: form.profile.trim() || "Private",
      }),
      note: t("review.books.note"),
      step: "books",
    },
    {
      areaKey: "tax",
      value:
        form.taxCountry === "at"
          ? t("review.tax.valueAt")
          : t("review.tax.valueGeneric", {
              currency: form.fiatCurrency,
              algorithm: form.gainsAlgorithm,
            }),
      note:
        form.taxCountry === "at"
          ? t("review.tax.noteAt")
          : t("review.tax.noteGeneric", {
              days: form.taxLongTermDays || "365",
            }),
      step: "books",
    },
    {
      areaKey: "sync",
      value: syncValue(t, form),
      note: syncNote(t, form, backendPreviewRows),
      step: "sync",
      details: syncDetails(form, backendPreviewRows),
    },
    {
      areaKey: "ai",
      value: aiValue(t, form),
      note: aiNote(t, form),
      step: "ai",
    },
    {
      areaKey: "updates",
      value: form.updateChecksEnabled
        ? t("review.updates.valueEnabled")
        : t("review.updates.valueDisabled"),
      note: form.updateChecksEnabled
        ? t("review.updates.noteEnabled")
        : t("review.updates.noteDisabled"),
      step: "sync",
    },
    {
      areaKey: "storage",
      value:
        form.databaseMode === "sqlcipher"
          ? t("review.storage.valueEncrypted")
          : t("review.storage.valuePlaintext"),
      note:
        form.databaseMode === "sqlcipher"
          ? t("review.storage.noteEncrypted", {
              touchId: form.enableTouchId
                ? t("review.storage.touchIdEnabled")
                : t("review.storage.touchIdNotEnabled"),
            })
          : t("review.storage.notePlaintext"),
      step: "security",
    },
    {
      areaKey: "secrets",
      value:
        form.databaseMode === "sqlcipher" && form.migrateCredentials
          ? t("review.secrets.valueEncrypted")
          : t("review.secrets.valueNone"),
      note:
        form.databaseMode === "sqlcipher" && form.migrateCredentials
          ? t("review.secrets.noteEncrypted")
          : t("review.secrets.noteNone"),
      step: "security",
    },
  ];
}

function syncValue(t: TFunction<"onboarding">, form: OnboardingForm): string {
  if (form.backendSetupMode === "skip") return t("review.sync.valueManual");
  if (form.backendSetupMode === "default")
    return t("review.sync.valueDefault");
  const kind = BACKEND_KIND_LABELS[form.backendKind] ?? form.backendKind;
  const endpoint =
    form.backendKind === "electrum"
      ? electrumEndpointUrl({
          host: form.backendHost,
          port: form.backendPort,
          useSsl: form.backendUseSsl,
        })
      : form.backendUrl.trim();
  return t("review.sync.valueCustom", {
    name: form.backendName.trim() || t("review.sync.customFallbackName"),
    detail: `${kind}${endpoint ? `, ${endpoint}` : ""}`,
  });
}

interface ReviewDetailSource {
  name: string;
  kind: string;
  url: string;
}

function syncNote(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): string {
  if (form.backendSetupMode === "skip") {
    return t("review.sync.noteSkip");
  }
  if (form.backendSetupMode === "default") {
    if (backendPreviewRows.length === 0) {
      return t("review.sync.noteDefaultLoading");
    }
    return t("review.sync.noteDefault");
  }
  if (form.backendUseProxy) {
    return t("review.sync.noteCustomProxy");
  }
  return t("review.sync.noteCustom");
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

function aiValue(t: TFunction<"onboarding">, form: OnboardingForm): string {
  if (form.aiSetupMode === "disabled") return t("review.ai.valueDisabled");
  const kind =
    AI_PROVIDER_KIND_LABELS[form.aiProviderKind] ?? form.aiProviderKind;
  return t("review.ai.value", {
    kind,
    name: form.aiProviderName.trim() || t("review.ai.fallbackName"),
  });
}

function aiNote(t: TFunction<"onboarding">, form: OnboardingForm): string {
  if (form.aiSetupMode === "disabled") {
    return t("review.ai.noteDisabled");
  }
  if (form.aiSetupMode === "remote") {
    return t("review.ai.noteRemote");
  }
  return t("review.ai.noteLocal");
}
