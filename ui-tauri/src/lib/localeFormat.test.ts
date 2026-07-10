import { afterEach, describe, expect, it } from "vitest";

import { useUiStore } from "@/store/ui";

import {
  currentUiLocale,
  formatCount,
  formatSats,
  formatUiNumber,
} from "./localeFormat";

const originalLanguage = useUiStore.getState().lang;

afterEach(() => {
  useUiStore.setState({ lang: originalLanguage });
});

describe("localeFormat", () => {
  it("formats counts and sats using the active German UI locale", () => {
    useUiStore.setState({ lang: "de" });

    expect(currentUiLocale()).toBe("de-AT");
    expect(formatCount(1234)).toBe("1\u00a0234");
    expect(formatSats(1234)).toBe("1\u00a0234 sats");
  });

  it("formats counts and sats using the active English UI locale", () => {
    useUiStore.setState({ lang: "en" });

    expect(currentUiLocale()).toBe("en-US");
    expect(formatCount(1234)).toBe("1,234");
    expect(formatSats(1234, { unit: "sat" })).toBe("1,234 sat");
  });

  it("supports language-sensitive fractional values", () => {
    expect(
      formatUiNumber(
        1234.5,
        { minimumFractionDigits: 2, maximumFractionDigits: 2 },
        "de",
      ),
    ).toBe("1\u00a0234,50");
  });
});
