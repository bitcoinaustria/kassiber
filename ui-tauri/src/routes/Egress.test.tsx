import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { EgressRow, type EgressRecord } from "./Egress";

const AI_EGRESS: EgressRecord = {
  id: 7,
  ts: "2026-07-04T12:34:56Z",
  subsystem: "ai",
  host: "api.openai.com",
  port: 443,
  scheme: "https",
  operation: "http.request",
  method: "POST",
  bytes_out: 2048,
  via_proxy: false,
  allowlist_status: "expected",
  allowlist_label: "provider:OpenAI",
  allowlist_source: "database",
  user_allowlisted: true,
};

describe("EgressRow", () => {
  it("keeps the stored request metadata collapsed by default", () => {
    const html = renderToStaticMarkup(
      <table>
        <tbody>
          <EgressRow record={AI_EGRESS} onToggle={vi.fn()} />
        </tbody>
      </table>,
    );

    expect(html).toContain("api.openai.com:443");
    expect(html).toContain("POST");
    expect(html).not.toContain("Stored record");
    expect(html).not.toContain("request body");
  });

  it("expands to the exact safe egress record without raw request fields", () => {
    const html = renderToStaticMarkup(
      <table>
        <tbody>
          <EgressRow record={AI_EGRESS} expanded onToggle={vi.fn()} />
        </tbody>
      </table>,
    );

    expect(html).toContain("Captured metadata");
    expect(html).toContain("Stored record");
    expect(html).toContain("&quot;host&quot;: &quot;api.openai.com&quot;");
    expect(html).toContain("&quot;method&quot;: &quot;POST&quot;");
    expect(html).toContain("provider:OpenAI");
    expect(html).toContain("Path, query values, request body, headers");
    expect(html).not.toContain("Authorization");
    expect(html).not.toContain("bc1qsecret");
    expect(html).not.toContain("tell me about my wallet");
  });
});
