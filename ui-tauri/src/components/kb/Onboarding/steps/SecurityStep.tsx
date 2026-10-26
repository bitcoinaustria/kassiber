import { KeyRound, LockKeyhole, ShieldOff } from "lucide-react";
import { useTranslation } from "react-i18next";

import { canUseTouchIdPassphraseUnlock } from "@/daemon/transport";

import { databasePassphraseHint } from "../constants";
import { CheckRow, ChoiceCard, TextField } from "../fields";
import { OnboardingStepActions, OnboardingStepPage } from "../frame";
import type { StepComponentProps } from "../types";

export const SecurityStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const { t } = useTranslation(["onboarding", "common"]);
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
    <OnboardingStepPage
      title={t("security.title")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
      >
        <div className="space-y-3">
          <ChoiceCard
            active={encrypted}
            icon={LockKeyhole}
            title={t("security.encrypt.title")}
            description={t("security.encrypt.description")}
            onClick={() => update("databaseMode", "sqlcipher")}
          >
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
              onChange={(value) => update("databasePassphraseConfirm", value)}
            />
            <p className="flex items-start gap-2 text-xs leading-5 text-ink-3">
              <KeyRound className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
              {t("security.passphraseNote")}
            </p>
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
          </ChoiceCard>
          <ChoiceCard
            active={form.databaseMode === "plaintext"}
            icon={ShieldOff}
            title={t("security.plaintext.title")}
            description={t("security.plaintext.description")}
            tone="warning"
            onClick={() => update("databaseMode", "plaintext")}
          >
            <CheckRow
              id="plaintext-ack"
              checked={form.plaintextAcknowledged}
              onCheckedChange={(checked) =>
                update("plaintextAcknowledged", checked)
              }
              label={t("security.plaintextAck")}
              description={t("security.plaintextAckDescription")}
            />
          </ChoiceCard>
        </div>

        {encrypted && (
          <details className="group border-t border-border pt-5">
            <summary className="cursor-pointer text-sm font-medium text-ink-2 marker:text-ink-3 hover:text-ink">
              {t("security.existingCredentials")}
            </summary>
            <div className="pt-4">
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
        )}

        <OnboardingStepActions
          goBack={goBack}
          disabled={!canCreateBooks || !canContinue}
        />
      </form>
    </OnboardingStepPage>
  );
};
