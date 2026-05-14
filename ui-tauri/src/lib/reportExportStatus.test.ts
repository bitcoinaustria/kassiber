import { describe, expect, it } from "vitest";

import {
  reportExportStatusForYear,
  type ReportExportStatus,
} from "./reportExportStatus";

describe("reportExportStatusForYear", () => {
  const status: ReportExportStatus = {
    year: 2026,
    tone: "success",
    message: "kassiber-austrian-e1kv-2026.pdf saved.",
    path: "/exports/kassiber-austrian-e1kv-2026.pdf",
  };

  it("keeps export notices for the active tax year", () => {
    expect(reportExportStatusForYear(status, 2026)).toBe(status);
  });

  it("hides stale export notices from a previous tax year", () => {
    expect(reportExportStatusForYear(status, 2025)).toBeNull();
  });
});
