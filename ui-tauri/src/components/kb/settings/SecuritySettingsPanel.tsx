import {
  CheckCircle2,
  Fingerprint,
  KeyRound,
  Lock,
  RefreshCw,
  ShieldCheck,
  ShieldOff,
} from "lucide-react";
import { useTranslation } from "react-i18next";

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
  onForgetAllUnlock,
  forgetAllPending,
  encryptedWorkspace,
  touchIdPlatformSupported,
  touchIdConfigured,
  touchIdStale,
  touchIdProtection,
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
  onForgetAllUnlock: () => void;
  forgetAllPending: boolean;
  encryptedWorkspace: boolean;
  touchIdPlatformSupported: boolean;
  touchIdConfigured: boolean;
  touchIdStale: boolean;
  touchIdProtection:
    | "biometry_current_set"
    | "application_local_authentication"
    | "legacy_shared"
    | null;
  touchIdStatusPending: boolean;
  touchIdStatusReason: string | null;
  onRefreshTouchId: () => void;
  onLockNow: () => void;
  onChangePassphrase: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const biometricActionable = encryptedWorkspace && touchIdPlatformSupported;
  const biometricStatus = touchIdStatusPending
    ? t("security.biometricChecking")
    : touchIdConfigured
      ? t("security.biometricEnrolled")
      : t("security.biometricNotEnrolled");
  const biometricDetail = encryptedWorkspace
    ? touchIdPlatformSupported
      ? touchIdConfigured
        ? touchIdProtection === "biometry_current_set"
          ? t("security.biometricSavedProtected")
          : touchIdProtection === "legacy_shared"
            ? t("security.biometricSavedLegacy")
            : t("security.biometricSavedAppGated")
        : touchIdStale
          ? t("security.biometricStale")
          : touchIdStatusReason
          ? t("security.biometricNotSetUpReason", { reason: touchIdStatusReason })
          : t("security.biometricVerifyHint")
      : t("security.biometricAvailableMacos")
    : t("security.biometricNeedsEncryption");

  return (
    <div className="space-y-6">
      {/* Encryption status + the two primary actions, surfaced first. */}
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <KeyRound className="size-4" aria-hidden="true" />
          <h3 className="text-sm font-semibold">
            {t("security.encryptionHeading")}
          </h3>
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
                {encryptedWorkspace
                  ? t("security.encrypted")
                  : t("security.notEncrypted")}
              </span>
              <p className="max-w-prose text-sm text-muted-foreground">
                {encryptedWorkspace
                  ? t("security.encryptedDetail")
                  : t("security.notEncryptedDetail")}
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
                {t("security.lockNow")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={onChangePassphrase}
                disabled={!encryptedWorkspace}
              >
                <KeyRound className="size-4" aria-hidden="true" />
                {t("security.changePassphrase")}
              </Button>
            </div>
          </div>
        </div>
      </section>

      {/* App lock */}
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Lock className="size-4" aria-hidden="true" />
          <h3 className="text-sm font-semibold">
            {t("security.appLockHeading")}
          </h3>
        </div>
        <div className="rounded-md border bg-background p-3">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0 space-y-1">
              <Label className="text-sm font-medium">
                {t("security.autoLockLabel")}
              </Label>
              <p className="text-sm text-muted-foreground">
                {t("security.autoLockDescription")}
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
              <Label className="text-xs text-muted-foreground">
                {t("security.lockAfter")}
              </Label>
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
                    {t("security.minutes", { count: minutes })}
                  </Button>
                ))}
              </div>
            </div>
          ) : null}
        </div>
        <SettingsSwitchRow
          label={t("security.requireOnLaunchLabel")}
          description={
            encryptedWorkspace
              ? t("security.requireOnLaunchEncrypted")
              : t("security.requireOnLaunchPlaintext")
          }
          checked={encryptedWorkspace || appLockPolicy.requirePassphraseOnLaunch}
          disabled={encryptedWorkspace}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ requirePassphraseOnLaunch: checked })
          }
        />
        <SettingsSwitchRow
          label={t("security.lockOnCloseLabel")}
          description={t("security.lockOnCloseDescription")}
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
          <h3 className="text-sm font-semibold">
            {t("security.biometricHeading")}
          </h3>
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
                <p className="text-sm font-medium">
                  {t("security.biometricTitle", { status: biometricStatus })}
                </p>
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
                {t("common:actions.refresh")}
              </Button>
              {touchIdConfigured ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!biometricActionable}
                  onClick={onForgetTouchId}
                >
                  {t("security.forget")}
                </Button>
              ) : (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!biometricActionable}
                  onClick={onEnrollTouchId}
                >
                  {t("security.setUp")}
                </Button>
              )}
            </div>
          </div>
          <div className="mt-3 flex items-start justify-between gap-4 border-t pt-3">
            <div className="min-w-0 space-y-1">
              <Label className="text-sm font-medium">
                {t("security.offerBiometricLabel")}
              </Label>
              <p className="text-sm text-muted-foreground">
                {t("security.offerBiometricDescription")}
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
          <div className="mt-3 flex items-center justify-between gap-4 border-t pt-3">
            <p className="text-sm text-muted-foreground">
              {t("security.forgetAllDescription")}
            </p>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={forgetAllPending}
              onClick={onForgetAllUnlock}
            >
              {t("security.forgetAll")}
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}
