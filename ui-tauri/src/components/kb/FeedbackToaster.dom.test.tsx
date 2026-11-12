// @vitest-environment happy-dom
//
// Mounted: expiry, pause, and dismissal are Radix Toast behavior, so only a
// rendered toast proves the wrapper wires them up.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
    removeItem: (key: string) => void map.delete(key),
  });
});

import {
  clearFeedback,
  MAX_VISIBLE_FEEDBACK,
  showFeedback,
  SUCCESS_FEEDBACK_MS,
  useFeedbackStore,
} from "@/lib/feedback";
import { useUiStore } from "@/store/ui";

import { FeedbackToaster } from "./FeedbackToaster";

function toasts(): HTMLElement[] {
  return screen.queryAllByTestId("feedback-toast");
}

beforeEach(() => {
  vi.useFakeTimers();
  clearFeedback();
  useUiStore.setState({ privacyMode: "normal", lang: "en" });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("FeedbackToaster", () => {
  it("expires a success after about four seconds", () => {
    render(<FeedbackToaster />);
    act(() => void showFeedback({ tone: "success", title: "Book refreshed" }));
    expect(toasts()).toHaveLength(1);
    // Radix announces each toast once through a polite live region.
    act(() => void vi.advanceTimersByTime(50));
    const announcement = document.querySelector('[role="status"][aria-live="polite"]');
    expect(announcement?.textContent).toContain("Book refreshed");

    act(() => void vi.advanceTimersByTime(SUCCESS_FEEDBACK_MS + 50));
    expect(useFeedbackStore.getState().items).toHaveLength(0);
  });

  it("pauses while the pointer is over the toasts and resumes after", () => {
    render(<FeedbackToaster />);
    act(() => void showFeedback({ tone: "success", title: "Book refreshed" }));
    const viewport = screen.getByRole("region");

    act(() => void fireEvent.pointerMove(viewport));
    act(() => void vi.advanceTimersByTime(SUCCESS_FEEDBACK_MS * 3));
    expect(toasts()).toHaveLength(1);

    act(() => void fireEvent.pointerLeave(viewport));
    act(() => void vi.advanceTimersByTime(SUCCESS_FEEDBACK_MS + 50));
    expect(useFeedbackStore.getState().items).toHaveLength(0);
  });

  it("dismisses from the close button and from Escape", () => {
    render(<FeedbackToaster />);
    act(() => {
      showFeedback({ tone: "info", title: "First", key: "1" });
      showFeedback({ tone: "info", title: "Second", key: "2" });
    });
    act(() => {
      fireEvent.click(screen.getAllByRole("button", { name: "Dismiss" })[0]);
    });
    act(() => void vi.advanceTimersByTime(300));
    expect(useFeedbackStore.getState().items.map((item) => item.title)).toEqual([
      "Second",
    ]);

    act(() => {
      fireEvent.keyDown(toasts()[0], { key: "Escape" });
    });
    act(() => void vi.advanceTimersByTime(300));
    expect(useFeedbackStore.getState().items).toHaveLength(0);
  });

  it("coalesces equivalent receipts and shows at most three", () => {
    render(<FeedbackToaster />);
    act(() => {
      showFeedback({ tone: "success", title: "Saved", key: "saved" });
      showFeedback({ tone: "success", title: "Saved", key: "saved" });
    });
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].textContent).toContain("×2");

    act(() => {
      for (let index = 0; index < 5; index += 1) {
        showFeedback({ tone: "info", title: `Receipt ${index}` });
      }
    });
    expect(toasts()).toHaveLength(MAX_VISIBLE_FEEDBACK);
    expect(toasts().map((toast) => toast.textContent)).toEqual([
      expect.stringContaining("Receipt 2"),
      expect.stringContaining("Receipt 3"),
      expect.stringContaining("Receipt 4"),
    ]);
  });

  it("projects descriptions through the privacy mode", () => {
    render(<FeedbackToaster />);
    act(() => {
      showFeedback({
        tone: "success",
        title: "Imported",
        description: "Cold Vault: 0.42 BTC",
        key: "a",
      });
      showFeedback({
        tone: "success",
        title: "Processed",
        description: "3 transactions",
        descriptionPrivacy: "safe",
        key: "b",
      });
    });
    expect(document.body.textContent).toContain("Cold Vault: 0.42 BTC");

    act(() => void useUiStore.setState({ privacyMode: "hideAmounts" }));
    expect(document.body.textContent).not.toContain("0.42");
    expect(document.body.textContent).toContain("3 transactions");

    act(() => void useUiStore.setState({ privacyMode: "hideAll" }));
    expect(document.body.textContent).not.toContain("Cold Vault");
    expect(document.body.textContent).toContain("Imported");
  });
});
