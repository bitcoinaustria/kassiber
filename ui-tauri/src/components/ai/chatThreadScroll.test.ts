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
  it("offers jump-to-top in a settled thread scrolled away from the top", () => {
    expect(
      resolveScrollAffordances("following", { atBottom: true, atTop: false }),
    ).toEqual({ jumpToLatest: false, jumpToTop: true });
  });

  it("hides both pills while an anchored turn is held", () => {
    expect(
      resolveScrollAffordances("anchoring", { atBottom: true, atTop: false }),
    ).toEqual({ jumpToLatest: false, jumpToTop: false });
  });

  it("offers jump-to-latest only while free of the live edge", () => {
    expect(
      resolveScrollAffordances("free", { atBottom: false, atTop: false }),
    ).toEqual({ jumpToLatest: true, jumpToTop: true });
    expect(
      resolveScrollAffordances("free", { atBottom: true, atTop: false }),
    ).toEqual({ jumpToLatest: false, jumpToTop: true });
  });

  it("hides jump-to-top at the top of the thread", () => {
    expect(
      resolveScrollAffordances("free", { atBottom: false, atTop: true }),
    ).toEqual({ jumpToLatest: true, jumpToTop: false });
  });
});
