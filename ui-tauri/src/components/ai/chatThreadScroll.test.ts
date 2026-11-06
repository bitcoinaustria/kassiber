import { describe, expect, it } from "vitest";

import {
  resolveScrollAffordances,
  resolveScrollMode,
} from "./chatThreadScroll";

describe("resolveScrollMode", () => {
  it("opts out of following when the reader scrolls backwards", () => {
    // The scrollbar and PageUp/Home fire no wheel/touchmove event, so this is
    // the only thing that stops streaming deltas yanking the view back down.
    expect(
      resolveScrollMode("following", { atBottom: false, scrolledUp: true }),
    ).toBe("free");
  });

  it("opts out mid-anchor too", () => {
    expect(
      resolveScrollMode("anchoring", { atBottom: false, scrolledUp: true }),
    ).toBe("free");
  });

  it("ignores forward movement so our own scrolls don't opt out", () => {
    expect(
      resolveScrollMode("following", { atBottom: false, scrolledUp: false }),
    ).toBe("following");
    expect(
      resolveScrollMode("anchoring", { atBottom: false, scrolledUp: false }),
    ).toBe("anchoring");
  });

  it("re-engages following at the live edge", () => {
    expect(
      resolveScrollMode("free", { atBottom: true, scrolledUp: true }),
    ).toBe("following");
  });

  it("holds the anchor when a shrinking turn clamps back to the edge", () => {
    expect(
      resolveScrollMode("anchoring", { atBottom: true, scrolledUp: true }),
    ).toBe("anchoring");
  });
});

describe("resolveScrollAffordances", () => {
  it("hides jump-to-latest while following or anchoring", () => {
    expect(resolveScrollAffordances("following", { atBottom: false })).toEqual({
      jumpToLatest: false,
    });
    expect(resolveScrollAffordances("anchoring", { atBottom: false })).toEqual({
      jumpToLatest: false,
    });
  });

  it("offers jump-to-latest only while free of the live edge", () => {
    expect(resolveScrollAffordances("free", { atBottom: false })).toEqual({
      jumpToLatest: true,
    });
    expect(resolveScrollAffordances("free", { atBottom: true })).toEqual({
      jumpToLatest: false,
    });
  });
});
