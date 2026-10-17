import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { Button } from "@/components/ui/button";

import {
  AI_PROVIDER_KIND_LABELS,
  BACKEND_KIND_LABELS,
  GAINS_ALGORITHM_DEFAULTS,
  electrumEndpointUrl,
} from "../constants";
import {
  OnboardingGroup,
  OnboardingRow,
  OnboardingStepActions,
  OnboardingStepPage,
} from "../frame";
import { NetworkStatus } from "../network";
import { endpointReach, type NetworkSummary } from "../networkReach";
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
  | "secrets"
  | "others";

/** Where a choice takes effect: kept on disk, or reaching the network. */
type ReviewGroup = "local" | "network";

// What reaches the network comes first: it is what a privacy-minded user
// most needs to confirm before anything is created.
const REVIEW_GROUPS: ReviewGroup[] = ["network", "local"];

const AREA_GROUP: Record<ReviewArea, ReviewGroup> = {
  books: "local",
  tax: "local",
  storage: "local",
  secrets: "local",
  sync: "network",
  ai: "network",
  updates: "network",
  others: "network",
};

interface ReviewRow {
  areaKey: ReviewArea;
  value: string;
  /** Only what the value doesn't already say. */
  note?: string;
  /** The step that changes it; absent for what setup doesn't configure. */
  step?: keyof typeof STEP_INDEX;
  details?: ReviewDetail[];
  /** What this lets Kassiber contact, for the network group. */
  network?: NetworkSummary;
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
  backendPreviewRows = [],
  canContinue = true,
  submitting = false,
}: StepComponentProps) => {
  const { t } = useTranslation("onboarding");
  const rows = reviewRows(t, form, backendPreviewRows);

  return (
    <OnboardingStepPage
      title={t("review.title")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
      >
        {REVIEW_GROUPS.map((group) => (
          <section
            key={group}
            aria-labelledby={`review-${group}`}
            className="space-y-2.5"
          >
            <h2
              id={`review-${group}`}
              className="px-1 text-sm font-medium text-ink"
            >
              {t(`review.group.${group}.title`)}
            </h2>
            <OnboardingGroup>
              {rows
                .filter((row) => AREA_GROUP[row.areaKey] === group)
                .map((row) => (
                  <OnboardingRow
                    key={row.areaKey}
                    title={t(`review.area.${row.areaKey}`)}
                    description={
                      <div className="space-y-1">
                        {row.network && (
                          <NetworkStatus
                            reach={row.network.reach}
                            label={row.network.label}
                          />
                        )}
                        <p className="text-sm text-ink-2">{row.value}</p>
                        {row.details?.length ? (
                          <ul className="list-none space-y-0.5 p-0">
                            {row.details.map((detail) => (
                              <li
                                key={`${detail.label}-${detail.value}`}
                                className="grid gap-x-3 sm:grid-cols-[10rem_minmax(0,1fr)]"
                              >
                                <span className="text-ink-2">
                                  {detail.label}
                                </span>
                                <span className="font-mono text-2xs leading-5 break-all text-ink-2">
                                  {detail.value}
                                </span>
                              </li>
                            ))}
                          </ul>
                        ) : null}
                        {row.note && <p>{row.note}</p>}
                      </div>
                    }
                    control={
                      row.step ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="text-ink-3 hover:text-ink"
                          aria-label={t("review.changeArea", {
                            area: t(`review.area.${row.areaKey}`),
                          })}
                          onClick={() =>
                            row.step && onJump?.(STEP_INDEX[row.step])
                          }
                        >
                          {t("review.change")}
                        </Button>
                      ) : undefined
                    }
                  />
                ))}
            </OnboardingGroup>
          </section>
        ))}

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
            ? undefined
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
      network: syncNetwork(t, form),
    },
    {
      areaKey: "ai",
      value: aiValue(t, form),
      note: aiNote(t, form),
      step: "ai",
      details: aiDetails(t, form),
      network: aiNetwork(t, form),
    },
    {
      areaKey: "updates",
      value: updateChecks
        ? t("review.updates.valueEnabled")
        : t("review.updates.valueDisabled"),
      note: updateChecks ? t("review.updates.noteEnabled") : undefined,
      step: "sync",
      details: updateChecks
        ? [{ label: "GitHub", value: "api.github.com" }]
        : undefined,
      network: updateChecks
        ? {
            reach: "internet",
            label: t("network.label", {
              reach: t("network.reach.internet"),
              when: t("network.when.updates"),
            }),
          }
        : NO_CONNECTIONS(t),
    },
    {
      areaKey: "others",
      value: t("review.others.value"),
      network: { reach: "none", label: t("network.offUntilSetUp") },
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
          : undefined,
      step: "security",
    },
  ];
}

