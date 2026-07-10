import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { SecuritySettingsPanel } from "./SecuritySettingsPanel";

const renderPanel = (
  protection:
    | "biometry_current_set"
    | "application_local_authentication"
    | "legacy_shared",
  stale = false,
) =>
  renderToStaticMarkup(
    <SecuritySettingsPanel
      appLockPolicy={{
        autoLockWhenIdle: false,
        idleMinutes: 5,
        requirePassphraseOnLaunch: true,
        lockOnWindowClose: true,
        touchIdUnlock: true,
      }}
      setAppLockPolicy={vi.fn()}
      onEnrollTouchId={vi.fn()}
      onForgetTouchId={vi.fn()}
      onForgetAllUnlock={vi.fn()}
      forgetAllPending={false}
      encryptedWorkspace
      touchIdPlatformSupported
      touchIdConfigured={!stale}
      touchIdStale={stale}
      touchIdProtection={protection}
      touchIdStatusPending={false}
      touchIdStatusReason={null}
      onRefreshTouchId={vi.fn()}
      onLockNow={vi.fn()}
      onChangePassphrase={vi.fn()}
    />,
  );

describe("SecuritySettingsPanel biometric protection", () => {
  it("states the item-level current-enrollment guarantee precisely", () => {
    const html = renderPanel("biometry_current_set");
    expect(html).toContain("protected by the current Touch ID enrollment");
    expect(html).toContain("Changing enrolled fingerprints invalidates it");
  });

  it("labels the preview fallback as an application gate", () => {
    const html = renderPanel("application_local_authentication");
    expect(html).toContain("Preview build");
    expect(html).toContain("checks Touch ID before reading");
  });

  it("offers independent cleanup of every unlock enrollment", () => {
    const html = renderPanel("legacy_shared");
    expect(html).toContain("Legacy shared enrollment detected");
    expect(html).toContain("Forget all unlock methods");
    expect(html).toContain("CLI remembered-unlock entry");
  });

  it("explains CLI-side rotation without attempting a stale desktop entry", () => {
    const html = renderPanel("biometry_current_set", true);
    expect(html).toContain("passphrase changed outside the desktop app");
    expect(html).toContain("re-enroll Touch ID");
  });
});
