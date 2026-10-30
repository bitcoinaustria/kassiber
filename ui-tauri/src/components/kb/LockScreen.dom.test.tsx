// @vitest-environment happy-dom
//
// Mounted, not static: the lock screen's contract is interactive — submitting,
// the inline error, and which unlock paths it offers for the reason and the
// Touch ID state it was given.
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import type * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { TouchIdPassphraseStatus } from "@/daemon/transport";
import { clearAppLogRecords, getAppLogRecords } from "@/lib/appLogs";

import { ImportRootRestoreScreen, LockScreen } from "./LockScreen";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  clearAppLogRecords();
});

type LockProps = React.ComponentProps<typeof LockScreen>;

function renderLock(overrides: Partial<LockProps> = {}) {
  const props: LockProps = {
    reason: "encrypted",
    passphraseRequired: true,
    onUnlock: vi.fn(async () => ({ ok: true })),
    onTouchIdUnlock: vi.fn(async () => ({ ok: true })),
    touchIdEnabled: false,
    touchIdPlatformSupported: false,
    touchIdStatus: null,
    autoTouchIdPrompt: false,
    onReset: vi.fn(),
    ...overrides,
  };
  render(<LockScreen {...props} />);
  return props;
}

/**
 * A browser blurs a field once it is disabled mid-attempt; happy-dom keeps the
 * focus, which would hide whether the screen hands it back afterwards.
 */
function dropFocusLikeABrowser() {
  (document.activeElement as HTMLElement | null)?.blur();
}

const configuredTouchId: TouchIdPassphraseStatus = {
  platform: "macos",
  available: true,
  configured: true,
  stale: false,
};