const NO_CONNECTIONS = (t: TFunction<"onboarding">): NetworkSummary => ({
  reach: "none",
  label: t("network.none"),
});

function syncNetwork(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
): NetworkSummary {
  if (form.backendSetupMode === "skip") return NO_CONNECTIONS(t);
  const reach =
    form.backendSetupMode === "custom"
      ? endpointReach(customEndpoint(form))
      : "internet";
  return {
    reach,
    label: t("network.label", {
      reach: t(`network.reach.${reach}`),
      when: t("network.when.sync"),
    }),
  };
}

function aiNetwork(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
): NetworkSummary {
  if (form.backendSetupMode === "skip" || form.aiSetupMode === "disabled")
    return NO_CONNECTIONS(t);
  const url = form.aiBaseUrl.trim();
  const reach = url ? endpointReach(url) : "internet";
  return {
    reach,
    label: t("network.label", {
      reach: t(`network.reach.${reach}`),
      when: t("network.when.assistant"),
    }),
  };
}

function aiDetails(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
): ReviewDetail[] | undefined {
  const url = form.aiBaseUrl.trim();
  if (form.backendSetupMode === "skip" || form.aiSetupMode === "disabled")
    return undefined;
  return url ? [{ label: t("ai.baseUrl"), value: url }] : undefined;
}

function customEndpoint(form: OnboardingForm): string {
  return form.backendKind === "electrum"
    ? electrumEndpointUrl({
        host: form.backendHost,
        port: form.backendPort,
        useSsl: form.backendUseSsl,
      })
    : form.backendUrl.trim();
}

function syncValue(t: TFunction<"onboarding">, form: OnboardingForm): string {
  if (form.backendSetupMode === "skip") return t("review.sync.valueManual");
  if (form.backendSetupMode === "default")
    return t("review.sync.valueDefault");
  const kind = BACKEND_KIND_LABELS[form.backendKind] ?? form.backendKind;
  const endpoint = customEndpoint(form);
  return t("review.sync.valueCustom", {
    name: form.backendName.trim() || t("review.sync.customFallbackName"),
    detail: `${kind}${endpoint ? `, ${endpoint}` : ""}`,
  });
}

interface ReviewDetailSource {
  name: string;
  kind: string;
  url: string;
  chain: string;
}

function syncNote(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): string | undefined {
  if (form.backendSetupMode === "default") {
    return backendPreviewRows.length === 0
      ? t("review.sync.noteDefaultLoading")
      : t("connections.default.tradeoff");
  }
  if (form.backendSetupMode === "custom" && form.backendUseProxy) {
    return t("review.sync.noteCustomProxy");
  }
  return undefined;
}

function syncDetails(
  form: OnboardingForm,
  backendPreviewRows: ReviewDetailSource[],
): ReviewDetail[] | undefined {
  if (form.backendSetupMode !== "default") return undefined;
  const rows = form.includeLiquidBackends
    ? backendPreviewRows
    : backendPreviewRows.filter((backend) => backend.chain !== "liquid");
  if (rows.length === 0) return undefined;
  return rows.map((backend) => ({
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

function aiNote(
  t: TFunction<"onboarding">,
  form: OnboardingForm,
): string | undefined {
  if (form.backendSetupMode === "skip") return undefined;
  return form.aiSetupMode === "remote" ? t("review.ai.noteRemote") : undefined;
}
