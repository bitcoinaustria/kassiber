import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useUiStore } from "@/store/ui";
import { copyTextWithPolicy } from "./clipboard";

describe("copyTextWithPolicy", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    let current = "";
    Object.assign(navigator, {
      clipboard: {
        readText: vi.fn(async () => current),
        writeText: vi.fn(async (value: string) => {
          current = value;
        }),
      },
    });
    useUiStore.setState({ clearClipboard: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("clears copied text after the configured delay when unchanged", async () => {
    await copyTextWithPolicy("secret");
    await vi.advanceTimersByTimeAsync(30_000);

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("secret");
    expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith("");
  });

  it("leaves later clipboard content alone", async () => {
    await copyTextWithPolicy("secret");
    await navigator.clipboard.writeText("other");
    await vi.advanceTimersByTimeAsync(30_000);

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("secret");
    expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith("other");
  });
});
