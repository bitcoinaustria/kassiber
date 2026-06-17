import { KeyRound } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { canUseTouchIdPassphraseUnlock } from "@/daemon/transport";

import { databasePassphraseHint } from "../constants";
import { CheckRow, ChoiceCard, TextField } from "../fields";
import {
  OnboardingSingleColumnFrame,
  OnboardingStepActions,
} from "../frame";
import type { StepComponentProps } from "../types";

export const SecurityStep = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const { t } = useTranslation("onboarding");
  const encrypted = form.databaseMode === "sqlcipher";
  const touchIdAvailable = encrypted && canUseTouchIdPassphraseUnlock();
  const passphraseHint = encrypted
    ? databasePassphraseHint(
        form.databasePassphrase,
        form.databasePassphraseConfirm,
      )
    : null;
  const canCreateBooks = encrypted
    ? passphraseHint === null && form.recoveryAcknowledged
    : form.plaintextAcknowledged;
  return (
    <OnboardingSingleColumnFrame
      title={t("security.title")}
      currentStep={currentStep}
      totalSteps={totalSteps}
      goBack={goBack}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-5"
      >
        <div className="space-y-3">
          <ChoiceCard
            active={encrypted}
            title={t("security.encrypt.title")}
            description={t("security.encrypt.description")}
            onClick={() => update("databaseMode", "sqlcipher")}
          />
          <ChoiceCard
            active={form.databaseMode === "plaintext"}
            title={t("security.plaintext.title")}
            description={t("security.plaintext.description")}
            tone="warning"
            onClick={() => update("databaseMode", "plaintext")}
          />
        </div>

        {encrypted ? (
          <div className="space-y-3">
            <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
              <div className="flex items-center gap-2 text-sm font-semibold text-ink">
                <KeyRound className="size-4" />
                {t("security.databasePassphrase")}
              </div>
              <TextField
                label={t("security.passphrase")}
                name="database-passphrase"
                type="password"
                autoComplete="new-password"
                autoFocus
                value={form.databasePassphrase}
                placeholder={t("security.passphrasePlaceholder")}
                onChange={(value) => update("databasePassphrase", value)}
              />
              <TextField
                label={t("security.confirmPassphrase")}
                name="database-passphrase-confirm"
                type="password"
                autoComplete="new-password"
                value={form.databasePassphraseConfirm}
                placeholder={t("security.confirmPassphrasePlaceholder")}
                hint={passphraseHint}
                onChange={(value) =>
                  update("databasePassphraseConfirm", value)
                }
              />
              <p className="m-0 text-xs leading-5 text-ink-2">
                {t("security.passphraseNote")}
              </p>
            </div>
            <CheckRow
              id="recovery-ack"
              checked={form.recoveryAcknowledged}
              onCheckedChange={(checked) =>
                update("recoveryAcknowledged", checked)
              }
              label={t("security.recoveryAck")}
              description={t("security.recoveryAckDescription")}
            />
            {touchIdAvailable && (
              <CheckRow
                id="enable-touch-id"
                checked={form.enableTouchId}
                onCheckedChange={(checked) => update("enableTouchId", checked)}
                label={t("security.touchId")}
                description={t("security.touchIdDescription")}
              />
            )}
            <details className="rounded-lg border border-line bg-paper-2 p-3">
              <summary className="cursor-pointer text-sm font-medium text-ink marker:text-ink-3">
                {t("security.existingCredentials")}
              </summary>
              <div className="pt-3">
                <CheckRow
                  id="migrate-credentials"
                  checked={form.migrateCredentials}
                  onCheckedChange={(checked) =>
                    update("migrateCredentials", checked)
                  }
                  label={t("security.migrateCredentials")}
                  description={t("security.migrateCredentialsDescription")}
                />
              </div>
            </details>
          </div>
        ) : (
          <CheckRow
            id="plaintext-ack"
            checked={form.plaintextAcknowledged}
            onCheckedChange={(checked) =>
              update("plaintextAcknowledged", checked)
            }
            label={t("security.plaintextAck")}
            description={t("security.plaintextAckDescription")}
          />
        )}

        <OnboardingStepActions>
          <Button
            type="submit"
            className="w-full"
            disabled={!canCreateBooks || !canContinue}
          >
            {t("common:actions.continue")}
          </Button>
        </OnboardingStepActions>
      </form>
    </OnboardingSingleColumnFrame>
  );
};
