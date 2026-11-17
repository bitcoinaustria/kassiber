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
    expect(html).toContain("Tied to your current fingerprints");
    expect(html).toContain("Adding or removing one means setting it up again");
  });

  it("labels the preview fallback as an application gate", () => {
    const html = renderPanel("application_local_authentication");
    expect(html).toContain("Preview build");
    expect(html).toContain("asks for Touch ID before reading the saved passphrase");
    expect(html).toContain("not bound to Touch ID");
  });

  it("offers independent cleanup of every unlock enrollment", () => {
    const html = renderPanel("legacy_shared");
    expect(html).toContain("Set up by an older version");
    expect(html).toContain("Saved unlock methods");
    expect(html).toContain("Forget all");
    expect(html).toContain("saved unlock for these books, including entries from older versions");
  });

  it("explains CLI-side rotation without attempting a stale desktop entry", () => {
    const html = renderPanel("biometry_current_set", true);
    expect(html).toContain("passphrase was changed outside the app");
    expect(html).toContain("Set up Touch ID again");
  });
});
