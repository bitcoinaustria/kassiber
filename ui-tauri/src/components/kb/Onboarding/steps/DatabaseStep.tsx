import { Database, KeyRound, LockKeyhole } from "lucide-react";

import { Button } from "@/components/ui/button";

import { CheckRow, ChoiceCard, TextField } from "../fields";
import { databasePassphraseHint } from "../constants";
import {
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type { OnboardingForm, StepComponentProps } from "../types";

const DatabasePanel = ({ form }: { form: OnboardingForm }) => {
  const rows: ReadonlyArray<readonly [string, string]> = [
    ["State root", "~/.kassiber/{data,config,exports,attachments}"],
    [
      "Database",
      form.databaseMode === "sqlcipher" ? "SQLCipher 4" : "Plain SQLite",
    ],
    ["KDF", "SQLCipher stock kdf_iter 256000"],
    ["Backup", "tar | age bundle"],
    ["Outside perimeter", "attachments, exports, dotenv addressing rows"],
  ];
  return (
    <div className="flex h-full items-center">
      <div className="w-full max-w-lg rounded-lg border border-line bg-paper p-5 shadow-sm">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-md bg-ink text-paper">
            {form.databaseMode === "sqlcipher" ? (
              <LockKeyhole className="size-5" />
            ) : (
              <Database className="size-5" />
            )}
          </div>
          <div>
            <p className="font-semibold text-ink">
              {form.databaseMode === "sqlcipher"
                ? "Encrypted at rest"
                : "Plaintext preview"}
            </p>
            <p className="text-xs text-ink-2">
              Watch-only wallet data stays local.
            </p>
          </div>
        </div>
        <dl className="mt-5 space-y-3">
          {rows.map(([label, value]) => (
            <div
              key={label}
              className="flex items-start justify-between gap-4 border-b border-line pb-2 last:border-b-0"
            >
              <dt className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                {label}
              </dt>
              <dd className="m-0 max-w-[280px] text-right text-xs font-medium leading-5 text-ink">
                {value}
              </dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
};

export const DatabaseStep = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
}: StepComponentProps) => {
  const encrypted = form.databaseMode === "sqlcipher";
  const passphraseHint = encrypted
    ? databasePassphraseHint(
        form.databasePassphrase,
        form.databasePassphraseConfirm,
      )
    : null;
  const canOpenLedger = encrypted
    ? passphraseHint === null && form.recoveryAcknowledged
    : form.plaintextAcknowledged;
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Protect the database"
        eyebrow="Database"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-5">
            <div className="space-y-3">
              <ChoiceCard
                active={encrypted}
                title="SQLCipher database"
                description="Recommended for real books. The local SQLite file is encrypted at rest with a passphrase."
                onClick={() => update("databaseMode", "sqlcipher")}
              />
              <ChoiceCard
                active={form.databaseMode === "plaintext"}
                title="Plaintext preview"
                description="For mock data or throwaway evaluation only. The database remains readable on disk."
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
                    Used to unlock this UI session; the passphrase is not
                    stored in the persisted UI profile.
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
                <CheckRow
                  id="migrate-credentials"
                  checked={form.migrateCredentials}
                  onCheckedChange={(checked) =>
                    update("migrateCredentials", checked)
                  }
                  label="Move existing backend credentials into the encrypted DB."
                  description="Tokens, RPC passwords, auth headers, and usernames should not remain in backends.env."
                />
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
          </div>

          <Button
            onClick={onSubmit}
            className="w-full"
            disabled={!canOpenLedger}
          >
            Open ledger
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DatabasePanel form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
