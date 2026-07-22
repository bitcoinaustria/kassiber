import { describe, expect, it, vi } from "vitest";

import {
  APP_UPDATE_PERIOD_MS,
  APP_UPDATE_START_DELAY_MS,
  runManualAppUpdateCheck,
  startAppUpdateScheduler,
} from "./appUpdate";
import { uiStatePartialForStorage, useUiStore } from "@/store/ui";

describe("app update checks", () => {
  it("uses Sparrow's delayed daily cadence", () => {
    expect(APP_UPDATE_START_DELAY_MS).toBe(10_000);
    expect(APP_UPDATE_PERIOD_MS).toBe(86_400_000);
  });

  it("keeps release information transient", () => {
    const state = {
      ...useUiStore.getState(),
      appUpdate: {
        currentVersion: "0.22.55",
        latestVersion: "0.22.56",
        releaseUrl:
          "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56",
        updateAvailable: true,
        prerelease: true,
        checkedAt: 1_784_688_800,
      },
    };

    const stored = uiStatePartialForStorage(state);
    expect(stored).not.toHaveProperty("appUpdate");
    expect(stored).toHaveProperty("automaticUpdateChecks", true);
  });

  it("starts once after the delay and repeats daily until stopped", async () => {
    vi.useFakeTimers();
    try {
      const result = {
        currentVersion: "0.22.55",
        latestVersion: "0.23.0",
        releaseUrl:
          "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.23.0",
        updateAvailable: true,
        prerelease: false,
        checkedAt: 1_784_688_800,
      };
      const check = vi.fn().mockResolvedValue(result);
      const setUpdate = vi.fn();
      const stop = startAppUpdateScheduler(check, setUpdate);

      await vi.advanceTimersByTimeAsync(APP_UPDATE_START_DELAY_MS - 1);
      expect(check).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(check).toHaveBeenCalledTimes(1);
      expect(setUpdate).toHaveBeenCalledWith(result);

      await vi.advanceTimersByTimeAsync(APP_UPDATE_PERIOD_MS);
      expect(check).toHaveBeenCalledTimes(2);

      stop();
      await vi.advanceTimersByTimeAsync(APP_UPDATE_PERIOD_MS);
      expect(check).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("offers the GitHub release after a manual check finds an update", async () => {
    const result = {
      currentVersion: "0.22.55",
      latestVersion: "0.23.0",
      releaseUrl:
        "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.23.0",
      updateAvailable: true,
      prerelease: false,
      checkedAt: 1_784_688_800,
    };
    const setUpdate = vi.fn();
    const showDialog = vi.fn().mockResolvedValue("Open GitHub");
    const openUrl = vi.fn().mockResolvedValue(undefined);

    await runManualAppUpdateCheck({
      check: vi.fn().mockResolvedValue(result),
      setUpdate,
      showDialog,
      openUrl,
    });

    expect(setUpdate).toHaveBeenCalledWith(result);
    expect(showDialog).toHaveBeenCalledWith(
      expect.stringContaining("v0.23.0"),
      expect.objectContaining({
        buttons: { ok: "Open GitHub", cancel: "Not now" },
      }),
    );
    expect(openUrl).toHaveBeenCalledWith(result.releaseUrl);
  });

  it("reports that the installed version is current", async () => {
    const setUpdate = vi.fn();
    const showDialog = vi.fn().mockResolvedValue("OK");
    const openUrl = vi.fn().mockResolvedValue(undefined);

    await runManualAppUpdateCheck({
      check: vi.fn().mockResolvedValue({
        currentVersion: "0.23.0",
        latestVersion: "0.23.0",
        releaseUrl: null,
        updateAvailable: false,
        prerelease: false,
        checkedAt: 1_784_688_800,
      }),
      setUpdate,
      showDialog,
      openUrl,
    });

    expect(setUpdate).toHaveBeenCalledTimes(1);
    expect(showDialog).toHaveBeenCalledWith(
      "Kassiber v0.23.0 is up to date.",
      expect.any(Object),
    );
    expect(openUrl).not.toHaveBeenCalled();
  });

  it("reports GitHub failures for a user-requested check", async () => {
    const setUpdate = vi.fn();
    const showDialog = vi.fn().mockResolvedValue("OK");

    await runManualAppUpdateCheck({
      check: vi.fn().mockRejectedValue(new Error("offline")),
      setUpdate,
      showDialog,
      openUrl: vi.fn(),
    });

    expect(setUpdate).not.toHaveBeenCalled();
    expect(showDialog).toHaveBeenCalledWith(
      expect.stringContaining("could not check GitHub"),
      expect.objectContaining({ kind: "error" }),
    );
  });
});
