import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowRight,
  CheckCircle2,
  Database,
  FolderOpen,
  KeyRound,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { getTransport, type ImportProjectSelection } from "@/daemon/transport";
import { cn } from "@/lib/utils";
import type { Profile, ProfilesSnapshot, Workspace } from "@/mocks/profiles";
import { useUiStore, type Identity } from "@/store/ui";

import {
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "./frame";

interface ImportProjectPanelProps {
  selection: ImportProjectSelection;
  encrypted: boolean;
  snapshot: ProfilesSnapshot | null;
  loadingProfiles: boolean;
  error: string | null;
  onCancel: () => void;
  onUnlock: (passphrase: string) => Promise<void>;
  onRefreshProfiles: () => Promise<void>;
}

export function ImportProjectPanel({
  selection,
  encrypted,
  snapshot,
  loadingProfiles,
  error,
  onCancel,
  onUnlock,
  onRefreshProfiles,
}: ImportProjectPanelProps) {
  const navigate = useNavigate();
  const setIdentity = useUiStore((state) => state.setIdentity);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const [passphrase, setPassphrase] = useState("");
  const [openingProfileId, setOpeningProfileId] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const needsPassphrase = encrypted && !snapshot;
  const profileCount =
    snapshot?.workspaces.reduce(
      (total, workspace) => total + workspace.profiles.length,
      0,
    ) ?? 0;

  const submitUnlock = () => {
    setOpenError(null);
    const submittedPassphrase = passphrase;
    void onUnlock(submittedPassphrase)
      .catch((unlockError: unknown) => {
        setOpenError(
          unlockError instanceof Error
            ? unlockError.message
            : "Could not unlock database.",
        );
      })
      .finally(() => setPassphrase(""));
  };

  const openProfile = async (workspace: Workspace, profile: Profile) => {
    if (openingProfileId) return;
    setOpenError(null);
    setOpeningProfileId(profile.id);
    try {
      const envelope = await getTransport("real").invoke<{
        activeProfileId: string;
      }>({
        kind: "ui.profiles.switch",
        args: { profile_id: profile.id },
      });
      if (envelope.kind === "auth_required") {
        throw new Error("Database passphrase is required.");
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? "Could not open books.");
      }

      const taxCountry = normalizeTaxCountry(profile.taxCountry);
      const identity: Identity = {
        name: profile.name,
        workspace: workspace.name,
        country: taxCountry === "at" ? "AT" : "Generic",
        encrypted,
        profile: profile.name,
        taxCountry,
        fiatCurrency: profile.fiatCurrency ?? workspace.currency,
        taxLongTermDays: normalizeTaxLongTermDays(
          profile.taxLongTermDays,
          taxCountry,
        ),
        gainsAlgorithm: normalizeGainsAlgorithm(
          profile.gainsAlgorithm,
          taxCountry,
        ),
        databaseMode: encrypted ? "sqlcipher" : "plaintext",
        importedProject: {
          stateRoot: selection.stateRoot,
          dataRoot: selection.dataRoot,
          database: selection.database,
        },
      };
      setDataMode("real");
      setIdentity(identity);
      void navigate({ to: "/overview" });
    } catch (profileError) {
      setOpenError(
        profileError instanceof Error
          ? profileError.message
          : "Could not open books.",
      );
    } finally {
      setOpeningProfileId(null);
    }
  };

  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Import local ledger"
        eyebrow="Import"
        currentStep={0}
        totalSteps={1}
      >
        <div className="space-y-5 py-4">
          <div className="space-y-3 rounded-lg border border-line bg-paper-2 p-3 text-sm">
            <div className="flex items-start gap-3">
              <Database className="mt-0.5 size-4 shrink-0 text-ink" />
              <div className="min-w-0">
                <p className="font-medium text-ink">Selected local data</p>
                <p className="mt-1 break-all font-mono text-xs text-ink-3">
                  {selection.stateRoot}
                </p>
              </div>
            </div>
            <div className="flex flex-wrap gap-2 text-xs text-ink-2">
              <span className="rounded-md border border-line bg-paper px-2 py-1">
                {encrypted ? "SQLCipher" : "Plaintext"}
              </span>
              {profileCount > 0 && (
                <span className="rounded-md border border-line bg-paper px-2 py-1">
                  {profileCount} book{profileCount === 1 ? "" : "s"}
                </span>
              )}
            </div>
          </div>

          {needsPassphrase ? (
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                submitUnlock();
              }}
            >
              <div className="space-y-2">
                <Label htmlFor="import-passphrase">Database passphrase</Label>
                <Input
                  id="import-passphrase"
                  autoFocus
                  type="password"
                  value={passphrase}
                  disabled={loadingProfiles}
                  onChange={(event) => {
                    setPassphrase(event.currentTarget.value);
                    if (openError) setOpenError(null);
                  }}
                />
              </div>
              <Button
                type="submit"
                className="w-full"
                disabled={!passphrase || loadingProfiles}
              >
                <KeyRound className="size-4" aria-hidden="true" />
                Unlock database
              </Button>
            </form>
          ) : (
            <ProfileList
              snapshot={snapshot}
              loading={loadingProfiles}
              openingProfileId={openingProfileId}
              onOpen={openProfile}
              onRefresh={onRefreshProfiles}
            />
          )}

          {(error || openError) && (
            <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {openError ?? error}
            </p>
          )}

          <Button
            type="button"
            variant="outline"
            className="w-full"
            disabled={loadingProfiles || Boolean(openingProfileId)}
            onClick={() => {
              setPassphrase("");
              onCancel();
            }}
          >
            Back to new setup
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="p-6">
        <div className="flex h-full flex-col justify-between rounded-lg border border-line bg-paper p-5">
          <div className="space-y-4">
            <div className="flex size-10 items-center justify-center rounded-md border border-line bg-paper-2">
              <FolderOpen className="size-5 text-ink" aria-hidden="true" />
            </div>
            <div>
              <h3 className="text-xl font-semibold tracking-normal text-ink">
                Local Kassiber data
              </h3>
              <p className="mt-2 text-sm leading-6 text-ink-2">
                The desktop daemon now points at this selected data root. Once a
                books open, the same local database powers the rest of the app.
              </p>
            </div>
          </div>
          <div className="space-y-2 border-t border-line pt-4 text-xs text-ink-3">
            <p className="break-all font-mono">{selection.dataRoot}</p>
            <p className="break-all font-mono">{selection.database}</p>
          </div>
        </div>
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
}

