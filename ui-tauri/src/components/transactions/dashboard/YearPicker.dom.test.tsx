// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { year?: unknown }) =>
      options?.year !== undefined ? `${key}:${options.year}` : key,
  }),
}));

import { YearPicker } from "./YearPicker";

// Sixteen years of history, newest first.
const YEARS = Array.from({ length: 16 }, (_, index) => 2026 - index);

function Harness({ allowAll = false, initial = 2025 as number | null }) {
  const [value, setValue] = React.useState<number | null>(initial);
  return (
    <>
      <YearPicker years={YEARS} value={value} onChange={setValue} allowAll={allowAll} />
      <output>{String(value)}</output>
    </>
  );
}

describe("YearPicker", () => {
  afterEach(cleanup);

  it("steps to neighbouring years and stops at both ends", () => {
    render(<Harness initial={2026} />);
    const older = screen.getByRole("button", { name: "dashboard.export.yearPicker.older" });
    const newer = screen.getByRole("button", { name: "dashboard.export.yearPicker.newer" });
    expect((newer as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(older);
    expect(screen.getByText("2025", { selector: "output" })).toBeTruthy();
    for (let step = 0; step < 20; step += 1) fireEvent.click(older);
    expect(screen.getByText("2011", { selector: "output" })).toBeTruthy();
    expect((older as HTMLButtonElement).disabled).toBe(true);
  });

  it("jumps anywhere in a long history from one compact grid", () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.yearPicker.label:2025" }));
    expect(screen.getAllByRole("radio")).toHaveLength(16);
    fireEvent.click(screen.getByRole("radio", { name: "2013" }));
    expect(screen.getByText("2013", { selector: "output" })).toBeTruthy();
  });

  it("offers every year when asked, next to the newest", () => {
    render(<Harness allowAll initial={2026} />);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.yearPicker.newer" }));
    expect(screen.getByText("null", { selector: "output" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.yearPicker.older" }));
    expect(screen.getByText("2026", { selector: "output" })).toBeTruthy();
  });
});
