import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { FirstSyncCard } from "./FirstSyncCard";

describe("FirstSyncCard failure details", () => {
  it("shows the supplied failure instead of only the failed phase", () => {
    const html = renderToStaticMarkup(
      <FirstSyncCard
        progress={{ value: 62, label: "Decoding transactions", indeterminate: false }}
        failed
        failedPhase="decode_enrich"
        failureDetail="Backend request timed out. Retry the connection."
        onDismiss={() => {}}
      />,
    );
    expect(html).toContain("Backend request timed out. Retry the connection.");
    expect(html).toContain('role="alert"');
  });

  it("does not show an old error while another refresh runs", () => {
    const html = renderToStaticMarkup(
      <FirstSyncCard
        progress={{ value: 20, label: "Fetching history", indeterminate: false }}
        failureDetail="Old failure"
        onDismiss={() => {}}
      />,
    );
    expect(html).not.toContain("Old failure");
  });
});
