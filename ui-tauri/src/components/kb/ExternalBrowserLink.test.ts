import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { ExternalBrowserLink } from "./ExternalBrowserLink";

describe("ExternalBrowserLink", () => {
  it("opens links through the system browser path", async () => {
    const openUrl = vi.fn().mockResolvedValue(undefined);
    const preventDefault = vi.fn();
    const link = ExternalBrowserLink({
      href: "https://github.com/bitcoinaustria/kassiber/issues/new?template=bug_report.yml",
      openUrl,
      children: createElement("span", null, "Bug report"),
    });

    await link.props.onClick({ preventDefault });

    expect(preventDefault).toHaveBeenCalledOnce();
    expect(openUrl).toHaveBeenCalledWith(
      "https://github.com/bitcoinaustria/kassiber/issues/new?template=bug_report.yml",
    );
  });
});
