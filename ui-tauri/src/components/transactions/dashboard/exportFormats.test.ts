import { describe, expect, it } from "vitest";

import {
  EXPORT_FORMAT_GROUPS,
  EXPORT_FORMATS,
  exportFormatLogo,
  isToolFormat,
} from "./exportFormats";

describe("export formats", () => {
  it("gives every tool format the logo its connection uses", () => {
    for (const option of EXPORT_FORMATS.filter(isToolFormat)) {
      expect(option.catalogId, option.id).toBeTruthy();
      expect(exportFormatLogo(option), option.id).toBeTruthy();
      expect(option.daemon.kind, option.id).toBe("platform");
    }
  });

  it("gives every Kassiber format an icon and a known group", () => {
    for (const option of EXPORT_FORMATS) {
      expect(EXPORT_FORMAT_GROUPS).toContain(option.group);
      if (!isToolFormat(option)) expect(option.icon, option.id).toBeTruthy();
    }
    expect(new Set(EXPORT_FORMATS.map((option) => option.id)).size).toBe(EXPORT_FORMATS.length);
  });
});
