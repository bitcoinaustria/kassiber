import { ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConnectionsFields } from "../ConnectionsFields";
import { Switch } from "@/components/ui/switch";

import {
  OnboardingNote,
  OnboardingRow,
  OnboardingSection,
  OnboardingStepActions,
  OnboardingStepPage,
} from "../frame";
import { NetworkStatus } from "../network";
import type { StepComponentProps } from "../types";

export const SyncStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
  backendPreviewRows,
}: StepComponentProps) => {
  const { t } = useTranslation(["onboarding", "common"]);
  const offline = form.backendSetupMode === "skip";
  const updateChecks = form.updateChecksEnabled && !offline;
  return (
    <OnboardingStepPage
      title={t("sync.title")}
      lead={t("sync.lead")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
      >
        <div className="space-y-4">
          <ConnectionsFields
            form={form}
            update={update}
            defaultBackends={backendPreviewRows}
          />
          <OnboardingNote icon={ShieldCheck}>
            {t("sync.footnote")}
          </OnboardingNote>
        </div>

        <OnboardingSection title={t("sync.updateChecksHeading")}>
          {/* Offline setup blocks update checks without forgetting the
              user's answer, so switching back online restores it. */}
          <OnboardingRow
            disabled={offline}
            title={
              <label htmlFor="allow-update-checks">
                {t("sync.updateChecks")}
              </label>
            }
            description={t("sync.updateChecksDescription")}
            status={
              updateChecks ? (
                <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <NetworkStatus
                    reach="internet"
                    label={t("network.label", {
                      reach: t("network.reach.internet"),
                      when: t("network.when.updates"),
                    })}
                  />
                  <span className="font-mono text-2xs text-ink-2">
                    api.github.com
                  </span>
                </span>
              ) : (
                <NetworkStatus
                  reach="none"
                  label={offline ? t("network.offOffline") : t("network.none")}
                />
              )
            }
            control={
              <Switch
                id="allow-update-checks"
                checked={updateChecks}
                disabled={offline}
                onCheckedChange={(checked) =>
                  update("updateChecksEnabled", checked)
                }
              />
            }
          />
        </OnboardingSection>

        <OnboardingStepActions goBack={goBack} disabled={!canContinue} />
      </form>
    </OnboardingStepPage>
  );
};
