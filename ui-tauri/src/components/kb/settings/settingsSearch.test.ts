import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { buildAppSearchResults } from "@/components/kb/search";
import { isDevOnlySettingsSection } from "../devMode";
import { SETTINGS_SECTIONS } from "./SettingsNavigation";
import {
  SETTINGS_SEARCH_ENTRIES,
  settingsSearchKeywords,
} from "./settingsSearch";

// Every settings panel's source, to find the rows that carry each anchor.
const PANEL_SOURCES = Object.values(
  import.meta.glob<string>(["./*.tsx", "!./*.test.tsx"], {
    query: "?raw",
    import: "default",
    eager: true,
  }),
).join("\n");

const translate = (lng: string) => {
  const t = i18n.getFixedT(lng);
  return (key: string, options?: Record<string, unknown>) =>
    t(key as never, options as never) as unknown;
};

describe("SETTINGS_SEARCH_ENTRIES", () => {
  it("names unique anchors in existing sections", () => {
    const sectionIds = new Set(SETTINGS_SECTIONS.map((section) => section.id));
    const anchors = SETTINGS_SEARCH_ENTRIES.map((entry) => entry.anchor);
    expect(new Set(anchors).size).toBe(anchors.length);
    for (const entry of SETTINGS_SEARCH_ENTRIES) {
      expect(sectionIds.has(entry.section), entry.anchor).toBe(true);
    }
    // Every section is reachable by at least one of its settings.
    for (const id of sectionIds) {
      expect(
        SETTINGS_SEARCH_ENTRIES.some((entry) => entry.section === id),
        id,
      ).toBe(true);
    }
  });

  it("points every entry at a row or group that carries its anchor", () => {
    for (const { anchor } of SETTINGS_SEARCH_ENTRIES) {
      // A literal `anchorId="…"`, or a per-layer map value (`… : "…"`).
      const literal = PANEL_SOURCES.includes(`anchorId="${anchor}"`);
      const mapped = new RegExp(`:\\s*"${anchor}",?\\n`).test(PANEL_SOURCES);
      expect(literal || mapped, anchor).toBe(true);
    }
  });

  it("has a label and keywords in English and German", () => {
    for (const lng of ["en", "de"]) {
      const t = translate(lng);
      for (const entry of SETTINGS_SEARCH_ENTRIES) {
        const label = t(`settings:${entry.labelKey}`);
        expect(typeof label, `${lng} ${entry.labelKey}`).toBe("string");
        expect(label, `${lng} ${entry.labelKey}`).not.toContain(entry.labelKey);
        expect(
          settingsSearchKeywords(t(`settings:${entry.keywordsKey}`)).length,
          `${lng} ${entry.keywordsKey}`,
        ).toBeGreaterThan(0);
      }
    }
  });
});

describe("setting-level app search", () => {
  const search = (query: string, lng = "en", developerToolsEnabled = false) =>
    buildAppSearchResults({
      query,
      aiFeaturesEnabled: true,
      developerToolsEnabled,
      t: translate(lng),
      limit: 20,
    });

  it("finds a control by name and carries its anchor", () => {
    const [top] = search("clipboard");
    expect(top?.id).toBe("setting:security-privacy:clearClipboard");
    expect(top?.metadata?.settingAnchor).toBe("clearClipboard");
    expect(top?.route).toEqual({ to: "/settings", hash: "privacy" });

    expect(search("auto-lock").map((result) => result.id)).toContain(
      "setting:security-lock:autoLock",
    );
  });

  it("matches German queries in a German UI and English ones as well", () => {
    expect(search("Zwischenablage", "de")[0]?.id).toBe(
      "setting:security-privacy:clearClipboard",
    );
    expect(search("clipboard", "de")[0]?.id).toBe(
      "setting:security-privacy:clearClipboard",
    );
  });

  it("keeps developer-only settings hidden while developer tools are off", () => {
    const devEntries = SETTINGS_SEARCH_ENTRIES.filter((entry) =>
      isDevOnlySettingsSection(entry.section),
    );
    expect(devEntries.length).toBeGreaterThan(0);
    for (const entry of devEntries) {
      const id = `setting:${entry.section}:${entry.anchor}`;
      const label = translate("en")(`settings:${entry.labelKey}`) as string;
      expect(search(label).map((result) => result.id)).not.toContain(id);
      expect(search(label, "en", true).map((result) => result.id)).toContain(id);
    }
  });
});
