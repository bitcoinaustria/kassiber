// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { buildExitTaxFixture } from "@/mocks/exitTax";

import { ExitTax } from "./ExitTax";

const state = vi.hoisted(() => ({
  report: {} as ReturnType<typeof buildExitTaxFixture>,
  read: vi.fn(),
  export: vi.fn(),
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string, params: unknown) => {
    state.read(kind, params);
    return { data: { data: state.report }, isLoading: false, isError: false };
  },
  useDaemonMutation: () => ({ mutate: state.export }),
}));
vi.mock("@/components/ui/date-picker", () => ({
  DatePicker: ({ value, onChange }: { value: string; onChange: (value: string) => void }) => (
    <input aria-label="Departure date" value={value} onChange={(event) => onChange(event.target.value)} />
  ),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2025-01-01T23:30:00Z")); // 00:30 in Vienna
  vi.clearAllMocks();
});
afterEach(async () => {
  cleanup();
  vi.useRealTimers();
  await i18n.changeLanguage("en");
});

it.each([
  ["AT", "2025-01-02", "Europe/Vienna"],
  ["generic", "2025-01-01", "UTC"],
] as const)("uses the core's civil day for %s defaults, clearing, and exports", (jurisdiction, day, zone) => {
  state.report = buildExitTaxFixture(day);
  state.report.jurisdictionCode = jurisdiction;
  state.report.departureTimezone = zone;
  render(<ExitTax />);
  expect(state.read).toHaveBeenLastCalledWith("ui.reports.exit_tax_preview", {
    departure_date: undefined, destination: "eu_eea",
  });
  const date = screen.getByRole("textbox", { name: "Departure date" });
  expect((date as HTMLInputElement).value).toBe(day);
  expect(screen.getByText(`Departure timezone: ${zone}`)).toBeDefined();
  fireEvent.change(date, { target: { value: "2024-10-27" } });
  expect(state.read).toHaveBeenLastCalledWith("ui.reports.exit_tax_preview", {
    departure_date: "2024-10-27", destination: "eu_eea",
  });
  fireEvent.change(date, { target: { value: "" } });
  expect(state.read).toHaveBeenLastCalledWith("ui.reports.exit_tax_preview", {
    departure_date: undefined, destination: "eu_eea",
  });
  expect((date as HTMLInputElement).value).toBe(day);
  fireEvent.click(screen.getByRole("button", { name: /Export PDF/ }));
  expect(state.export).toHaveBeenLastCalledWith(
    { departure_date: day, destination: "eu_eea" }, expect.any(Object),
  );
});

it("shows the returned departure timezone in Austrian German", async () => {
  await i18n.changeLanguage("de");
  state.report = buildExitTaxFixture("2025-01-02");
  render(<ExitTax />);
  expect(screen.getByText("Zeitzone des Wegzugs: Europe/Vienna")).toBeDefined();
});
