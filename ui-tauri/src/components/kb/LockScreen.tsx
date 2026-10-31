import {
  ArrowLeft,
  ArrowRight,
  CircleAlert,
  Fingerprint,
  Loader2,
  LockKeyhole,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import type { TouchIdPassphraseStatus } from "@/daemon/transport";
import { emitAppLog } from "@/lib/appLogs";
import {
  appCanStartTouchIdPrompt,
  canEnrollTouchIdPassphrase,
  shouldAutoPromptTouchId,
  type LockReason,
} from "@/lib/appLock";

import { KassiberWordmark } from "./KassiberWordmark";
import { WORDMARK_COLUMNS } from "./kassiberWordmarkGrid";

/** The wordmark's cell size here, and the form's width to match it. */
const MARK_CELL = 4;
const MARK_WIDTH = WORDMARK_COLUMNS * MARK_CELL;

/**
 * The lock and import-restore screens, drawn in the shell's own outline: the
 * chrome, the title bar row above (traffic lights and the alpha chip stay on
 * it), and an inset panel with the page panel's hairline and gutters. Nothing
 * of the book shows through: the panel covers the nav and the page.
 *
 * Inside, the wordmark leads, centred, with the welcome screen's type under
 * it: the title and a muted lead, and no eyebrow over them that would only
 * restate the title. The form below is exactly the wordmark's width, and the
 * way out sits apart at the foot of the panel.
 * The wordmark decodes in when the screen appears with `decrypt`, and again
 * each time `decryptKey` changes.
 */
function LockedFrame({
  title,
  lead,
  decrypt = false,
  decryptKey,
  footer,
  children,
}: {
  title: string;
  lead: string;
  decrypt?: boolean;
  decryptKey?: number;
  footer?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="fixed inset-x-0 top-(--kb-toolbar-height) bottom-0 z-50 flex bg-sidebar md:px-1.5 md:pb-1.5">
      {/* `m-auto` rather than centring the column, so a short window
          scrolls to the wordmark instead of clipping it. */}
      <div className="flex min-w-0 flex-1 flex-col overflow-y-auto overscroll-none bg-background px-6 text-foreground md:rounded-(--kb-radius-window) md:border md:border-border/70">
        <div className="m-auto flex w-full max-w-md flex-col items-center gap-10 py-12 text-center animate-in fade-in-0 slide-in-from-bottom-2 duration-500">
          <KassiberWordmark
            key={decryptKey}
            cell={MARK_CELL}
            animate={decrypt}
          />
          <header className="flex flex-col items-center gap-3">
            <h1 className="text-2xl leading-tight font-semibold tracking-tight text-balance text-ink">
              {title}
            </h1>
            <p className="text-sm leading-relaxed text-balance text-ink-2">
              {lead}
            </p>
          </header>
          <div className="w-full" style={{ maxWidth: MARK_WIDTH }}>
            {children}
          </div>
        </div>
        {footer ? (
          <div className="flex shrink-0 justify-center pb-6">{footer}</div>
        ) : null}
      </div>
    </div>
  );
}

function LockError({ id, children }: { id?: string; children: string }) {
  return (
    <p
      id={id}
      role="alert"
      className="m-0 flex items-start justify-center gap-1.5 text-xs leading-5 text-destructive"
    >
      <CircleAlert className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
      {children}
    </p>
  );
}

/** The quiet way out, in the welcome screen's link style. */
function LockFooter({
  label,
  disabled,
  onClick,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="group -mx-1 inline-flex items-center gap-1.5 rounded-md px-1 py-0.5 text-sm font-medium text-ink-2 underline-offset-4 outline-none transition-colors hover:text-ink hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50"
    >
      <ArrowLeft
        className="size-3.5 transition-transform group-hover:-translate-x-0.5"
        aria-hidden="true"
      />
      {label}
    </button>
  );
}

/**
 * A rejected unlock call carries native or transport detail the screen does
 * not show, so it goes to the in-memory log for a support bundle instead.
 */
function logRejectedUnlock(method: "passphrase" | "touch-id", error: unknown) {
  emitAppLog({
    level: "warning",
    module: "lock",
    file: "components/kb/LockScreen.tsx",
    line: 0,
    msg: `${method} unlock rejected: ${error instanceof Error ? error.message : String(error)}`,
    fields: {},
  });
}

export function LockScreen({
  reason,
  passphraseRequired = true,
  onUnlock,
  onTouchIdUnlock,
  touchIdEnabled,
  touchIdPlatformSupported,
  touchIdStatus,
  autoTouchIdPrompt,
  onReset,
}: {
  reason: LockReason;
  passphraseRequired?: boolean;
  onUnlock: (
    passphrase: string,
    options?: { rememberWithTouchId?: boolean },
  ) => Promise<{ ok: boolean; error?: string | null }>;
  onTouchIdUnlock: () => Promise<{ ok: boolean; error?: string | null }>;
  touchIdEnabled: boolean;
  touchIdPlatformSupported: boolean;
  touchIdStatus: TouchIdPassphraseStatus | null;
  autoTouchIdPrompt: boolean;
  onReset: () => void;
}) {
  const { t } = useTranslation("chrome");
  const [passphrase, setPassphrase] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);
  const [touchIdSubmitting, setTouchIdSubmitting] = React.useState(false);
  const autoTouchIdPrompted = React.useRef(false);
  const canEnrollTouchId = canEnrollTouchIdPassphrase({
    platformSupported: touchIdPlatformSupported,
    passphraseRequired,
    touchIdEnabled,
    touchIdAvailable: touchIdStatus?.available !== false,
    touchIdStale: touchIdStatus?.stale === true,
  });
  const [enrollTouchId, setEnrollTouchId] = React.useState(
    () => touchIdEnabled && canEnrollTouchId,
  );
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const canUseTouchId =
    touchIdEnabled &&
    touchIdPlatformSupported &&
    passphraseRequired &&
    touchIdStatus?.available === true &&
    touchIdStatus.configured;
  const busy = submitting || touchIdSubmitting;
  React.useEffect(() => {
    if (passphraseRequired) inputRef.current?.focus();
  }, [passphraseRequired]);

  // A failed attempt hands the keyboard back to the passphrase field, so the
  // next try (or the fallback from a cancelled Touch ID sheet) is typing. The
  // field is disabled during a passphrase attempt and absent during Touch ID,
  // so this waits until it is mounted and enabled again.
  const refocusPassphrase = React.useRef(false);
  React.useEffect(() => {
    if (busy || !refocusPassphrase.current) return;
    refocusPassphrase.current = false;
    if (passphraseRequired) inputRef.current?.focus();
  }, [busy, passphraseRequired]);

  React.useEffect(() => {
    if (!canEnrollTouchId) {
      setEnrollTouchId(false);
    }
  }, [canEnrollTouchId]);

  // Each unlock attempt decodes the wordmark again while the books decrypt.
  const [attempt, setAttempt] = React.useState(0);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    // An empty field cannot open the books, and asking the daemon anyway
    // would re-lock the screen as if a passphrase had been refused.
    if (passphraseRequired && passphrase === "") {
      setError(t("lock.passphraseEmpty"));
      inputRef.current?.focus();
      return;
    }
    setError(null);
    setSubmitting(true);
    setAttempt((count) => count + 1);
    try {
      const result = await onUnlock(passphrase, {
        rememberWithTouchId: canEnrollTouchId ? enrollTouchId : undefined,
      });
      if (!result.ok) {
        setError(result.error ?? t("lock.passphraseFailed"));
        setPassphrase("");
        refocusPassphrase.current = true;
      }
    } catch (rejection) {
      logRejectedUnlock("passphrase", rejection);
      setError(t("lock.unlockFailed"));
      refocusPassphrase.current = true;
    } finally {
      setSubmitting(false);
    }
  };

  const submitTouchId = React.useCallback(async () => {
    if (touchIdSubmitting || submitting) return;
    let keepPending = false;
    setError(null);
    setTouchIdSubmitting(true);
    setAttempt((count) => count + 1);
    try {
      const result = await onTouchIdUnlock();
      if (!result.ok) {
        setError(result.error ?? t("lock.touchIdFailed"));
        refocusPassphrase.current = true;
      } else {
        keepPending = true;
      }
    } catch (rejection) {
      // The native side rejects rather than answering when the macOS sheet
      // is cancelled or the Keychain refuses the read; its text is a raw
      // Security framework code, so say it in the user's language instead.
      logRejectedUnlock("touch-id", rejection);
      setError(t("lock.touchIdFailed"));
      refocusPassphrase.current = true;
    } finally {
      if (!keepPending) {
        setTouchIdSubmitting(false);
      }
    }
  }, [onTouchIdUnlock, submitting, t, touchIdSubmitting]);

  React.useEffect(() => {
    if (!autoTouchIdPrompt || !canUseTouchId) return;
    if (autoTouchIdPrompted.current) return;
    const { appVisible, windowFocused } = appCanStartTouchIdPrompt();
    autoTouchIdPrompted.current = true;
    if (
      !shouldAutoPromptTouchId({
        autoPromptRequested: autoTouchIdPrompt,
        canUseTouchId,
        appVisible,
        windowFocused,
      })
    ) {
      return;
    }
    void submitTouchId();
  }, [autoTouchIdPrompt, canUseTouchId, submitTouchId]);

  // Why Touch ID is not on offer, when the user turned it on.
  const touchIdNote =
    touchIdEnabled && touchIdPlatformSupported && passphraseRequired
      ? touchIdStatus?.available === false
        ? touchIdStatus.reason
          ? t("lock.touchIdUnavailableReason", { reason: touchIdStatus.reason })
          : t("lock.touchIdUnavailable")
        : touchIdStatus?.available === true && !touchIdStatus.configured
          ? touchIdStatus.reason
            ? t("lock.touchIdNotSetUpReason", { reason: touchIdStatus.reason })
            : t("lock.touchIdNotSetUp")
          : null
      : null;
  const unlockLabel = submitting
    ? t("lock.unlocking")
    : passphraseRequired
      ? t("lock.unlock")
      : t("lock.openBooks");

  return (
    <LockedFrame
      decrypt
      decryptKey={attempt}
      title={passphraseRequired ? t("lock.title") : t("lock.sessionTitle")}
      lead={touchIdSubmitting ? t("lock.touchIdLead") : t(`lock.lead.${reason}`)}
      footer={
        <LockFooter
          label={t("lock.backToSetup")}
          disabled={submitting}
          onClick={onReset}
        />
      }
    >
      {touchIdSubmitting ? (
        <div
          role="status"
          className="flex min-h-11 items-center justify-center gap-2 text-sm text-ink-2"
        >
          <Fingerprint className="size-5" aria-hidden="true" />
          {t("lock.waitingForTouchId")}
        </div>
      ) : (
        <form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            void submit(event);
          }}
        >
          {passphraseRequired ? (
            <div className="relative">
              <label htmlFor="lock-passphrase" className="sr-only">
                {t("lock.passphrase")}
              </label>
              {/* The screen's one lock, where the key goes. */}
              <LockKeyhole
                className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-ink-3"
                aria-hidden="true"
              />
              <Input
                id="lock-passphrase"
                ref={inputRef}
                type="password"
                autoComplete="current-password"
                placeholder={t("lock.passphrase")}
                className="h-11 pr-12 pl-10"
                value={passphrase}
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? "lock-error" : undefined}
                onChange={(event) => {
                  setPassphrase(event.target.value);
                  if (error) setError(null);
                }}
                disabled={busy}
              />
              {/* Placed by a wrapper: `kb-hit-floor` keeps every button
                  `position: relative`, outranking an `absolute` utility. */}
              <div className="absolute inset-y-0 right-1.5 flex items-center">
                <Button
                  type="submit"
                  size="icon-sm"
                  disabled={busy}
                  aria-label={unlockLabel}
                  title={unlockLabel}
                >
                  {submitting ? (
                    <Loader2 className="animate-spin" aria-hidden="true" />
                  ) : (
                    <ArrowRight aria-hidden="true" />
                  )}
                </Button>
              </div>
            </div>
          ) : (
            <Button
              size="lg"
              type="submit"
              className="group w-full"
              disabled={busy}
            >
              {submitting ? (
                <Loader2 className="animate-spin" aria-hidden="true" />
              ) : null}
              {unlockLabel}
              {submitting ? null : (
                <ArrowRight
                  className="transition-transform group-hover:translate-x-0.5"
                  aria-hidden="true"
                />
              )}
            </Button>
          )}
          {/* A line stays reserved for the error, so a wrong passphrase does
              not jolt the centred form upwards. */}
          <div className="min-h-5">
            {error ? <LockError id="lock-error">{error}</LockError> : null}
          </div>
          {canUseTouchId ? (
            <Button
              size="lg"
              type="button"
              variant="outline"
              className="w-full"
              disabled={busy}
              onClick={() => {
                void submitTouchId();
              }}
            >
              <Fingerprint aria-hidden="true" />
              {t("lock.unlockWithTouchId")}
            </Button>
          ) : null}
          {touchIdNote ? (
            <p className="m-0 text-xs leading-5 text-ink-3">{touchIdNote}</p>
          ) : null}
          {canEnrollTouchId ? (
            <label
              htmlFor="lock-touch-id-enroll"
              className="mt-2 flex items-center justify-between gap-4 rounded-(--kb-radius-card) border border-border/70 bg-card/40 px-4 py-3 text-left"
            >
              <span className="min-w-0 space-y-1">
                <span className="block text-sm font-medium text-ink">
                  {t("lock.useTouchIdNextTime")}
                </span>
                <span className="block text-xs leading-normal text-ink-3">
                  {t("lock.useTouchIdNextTimeBody")}
                </span>
              </span>
              <Switch
                id="lock-touch-id-enroll"
                checked={enrollTouchId}
                disabled={submitting}
                onCheckedChange={setEnrollTouchId}
              />
            </label>
          ) : null}
        </form>
      )}
    </LockedFrame>
  );
}

export function ImportRootRestoreScreen({
  error,
  onReset,
}: {
  error: string | null;
  onReset: () => void;
}) {
  const { t } = useTranslation("chrome");
  return (
    <LockedFrame
      title={t("importRoot.title")}
      lead={t("importRoot.body")}
      footer={
        error ? (
          <LockFooter label={t("importRoot.backToSetup")} onClick={onReset} />
        ) : null
      }
    >
      {error ? (
        <LockError>{error}</LockError>
      ) : (
        // The first-sync card's indeterminate bar: opening reports no steps.
        <div
          role="status"
          className="h-1 w-full overflow-hidden rounded-full bg-primary/15"
        >
          <span className="sr-only">{t("importRoot.title")}</span>
          <div className="h-full w-1/3 rounded-full bg-primary/80 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto" />
        </div>
      )}
    </LockedFrame>
  );
}