describe("LockScreen", () => {
  it("asks encrypted books for the passphrase and submits it", async () => {
    const props = renderLock();

    expect(
      screen.getByRole("heading", { name: "Unlock your books" }),
    ).toBeTruthy();
    expect(
      screen.getByText("Enter the passphrase to decrypt them on this device."),
    ).toBeTruthy();
    const field = screen.getByLabelText("Passphrase");
    expect(document.activeElement).toBe(field);

    fireEvent.change(field, { target: { value: "correct horse" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));
    });

    expect(props.onUnlock).toHaveBeenCalledWith("correct horse", {
      rememberWithTouchId: undefined,
    });
  });

  it("explains a daemon that asked for the passphrase again", () => {
    renderLock({ reason: "daemon-auth" });

    expect(
      screen.getByText(
        "Kassiber needs the passphrase again before it can show them.",
      ),
    ).toBeTruthy();
  });

  it("shows a failed unlock inline, clears the field and the error on typing", async () => {
    renderLock({
      onUnlock: vi.fn(async () => {
        dropFocusLikeABrowser();
        return {
          ok: false,
          error: "That passphrase did not unlock these books.",
        };
      }),
    });
    const field = screen.getByLabelText("Passphrase") as HTMLInputElement;

    fireEvent.change(field, { target: { value: "wrong" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));
    });

    expect(screen.getByRole("alert").textContent).toBe(
      "That passphrase did not unlock these books.",
    );
    expect(field.value).toBe("");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.getAttribute("aria-describedby")).toBe("lock-error");

    // Focus comes back once the field is enabled again, ready for a retry.
    expect(document.activeElement).toBe(field);

    fireEvent.change(field, { target: { value: "w" } });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(field.getAttribute("aria-invalid")).toBeNull();
  });

  it("asks for the passphrase instead of sending an empty one", async () => {
    const props = renderLock();
    const field = screen.getByLabelText("Passphrase");
    dropFocusLikeABrowser();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));
    });

    expect(props.onUnlock).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe(
      "Enter the passphrase first.",
    );
    expect(document.activeElement).toBe(field);
  });

  it("keeps the passphrase and logs the detail when the unlock call itself fails", async () => {
    renderLock({
      onUnlock: vi.fn(async () => {
        dropFocusLikeABrowser();
        throw new Error("daemon supervisor is not running");
      }),
    });
    const field = screen.getByLabelText("Passphrase") as HTMLInputElement;

    fireEvent.change(field, { target: { value: "correct horse" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));
    });

    expect(screen.getByRole("alert").textContent).toBe(
      "Kassiber could not unlock these books. Try again.",
    );
    // The passphrase was never refused, so a retry needs no retyping.
    expect(field.value).toBe("correct horse");
    expect(document.activeElement).toBe(field);
    expect(getAppLogRecords().map((record) => record.msg)).toContain(
      "passphrase unlock rejected: daemon supervisor is not running",
    );
  });

  it("opens plaintext books without a passphrase field", async () => {
    const props = renderLock({ reason: "session", passphraseRequired: false });

    expect(
      screen.getByRole("heading", { name: "Your books are locked" }),
    ).toBeTruthy();
    expect(screen.queryByLabelText("Passphrase")).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Open books/ }));
    });

    expect(props.onUnlock).toHaveBeenCalledWith("", {
      rememberWithTouchId: undefined,
    });
  });

  it("offers Touch ID only once it is set up for these books", async () => {
    const props = renderLock({
      touchIdEnabled: true,
      touchIdPlatformSupported: true,
      touchIdStatus: configuredTouchId,
    });

    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Unlock with Touch ID" }),
      );
    });
    expect(props.onTouchIdUnlock).toHaveBeenCalledTimes(1);

    cleanup();
    renderLock({
      touchIdEnabled: true,
      touchIdPlatformSupported: true,
      touchIdStatus: { ...configuredTouchId, configured: false },
    });
    expect(
      screen.queryByRole("button", { name: "Unlock with Touch ID" }),
    ).toBeNull();
    expect(
      screen.getByText(
        "No Touch ID passphrase is saved for these books. Unlock once with the passphrase to save it again.",
      ),
    ).toBeTruthy();
  });

  it("falls back to the passphrase when the native Touch ID sheet is cancelled", async () => {
    // Tauri rejects the call instead of answering with an envelope.
    let cancel!: (reason: unknown) => void;
    const prompt = new Promise<{ ok: boolean }>((_, reject) => {
      cancel = reject;
    });
    const props = renderLock({
      reason: "daemon-auth",
      touchIdEnabled: true,
      touchIdPlatformSupported: true,
      touchIdStatus: configuredTouchId,
      onTouchIdUnlock: vi.fn(() => prompt),
    });
    fireEvent.change(screen.getByLabelText("Passphrase"), {
      target: { value: "correct horse" },
    });

    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Unlock with Touch ID" }),
      );
    });

    expect(screen.queryByLabelText("Passphrase")).toBeNull();
    expect(screen.queryByRole("button", { name: "Unlock" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Touch ID/ })).toBeNull();
    expect(screen.getByRole("status").textContent).toBe(
      "Waiting for Touch ID...",
    );
    expect(
      screen.getByText("Confirm with Touch ID in the macOS prompt."),
    ).toBeTruthy();
    expect(
      screen.queryByText(
        "Kassiber needs the passphrase again before it can show them.",
      ),
    ).toBeNull();
    expect(props.onUnlock).not.toHaveBeenCalled();

    await act(async () => {
      cancel("macOS protected Keychain rejected the operation (-128).");
    });

    expect(screen.getByRole("alert").textContent).toBe(
      "Touch ID did not unlock these books.",
    );
    const field = screen.getByLabelText("Passphrase") as HTMLInputElement;
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe("correct horse");
    expect(screen.queryByRole("status")).toBeNull();
    expect(
      screen.getByText(
        "Kassiber needs the passphrase again before it can show them.",
      ),
    ).toBeTruthy();
    expect(getAppLogRecords().map((record) => record.msg)).toContain(
      "touch-id unlock rejected: macOS protected Keychain rejected the operation (-128).",
    );

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));
    });
    expect(props.onUnlock).toHaveBeenCalledWith("correct horse", {
      rememberWithTouchId: undefined,
    });
  });

  it.each([true, false])(
    "shows only Touch ID while an automatic prompt runs (ok=%s)",
    async (ok) => {
      vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      let finish!: (result: { ok: boolean; error?: string }) => void;
      const prompt = new Promise<{ ok: boolean; error?: string }>((resolve) => {
        finish = resolve;
      });
      const props = renderLock({
        touchIdEnabled: true,
        touchIdPlatformSupported: true,
        touchIdStatus: configuredTouchId,
        autoTouchIdPrompt: true,
        onTouchIdUnlock: vi.fn(() => prompt),
      });

      expect(props.onTouchIdUnlock).toHaveBeenCalledTimes(1);
      expect(screen.queryByLabelText("Passphrase")).toBeNull();
      expect(screen.getByRole("status").textContent).toBe(
        "Waiting for Touch ID...",
      );

      await act(async () => {
        finish({ ok, error: ok ? undefined : "The saved passphrase is stale." });
      });

      if (ok) {
        // Keep the waiting view until AppShell removes the lock screen.
        expect(screen.queryByLabelText("Passphrase")).toBeNull();
        expect(screen.getByRole("status")).toBeTruthy();
      } else {
        expect(screen.getByRole("alert").textContent).toBe(
          "The saved passphrase is stale.",
        );
        expect(document.activeElement).toBe(screen.getByLabelText("Passphrase"));
      }
      expect(props.onTouchIdUnlock).toHaveBeenCalledTimes(1);
      expect(props.onUnlock).not.toHaveBeenCalled();
    },
  );

  it("asks to save the passphrase for Touch ID when the switch is on", async () => {
    const props = renderLock({
      touchIdPlatformSupported: true,
      touchIdStatus: { ...configuredTouchId, configured: false },
    });

    fireEvent.click(screen.getByRole("switch"));
    fireEvent.change(screen.getByLabelText("Passphrase"), {
      target: { value: "correct horse" },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Unlock" }));
    });

    expect(props.onUnlock).toHaveBeenCalledWith("correct horse", {
      rememberWithTouchId: true,
    });
  });

  it("starts Touch ID by itself only while the window has focus", async () => {
    const autoPrompt = {
      touchIdEnabled: true,
      touchIdPlatformSupported: true,
      touchIdStatus: configuredTouchId,
      autoTouchIdPrompt: true,
    };
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(false);

    const background = renderLock(autoPrompt);
    await act(async () => {});
    expect(background.onTouchIdUnlock).not.toHaveBeenCalled();

    cleanup();
    hasFocus.mockReturnValue(true);
    const foreground = renderLock(autoPrompt);
    await act(async () => {});
    expect(foreground.onTouchIdUnlock).toHaveBeenCalledTimes(1);
  });

  it("goes back to setup from the footer", () => {
    const props = renderLock();

    fireEvent.click(screen.getByRole("button", { name: "Back to setup" }));

    expect(props.onReset).toHaveBeenCalledTimes(1);
  });
});

describe("ImportRootRestoreScreen", () => {
  it("shows progress while the folder opens, with no way out yet", () => {
    render(<ImportRootRestoreScreen error={null} onReset={vi.fn()} />);

    expect(
      screen.getByRole("heading", { name: "Opening local books" }),
    ).toBeTruthy();
    expect(screen.getByRole("status")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Back to setup" })).toBeNull();
  });

  it("reports a failed open and offers setup again", () => {
    const onReset = vi.fn();
    render(
      <ImportRootRestoreScreen
        error="Could not open the selected local books folder."
        onReset={onReset}
      />,
    );

    expect(screen.getByRole("alert").textContent).toBe(
      "Could not open the selected local books folder.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Back to setup" }));
    expect(onReset).toHaveBeenCalledTimes(1);
  });
});
