// @vitest-environment happy-dom
//
// Mounted: the switch writes the persisted UI store and re-renders.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// The UI store persists the currency switch; give it somewhere to write.
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

import "@/i18n";
import { useCurrency } from "@/lib/currency";
import { useUiStore } from "@/store/ui";

import { CurrencyToggleText } from "./CurrencyToggleText";

afterEach(() => {
  cleanup();
  useUiStore.setState({ currency: "btc" });
});

function Amount() {
  const currency = useCurrency();
  return <CurrencyToggleText>{currency === "eur" ? "€ 99,50" : "₿ 0.00199000"}</CurrencyToggleText>;
}

describe("currency toggle text", () => {
  it("keeps the shown amount as its name and describes the switch, in both currencies", () => {
    render(<Amount />);
    const toFiat = screen.getByRole("button", {
      name: "₿ 0.00199000",
      description: "Show amounts in fiat",
    });
    fireEvent.click(toFiat);
    expect(useUiStore.getState().currency).toBe("eur");
    screen.getByRole("button", { name: "€ 99,50", description: "Show amounts in bitcoin" });
  });

  it("switches from the keyboard with Enter or Space", () => {
    render(<Amount />);
    fireEvent.keyDown(screen.getByRole("button"), { key: "Enter" });
    expect(useUiStore.getState().currency).toBe("eur");
    fireEvent.keyDown(screen.getByRole("button"), { key: " " });
    expect(useUiStore.getState().currency).toBe("btc");
    fireEvent.keyDown(screen.getByRole("button"), { key: "a" });
    expect(useUiStore.getState().currency).toBe("btc");
  });
});
