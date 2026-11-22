// @vitest-environment happy-dom
//
// Codex 4660319759: the overview reports a source whose coverage ends before
// today as `behind`; the pill and the status filter name it like every other
// status instead of showing a raw translation key.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import "@/i18n";
import { connectionStatusStyles } from "@/lib/connectionDisplay";

import { ConnectionStatusPill } from "./ConnectionStatusPill";
import { statusDotStyles } from "./wallets/format";
import { statusFilterOptions } from "./wallets/WalletsFilters";

afterEach(cleanup);

describe("connection status pill", () => {
  it("names a behind source and styles it", () => {
    render(<ConnectionStatusPill status="behind" />);
    const pill = screen.getByText("Behind");
    expect(pill.className).toContain(connectionStatusStyles.behind);
    expect(statusDotStyles.behind).toBeTruthy();
  });

  it("offers behind as a status filter", () => {
    expect(statusFilterOptions).toContain("behind");
  });
});
