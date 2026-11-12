// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { DatePicker } from "./date-picker";

beforeEach(() => {
  vi.useFakeTimers();
  // The browser's local day differs from the book's civil day.
  vi.setSystemTime(new Date(2025, 0, 1, 23, 30));
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it.each([
  ["core", "2025-01-02", "2", "1"],
  ["browser fallback", undefined, "1", "2"],
] as const)("outlines today using the %s day", (_source, today, expected, other) => {
  const onChange = vi.fn();
  const { container } = render(<DatePicker value="2025-01-15" today={today} onChange={onChange} />);
  fireEvent.click(container.querySelector("button")!);
  const outlined = screen.getByRole("button", { name: expected });
  expect(outlined.className).toContain("border-ring");
  expect(screen.getByRole("button", { name: other }).className).not.toContain("border-ring");
  expect(screen.getByRole("button", { name: "15" }).getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(outlined);
  expect(onChange).toHaveBeenCalledWith(`2025-01-0${expected}`);
});
