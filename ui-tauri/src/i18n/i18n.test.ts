import { describe, expect, it } from "vitest";

import { DEFAULT_LANGUAGE, SUPPORTED_LANGUAGE_CODES } from "./config";
import { NAMESPACES, resources } from "./resources";

type Bundle = Record<string, unknown>;

/** Flatten a nested resource object into sorted dotted leaf paths. */
function leafKeys(bundle: Bundle, prefix = ""): string[] {
  return Object.entries(bundle)
    .flatMap(([key, value]) => {
      const path = prefix ? `${prefix}.${key}` : key;
      return value !== null &&
        typeof value === "object" &&
        !Array.isArray(value)
        ? leafKeys(value as Bundle, path)
        : [path];
    })
    .sort();
}

describe("i18n resource bundles", () => {
  it("ships a bundle for every supported language", () => {
    for (const code of SUPPORTED_LANGUAGE_CODES) {
      expect(resources[code], `missing bundle for "${code}"`).toBeTruthy();
    }
  });

  // Parity, not non-emptiness: a registered-but-unfilled namespace is a valid
  // mid-migration state. The guard that matters is that no language drifts from
  // another (no half-translated namespace), which is enforced for every key.
  it.each(NAMESPACES)(
    "keeps every language in lockstep with %s keys",
    (ns) => {
      const reference = leafKeys(resources[DEFAULT_LANGUAGE][ns]);
      for (const code of SUPPORTED_LANGUAGE_CODES) {
        expect(
          leafKeys(resources[code][ns]),
          `${code}/${ns} keys must match ${DEFAULT_LANGUAGE}/${ns}`,
        ).toEqual(reference);
      }
    },
  );
});
