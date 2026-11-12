// @vitest-environment happy-dom
//
// The privacy modes against the live store: the mask component, a former
// leak site (the import plan's totals), the title-bar cycle, and the
// "hide amounts when Kassiber unlocks" hook.
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

import { PlanConsentBody } from "@/components/ai/PlanConsentBody";
import { planConsentPreview } from "@/components/ai/planConsent";
import {
  isHidden,
  maskSensitive,
  SENSITIVE_MASK,
  sensitiveClass,
  useUnlockPrivacy,
} from "@/lib/sensitive";
import { useUiStore } from "@/store/ui";

import { PrivacyModeButton } from "./PrivacyModeButton";
import { Sensitive } from "./Sensitive";

const store = useUiStore.getState;

afterEach(() => {
  cleanup();
  act(() => {
    store().setHideAmountsOnUnlock(false);
    store().setPrivacyMode("normal");
  });
});

describe("privacy mode helpers", () => {
  it("hides amounts in both hiding modes and shares only in hide all", () => {
    expect(isHidden("normal")).toBe(false);
    expect(isHidden("hideAmounts")).toBe(true);
    expect(isHidden("hideAll")).toBe(true);
    expect(isHidden("normal", "share")).toBe(false);
    expect(isHidden("hideAmounts", "share")).toBe(false);
    expect(isHidden("hideAll", "share")).toBe(true);
    expect(maskSensitive("0.5 BTC", true)).toBe(SENSITIVE_MASK);
    expect(maskSensitive("0.5 BTC", false)).toBe("0.5 BTC");
    expect(sensitiveClass(true)).toBe("sensitive");
    expect(sensitiveClass(false)).toBe("");
  });
});

describe("<Sensitive>", () => {
  it("renders the value while nothing is hidden", () => {
    const { container } = render(<Sensitive>1,234.56 EUR</Sensitive>);
    expect(container.textContent).toBe("1,234.56 EUR");
  });

  it("keeps the digits out of the DOM when amounts are hidden", () => {
    act(() => store().setPrivacyMode("hideAmounts"));
    const { container } = render(<Sensitive>1,234.56 EUR</Sensitive>);
    expect(container.innerHTML).not.toMatch(/\d/);
    expect(container.textContent).toBe(SENSITIVE_MASK);
    expect(container.querySelector(".sensitive")).not.toBeNull();
    expect(screen.getByLabelText("Hidden amount")).toBeTruthy();
  });

  it("shows shares under hide amounts and masks them under hide all", () => {
    act(() => store().setPrivacyMode("hideAmounts"));
    const { container, rerender } = render(<Sensitive kind="share">42%</Sensitive>);
    expect(container.textContent).toBe("42%");
    act(() => store().setPrivacyMode("hideAll"));
    rerender(<Sensitive kind="share">42%</Sensitive>);
    expect(container.innerHTML).not.toMatch(/\d/);
    expect(screen.getByLabelText("Hidden share")).toBeTruthy();
  });
});

describe("import plan totals", () => {
  const plan = {
    ready: true,
    plan_digest: "a".repeat(64),
    target: { action: "create", label: "Broker XYZ", completeness_class: "manual" },
    rows: { in_file: 3, to_import: 2, already_present: 1, would_update_existing: 0 },
    date_range: { first: "2024-03-01T10:00:00Z", last: "2024-04-02T10:00:00Z", undated: 0 },
    totals: [{ asset: "BTC", direction: "inbound", rows: 1, amount_msat: 50_000_000_000, fee_msat: 10_000_000 }],
    overlap: { existing_rows_in_date_range: 0, external_ids_in_other_wallets: 0 },
    warnings: [],
    blockers: [],
  };
  const preview = planConsentPreview("ui.wallets.import_attachment.apply", { status: "ready", plan });

  it("prints the totals while nothing is hidden", () => {
    const { container } = render(<PlanConsentBody preview={preview} />);
    expect(container.textContent).toContain("0.5 BTC");
    expect(container.textContent).toContain("0.0001 BTC");
  });

  it("masks the amount and fee while amounts are hidden", () => {
    act(() => store().setPrivacyMode("hideAmounts"));
    const { container } = render(<PlanConsentBody preview={preview} />);
    expect(container.textContent).not.toContain("0.5 BTC");
    expect(container.textContent).not.toContain("0.0001");
    expect(container.querySelectorAll("[data-sensitive-mask]")).toHaveLength(2);
  });
});

describe("title-bar privacy button", () => {
  it("steps normal → hide amounts → hide all → normal", () => {
    render(<PrivacyModeButton />);
    const button = screen.getByRole("button");
    expect(button.getAttribute("data-privacy-mode")).toBe("normal");
    expect(button.getAttribute("aria-label")).toBe("Privacy: Show all. Switch to Hide amounts.");
    for (const mode of ["hideAmounts", "hideAll", "normal"]) {
      fireEvent.click(button);
      expect(store().privacyMode).toBe(mode);
      expect(button.getAttribute("data-privacy-mode")).toBe(mode);
    }
  });
});

describe("hide amounts when Kassiber unlocks", () => {
  it("raises the mode on each lock change only when enabled", () => {
    const { rerender } = renderHook(({ locked }) => useUnlockPrivacy(locked), {
      initialProps: { locked: true },
    });
    rerender({ locked: false });
    expect(store().privacyMode).toBe("normal");

    act(() => store().setHideAmountsOnUnlock(true));
    rerender({ locked: true });
    act(() => store().setPrivacyMode("normal"));
    rerender({ locked: false });
    expect(store().privacyMode).toBe("hideAmounts");
  });
});
