import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import type { AppLockPolicy } from "@/store/ui";
import { cn } from "@/lib/utils";
import { SettingsGroup, SettingsRow, SettingsSelect } from "./SettingsControls";

const IDLE_MINUTES = [1, 5, 15, 30, 60];

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
  // A stored value outside the list (an older build's) stays selectable.
  const idleMinutes = IDLE_MINUTES.includes(appLockPolicy.idleMinutes)
    ? IDLE_MINUTES
    : [...IDLE_MINUTES, appLockPolicy.idleMinutes].sort((a, b) => a - b);

  return (
    <div className="space-y-6">
      <SettingsGroup
        title={t("security.encryptionHeading")}
        actions={
          <Button type="button" size="sm" variant="outline" onClick={onLockNow}>
            {t("security.lockNow")}
          </Button>
        }
      >
        <SettingsRow
          label={t("security.encryptionStatus")}
          description={
            encryptedWorkspace
              ? t("security.encryptedDetail")
              : t("security.notEncryptedDetail")
          }
        >
          <span
            className={cn(
              "text-sm",
              encryptedWorkspace
                ? "text-emerald-600 dark:text-emerald-400"
                : "text-amber-600 dark:text-amber-400",
            )}
          >
            {encryptedWorkspace
              ? t("security.encrypted")
              : t("security.notEncrypted")}
          </span>
        </SettingsRow>
        <SettingsRow
          label={t("security.passphraseLabel")}
          description={
            encryptedWorkspace
              ? t("security.passphraseDescription")
              : t("security.passphraseNeedsEncryption")
          }
        >
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={onChangePassphrase}
            disabled={!encryptedWorkspace}
          >
            {t("security.changePassphrase")}
          </Button>
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title={t("security.appLockHeading")}>
        <SettingsRow
          label={t("security.autoLockLabel")}
          description={t("security.autoLockDescription")}
          htmlFor="settings-auto-lock"
        >
          <Switch
            id="settings-auto-lock"
            checked={appLockPolicy.autoLockWhenIdle}
            onCheckedChange={(checked) =>
              setAppLockPolicy({ autoLockWhenIdle: checked })
            }
          />
        </SettingsRow>
        {appLockPolicy.autoLockWhenIdle ? (
          <SettingsRow
            label={t("security.lockAfter")}
            htmlFor="settings-lock-after"
          >
            <SettingsSelect
              value={String(appLockPolicy.idleMinutes)}
              onValueChange={(value) =>
                setAppLockPolicy({ idleMinutes: Number(value) })
              }
              options={idleMinutes.map((minutes) => ({
                value: String(minutes),
                label: t("security.minutes", { count: minutes }),
              }))}
              triggerProps={{ id: "settings-lock-after" }}
            />
          </SettingsRow>
        ) : null}
        <SettingsRow
          label={t("security.requireOnLaunchLabel")}
          description={
            encryptedWorkspace
              ? t("security.requireOnLaunchEncrypted")
              : t("security.requireOnLaunchPlaintext")
          }
          htmlFor="settings-require-on-launch"
        >
          <Switch
            id="settings-require-on-launch"
            checked={encryptedWorkspace || appLockPolicy.requirePassphraseOnLaunch}
            disabled={encryptedWorkspace}
            onCheckedChange={(checked) =>
              setAppLockPolicy({ requirePassphraseOnLaunch: checked })
            }
          />
        </SettingsRow>
        <SettingsRow
          label={t("security.lockOnCloseLabel")}
          description={t("security.lockOnCloseDescription")}
          htmlFor="settings-lock-on-close"
        >
          <Switch
            id="settings-lock-on-close"
            checked={appLockPolicy.lockOnWindowClose}
            onCheckedChange={(checked) =>
              setAppLockPolicy({ lockOnWindowClose: checked })
            }
          />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title={t("security.biometricHeading")}>
        <SettingsRow label={t("security.biometricLabel")} description={biometricDetail}>
          <span
            className={cn(
              "text-sm",
              touchIdConfigured && !touchIdStatusPending
                ? "text-emerald-600 dark:text-emerald-400"
                : "text-muted-foreground",
            )}
          >
            {biometricStatus}
          </span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={!biometricActionable}
            onClick={onRefreshTouchId}
          >
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
        </SettingsRow>
        <SettingsRow
          label={t("security.offerBiometricLabel")}
          description={t("security.offerBiometricDescription")}
          htmlFor="settings-offer-biometric"
        >
          <Switch
            id="settings-offer-biometric"
            checked={appLockPolicy.touchIdUnlock && touchIdConfigured}
            disabled={!encryptedWorkspace || !touchIdConfigured}
            onCheckedChange={(checked) =>
              setAppLockPolicy({ touchIdUnlock: checked })
            }
          />
        </SettingsRow>
        <SettingsRow
          label={t("security.savedUnlockLabel")}
          description={t("security.forgetAllDescription")}
        >
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={forgetAllPending}
            onClick={onForgetAllUnlock}
          >
            {t("security.forgetAll")}
          </Button>
        </SettingsRow>
      </SettingsGroup>
    </div>
  );
}