function ProfileList({
  snapshot,
  loading,
  openingProfileId,
  onOpen,
  onRefresh,
}: {
  snapshot: ProfilesSnapshot | null;
  loading: boolean;
  openingProfileId: string | null;
  onOpen: (workspace: Workspace, profile: Profile) => void;
  onRefresh: () => Promise<void>;
}) {
  if (loading) {
    return (
      <div className="rounded-lg border border-line bg-paper-2 px-4 py-8 text-center text-sm text-ink-2">
        Loading local books...
      </div>
    );
  }

  if (!snapshot || snapshot.workspaces.length === 0) {
    return (
      <div className="space-y-3 rounded-lg border border-line bg-paper-2 p-4 text-sm text-ink-2">
        <p>No books were found in this local data root.</p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            void onRefresh().catch(() => {});
          }}
        >
          Refresh
        </Button>
      </div>
    );
  }

  return (
    <div className="max-h-[42vh] space-y-4 overflow-y-auto pr-1">
      {snapshot.workspaces.map((workspace) => (
        <section key={workspace.id} className="space-y-2">
          <div className="flex flex-wrap items-baseline gap-2 border-b border-line pb-1">
            <h4 className="text-sm font-semibold text-ink">{workspace.name}</h4>
            <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-ink-3">
              {workspace.currency} · {workspace.jurisdiction}
            </span>
          </div>
          <div className="grid gap-2">
            {workspace.profiles.map((profile) => {
              const isOpening = openingProfileId === profile.id;
              return (
                <button
                  key={profile.id}
                  type="button"
                  disabled={Boolean(openingProfileId)}
                  onClick={() => onOpen(workspace, profile)}
                  className={cn(
                    "flex min-h-24 w-full items-start justify-between gap-3 rounded-lg border border-line bg-paper px-3 py-3 text-left transition-colors hover:bg-paper-2 disabled:cursor-not-allowed disabled:opacity-70",
                    profile.active && "border-ink bg-paper-2",
                  )}
                >
                  <div className="min-w-0 space-y-2">
                    <div className="flex items-center gap-2">
                      <p className="truncate text-sm font-medium text-ink">
                        {profile.name}
                      </p>
                      {profile.active && (
                        <CheckCircle2
                          className="size-4 shrink-0 text-accent"
                          aria-label="Active"
                        />
                      )}
                    </div>
                    <p className="line-clamp-2 text-xs leading-5 text-ink-2">
                      {profile.taxPolicy}
                    </p>
                    <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-ink-3">
                      {profile.accounts} buckets · {profile.wallets} wallets
                    </p>
                  </div>
                  <span className="inline-flex shrink-0 items-center gap-1 text-sm font-medium text-ink">
                    {isOpening ? "Opening" : "Open"}
                    <ArrowRight className="size-4" aria-hidden="true" />
                  </span>
                </button>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}

function normalizeTaxCountry(
  taxCountry: Profile["taxCountry"],
): NonNullable<Identity["taxCountry"]> {
  return taxCountry === "at" ? "at" : "generic";
}

function normalizeTaxLongTermDays(
  value: Profile["taxLongTermDays"],
  taxCountry: NonNullable<Identity["taxCountry"]>,
): number {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : taxCountry === "at"
      ? 0
      : 365;
}

const GAINS_ALGORITHMS = new Set<NonNullable<Identity["gainsAlgorithm"]>>([
  "FIFO",
  "LIFO",
  "HIFO",
  "LOFO",
  "MOVING_AVERAGE_AT",
]);

function normalizeGainsAlgorithm(
  value: Profile["gainsAlgorithm"],
  taxCountry: NonNullable<Identity["taxCountry"]>,
): NonNullable<Identity["gainsAlgorithm"]> {
  if (value && GAINS_ALGORITHMS.has(value)) {
    return value;
  }
  return taxCountry === "at" ? "MOVING_AVERAGE_AT" : "FIFO";
}
