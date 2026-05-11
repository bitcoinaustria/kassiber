/**
 * Pins the keymap helper's typing-target guard, key-matching shape,
 * and the first-match-wins dispatcher.
 *
 * Pure-function tests only — DOM-dependent paths are covered by the
 * route-level component tests once they land.
 */

import { describe, expect, test, vi } from "vitest";

import { matchesKey, pickBinding, type Keybinding } from "./keymap";

const noop = vi.fn();

function binding(overrides: Partial<Keybinding>): Keybinding {
  return {
    keys: "p",
    description: "test",
    handler: noop,
    ...overrides,
  };
}

describe("matchesKey", () => {
  test("matches by exact key", () => {
    expect(matchesKey({ key: "p" }, "p")).toBe(true);
  });

  test("matches case-insensitively", () => {
    expect(matchesKey({ key: "P" }, "p")).toBe(true);
  });

  test("accepts arrays of alias keys", () => {
    expect(matchesKey({ key: "ArrowDown" }, ["j", "ArrowDown"])).toBe(true);
  });

  test("returns false on mismatch", () => {
    expect(matchesKey({ key: "x" }, "p")).toBe(false);
  });
});

describe("pickBinding", () => {
  test("returns the first matching binding", () => {
    const first = binding({ keys: "p", description: "first" });
    const second = binding({ keys: "p", description: "second" });
    expect(pickBinding([first, second], { key: "p", target: null }, false)).toBe(first);
  });

  test("skips bindings when typing and allowInInput is false", () => {
    const b = binding({ keys: "p" });
    expect(pickBinding([b], { key: "p", target: null }, true)).toBeNull();
  });

  test("includes binding when typing and allowInInput is true", () => {
    const b = binding({ keys: "Escape", allowInInput: true });
    expect(pickBinding([b], { key: "Escape", target: null }, true)).toBe(b);
  });

  test("respects custom match predicates", () => {
    const b = binding({ keys: "p", match: () => false });
    expect(pickBinding([b], { key: "p", target: null }, false)).toBeNull();
  });

  test("returns null when no binding applies", () => {
    expect(pickBinding([], { key: "p", target: null }, false)).toBeNull();
  });
});
