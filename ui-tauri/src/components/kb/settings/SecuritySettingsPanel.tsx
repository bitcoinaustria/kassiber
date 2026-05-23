import {
  CheckCircle2,
  Fingerprint,
  KeyRound,
  Lock,
  RefreshCw,
  ShieldCheck,
  ShieldOff,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { AppLockPolicy } from "@/store/ui";
import { cn } from "@/lib/utils";
import { SettingsSwitchRow } from "./SettingsControls";

export function SecuritySettingsPanel({
  appLockPolicy,
  setAppLockPolicy,
  onEnrollTouchId,
  onForgetTouchId,
  encryptedWorkspace,
  touchIdPlatformSupported,
  touchIdConfigured,
  touchIdStatusPending,
  touchIdStatusReason,
  onRefreshTouchId,
  onLockNow,
  onChangePassphrase,
}: {
  appLockPolicy: AppLockPolicy;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  onEnrollTouchId: () => void;
  onForgetTouchId: () => void;
  encryptedWorkspace: boolean;
  touchIdPlatformSupported: boolean;
  touchIdConfigured: boolean;
  touchIdStatusPending: boolean;
  touchIdStatusReason: string | null;
  onRefreshTouchId: () => void;
  onLockNow: () => void;
  onChangePassphrase: () => void;
}) {
  const biometricActionable = encryptedWorkspace && touchIdPlatformSupported;
  const biometricStatus = touchIdStatusPending
    ? "Checking enrollment…"
    : touchIdConfigured
      ? "Enrolled"
      : "Not enrolled";
  const biometricDetail = encryptedWorkspace
    ? touchIdPlatformSupported
      ? touchIdConfigured
        ? "Saved for these books in this macOS user account."
        : touchIdStatusReason
          ? `Not set up: ${touchIdStatusReason}`
          : "Verify the database passphrase once to save it in macOS Keychain."
      : "Touch ID unlock is available in the macOS desktop app."
    : "Available after these books use SQLCipher encryption.";

  return (
    <div className="space-y-6">
      {/* Encryption status + the two primary actions, surfaced first. */}
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <KeyRound className="size-4" aria-hidden="true" />
          <h3 className="text-sm font-semibold">Database encryption</h3>
        </div>
        <div className="rounded-md border bg-background p-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0 space-y-1.5">
              <span
                className={cn(
                  "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium",
                  encryptedWorkspace
                    ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                    : "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
                )}
              >
                {encryptedWorkspace ? (
                  <ShieldCheck className="size-3" aria-hidden="true" />
                ) : (
                  <ShieldOff className="size-3" aria-hidden="true" />
                )}
                {encryptedWorkspace ? "Encrypted · SQLCipher" : "Not encrypted"}
              </span>
              <p className="max-w-prose text-sm text-muted-foreground">
                {encryptedWorkspace
                  ? "Locking closes the daemon's database handle; unlocking reopens the local SQLCipher database with your passphrase."
                  : "These books are stored unencrypted on this device. Lock still returns to the lock screen, but the data on disk is not protected by a passphrase."}
              </p>
            </div>
            <div className="flex shrink-0 flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={onLockNow}
              >
                <Lock className="size-4" aria-hidden="true" />
                Lock now
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={onChangePassphrase}
                disabled={!encryptedWorkspace}
              >
                <KeyRound className="size-4" aria-hidden="true" />
                Change passphrase
              </Button>
            </div>
          </div>
        </div>
      </section>

      {/* App lock */}
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Lock className="size-4" aria-hidden="true" />
          <h3 className="text-sm font-semibold">App lock</h3>
        </div>
        <div className="rounded-md border bg-background p-3">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0 space-y-1">
              <Label className="text-sm font-medium">Auto-lock when idle</Label>
              <p className="text-sm text-muted-foreground">
                Require the passphrase again after a period of inactivity.
              </p>
            </div>
            <Switch
              checked={appLockPolicy.autoLockWhenIdle}
              onCheckedChange={(checked) =>
                setAppLockPolicy({ autoLockWhenIdle: checked })
              }
            />
          </div>
          {appLockPolicy.autoLockWhenIdle ? (
            <div className="mt-3 space-y-2 border-t pt-3">
              <Label className="text-xs text-muted-foreground">Lock after</Label>
              <div className="flex flex-wrap gap-1.5">
                {[1, 5, 15, 30, 60].map((minutes) => (
                  <Button
                    key={minutes}
                    type="button"
                    size="sm"
                    variant={
                      appLockPolicy.idleMinutes === minutes
                        ? "default"
                        : "outline"
                    }
                    onClick={() => setAppLockPolicy({ idleMinutes: minutes })}
                  >
                    {minutes}m
                  </Button>
                ))}
              </div>
            </div>
          ) : null}
        </div>
        <SettingsSwitchRow
          label="Require passphrase on launch"
          description={
            encryptedWorkspace
              ? "Prompt immediately when Kassiber opens; cold starts still need the database passphrase when the daemon is locked."
              : "Prompt every time Kassiber opens."
          }
          checked={appLockPolicy.requirePassphraseOnLaunch}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ requirePassphraseOnLaunch: checked })
          }
        />
        <SettingsSwitchRow
          label="Lock on window close"
          description="Clear in-memory decrypted state when the app window closes."
          checked={appLockPolicy.lockOnWindowClose}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ lockOnWindowClose: checked })
          }
        />
      </section>

      {/* Biometric unlock */}
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Fingerprint className="size-4" aria-hidden="true" />
          <h3 className="text-sm font-semibold">Biometric unlock</h3>
        </div>
        <div className="rounded-md border bg-background p-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0 space-y-1">
              <div className="flex items-center gap-2">
                {touchIdConfigured ? (
                  <CheckCircle2
                    className="size-4 text-emerald-600 dark:text-emerald-400"
                    aria-hidden="true"
                  />
                ) : null}
                <p className="text-sm font-medium">Touch ID · {biometricStatus}</p>
              </div>
              <p className="text-sm text-muted-foreground">{biometricDetail}</p>
            </div>
            <div className="flex shrink-0 flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={!biometricActionable}
                onClick={onRefreshTouchId}
              >
                <RefreshCw className="size-4" aria-hidden="true" />
                Refresh
              </Button>
              {touchIdConfigured ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!biometricActionable}
                  onClick={onForgetTouchId}
                >
                  Forget
                </Button>
              ) : (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!biometricActionable}
                  onClick={onEnrollTouchId}
                >
                  Set up
                </Button>
              )}
            </div>
          </div>
          <div className="mt-3 flex items-start justify-between gap-4 border-t pt-3">
            <div className="min-w-0 space-y-1">
              <Label className="text-sm font-medium">
                Offer biometric unlock on the lock screen
              </Label>
              <p className="text-sm text-muted-foreground">
                Remember this device's biometric unlock preference between
                sessions.
              </p>
            </div>
            <Switch
              checked={appLockPolicy.touchIdUnlock && touchIdConfigured}
              disabled={!encryptedWorkspace || !touchIdConfigured}
              onCheckedChange={(checked) =>
                setAppLockPolicy({ touchIdUnlock: checked })
              }
            />
          </div>
        </div>
      </section>
    </div>
  );
}
