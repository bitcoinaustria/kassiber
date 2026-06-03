import { KeyRound, Loader2 } from "lucide-react";

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
  submitting = false,
}: StepComponentProps) => {
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
      title="Protect your books"
      eyebrow="Security"
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
            title="Encrypt these books"
            description="Recommended for real data. The local SQLite database opens only with your passphrase."
            onClick={() => update("databaseMode", "sqlcipher")}
          />
          <ChoiceCard
            active={form.databaseMode === "plaintext"}
            title="Leave it unencrypted"
            description="Useful for throwaway evaluation only. Anyone with disk access can read the database."
            tone="warning"
            onClick={() => update("databaseMode", "plaintext")}
          />
        </div>

        {encrypted ? (
          <div className="space-y-3">
            <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
              <div className="flex items-center gap-2 text-sm font-semibold text-ink">
                <KeyRound className="size-4" />
                Database passphrase
              </div>
              <TextField
                label="Passphrase"
                name="database-passphrase"
                type="password"
                autoComplete="new-password"
                autoFocus
                value={form.databasePassphrase}
                placeholder="At least 12 characters"
                onChange={(value) => update("databasePassphrase", value)}
              />
              <TextField
                label="Confirm passphrase"
                name="database-passphrase-confirm"
                type="password"
                autoComplete="new-password"
                value={form.databasePassphraseConfirm}
                placeholder="Repeat passphrase"
                hint={passphraseHint}
                onChange={(value) =>
                  update("databasePassphraseConfirm", value)
                }
              />
              <p className="m-0 text-xs leading-5 text-ink-2">
                Sent only to the local daemon to unlock the database — never
                stored in the UI.
              </p>
            </div>
            <CheckRow
              id="recovery-ack"
              checked={form.recoveryAcknowledged}
              onCheckedChange={(checked) =>
                update("recoveryAcknowledged", checked)
              }
              label="I understand there is no passphrase recovery path."
              description="If the passphrase is lost, the SQLCipher database cannot be opened."
            />
            {touchIdAvailable && (
              <CheckRow
                id="enable-touch-id"
                checked={form.enableTouchId}
                onCheckedChange={(checked) => update("enableTouchId", checked)}
                label="Unlock with Touch ID"
                description="Store the passphrase in the macOS Keychain so you can unlock with Touch ID instead of typing it. Remove it anytime in Settings."
              />
            )}
            <details className="rounded-lg border border-line bg-paper-2 p-3">
              <summary className="cursor-pointer text-sm font-medium text-ink marker:text-ink-3">
                Existing backend credentials
              </summary>
              <div className="pt-3">
                <CheckRow
                  id="migrate-credentials"
                  checked={form.migrateCredentials}
                  onCheckedChange={(checked) =>
                    update("migrateCredentials", checked)
                  }
                  label="Move credentials from backends.env into the encrypted DB."
                  description="Useful when upgrading an existing CLI setup. New books can leave this checked safely."
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
            label="I understand plaintext mode is not for real wallet data."
            description="Balances, addresses, tags, and backend metadata are readable by anything with disk access."
          />
        )}

        <OnboardingStepActions>
          <Button
            type="submit"
            className="w-full"
            disabled={!canCreateBooks || !canContinue}
          >
            {submitting ? (
              <>
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                Creating books…
              </>
            ) : (
              "Create books"
            )}
          </Button>
        </OnboardingStepActions>
      </form>
    </OnboardingSingleColumnFrame>
  );
};
