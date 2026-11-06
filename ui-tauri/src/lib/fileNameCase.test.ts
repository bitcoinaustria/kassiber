/**
 * macOS and Windows check out the source on case-insensitive file systems,
 * where `SettingsModal.tsx` and `settingsModal.ts` collide and the desktop
 * build fails. Linux CI would never notice, so this keeps names in one folder
 * distinct beyond their case and extension.
 */
import { readdirSync, statSync } from "node:fs";
import { join, parse, relative } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(__dirname, "..");

function folders(dir: string): string[] {
  return [
    dir,
    ...readdirSync(dir)
      .map((name) => join(dir, name))
      .filter((path) => statSync(path).isDirectory())
      .flatMap(folders),
  ];
}

describe("source file names", () => {
  it("stay distinct on case-insensitive file systems", () => {
    const collisions: string[] = [];
    for (const dir of folders(SRC)) {
      // Module resolution drops the extension, so `a.ts` and `A.tsx` collide too.
      const seen = new Map<string, string>();
      for (const name of readdirSync(dir)) {
        const key = parse(name).name.toLowerCase();
        const other = seen.get(key);
        if (other && other !== parse(name).name) {
          collisions.push(`${relative(SRC, join(dir, other))} ~ ${relative(SRC, join(dir, name))}`);
        }
        seen.set(key, parse(name).name);
      }
    }
    expect(collisions).toEqual([]);
  });
});
