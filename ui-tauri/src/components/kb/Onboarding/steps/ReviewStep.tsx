import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { Button } from "@/components/ui/button";

import {
  AI_PROVIDER_KIND_LABELS,
  BACKEND_KIND_LABELS,
  GAINS_ALGORITHM_DEFAULTS,
  electrumEndpointUrl,
} from "../constants";
import { OnboardingStepActions, OnboardingStepPage } from "../frame";
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
    <OnboardingStepPage
      eyebrow={t("frame.step", { current: currentStep + 1, total: totalSteps })}
      title={t("review.title")}
      lead={t("review.intro")}
      className="max-w-2xl"
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
      >
        <dl className="kb-surface divide-y divide-border overflow-hidden">
          {rows.map((row) => (
            <div
              key={row.areaKey}
              className="grid gap-x-4 gap-y-1 px-(--kb-card-padding) py-4 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto]"
            >
              <dt className="pt-0.5 font-mono text-2xs font-medium uppercase tracking-[0.14em] text-ink-3">
                {t(`review.area.${row.areaKey}`)}
              </dt>
              <dd className="min-w-0 space-y-1">
                <p className="text-sm font-medium text-ink">{row.value}</p>
                {row.details?.length ? (
                  <ul className="list-none space-y-1 p-0 text-xs leading-5 text-ink-2">
                    {row.details.map((detail) => (
                      <li
                        key={`${detail.label}-${detail.value}`}
                        className="grid gap-x-3 gap-y-0.5 sm:grid-cols-[8rem_minmax(0,1fr)]"
                      >
                        <span className="font-medium text-ink">
                          {detail.label}
                        </span>
                        <span className="break-all font-mono text-xs leading-5 text-ink-2">
                          {detail.value}
                        </span>
                      </li>
                    ))}
                  </ul>
                ) : null}
                <p className="text-xs leading-5 text-ink-2">{row.note}</p>
              </dd>
              <div className="sm:-my-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-label={t("review.changeArea", {
                    area: t(`review.area.${row.areaKey}`),
                  })}
                  onClick={() => onJump?.(STEP_INDEX[row.step])}
                >
                  {t("review.change")}
                </Button>
              </div>
            </div>
          ))}
        </dl>

        <OnboardingStepActions
          goBack={goBack}
          label={t("review.createLocalBooks")}
          busyLabel={t("review.creatingBooks")}
          busy={submitting}
          disabled={!canContinue}
        />
      </form>
    </OnboardingStepPage>
  );
};

function reviewRows(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): ReviewRow[] {
  // Offline setup blocks update checks and AI at finish; show what will apply.
  const updateChecks =
    form.backendSetupMode !== "skip" && form.updateChecksEnabled;
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
      // The values actually chosen, not the jurisdiction's defaults: an
      // Austrian book can still be set to another currency or method.
      value: t("review.tax.value", {
        region:
          form.taxCountry === "at"
            ? t("essentials.jurisdictionAustria")
            : t("essentials.jurisdictionGeneric"),
        currency: form.fiatCurrency,
        method: t(`books.method.${form.gainsAlgorithm}`),
      }),
      note:
        form.taxCountry === "at"
          ? form.fiatCurrency === "EUR" &&
            form.gainsAlgorithm === GAINS_ALGORITHM_DEFAULTS.at
            ? t("review.tax.noteAt")
            : t("review.tax.noteAtCustom")
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
      value: updateChecks
        ? t("review.updates.valueEnabled")
        : t("review.updates.valueDisabled"),
      note: updateChecks
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
  if (form.backendSetupMode === "skip" || form.aiSetupMode === "disabled")
    return t("review.ai.valueDisabled");
  const kind =
    AI_PROVIDER_KIND_LABELS[form.aiProviderKind] ?? form.aiProviderKind;
  return t("review.ai.value", {
    kind,
    name: form.aiProviderName.trim() || t("review.ai.fallbackName"),
  });
}

function aiNote(t: TFunction<"onboarding">, form: OnboardingForm): string {
  if (form.backendSetupMode === "skip" || form.aiSetupMode === "disabled") {
    return t("review.ai.noteDisabled");
  }
  if (form.aiSetupMode === "remote") {
    return t("review.ai.noteRemote");
  }
  return t("review.ai.noteLocal");
}
