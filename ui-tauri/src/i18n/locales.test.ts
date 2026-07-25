/**
 * Structural guards over the translation bundles.
 *
 * These are checks `tsc` cannot make. The generated i18n key types are built by
 * parsing each bundle with `JSON.parse`, which silently keeps the LAST value for
 * a repeated key — so a duplicate key type-checks, passes review, and quietly
 * blanks whichever block lost. That is exactly how `settings.privacy.evidence`
 * shipped twice, leaving the Privacy panel's evidence badges rendering raw key
 * names. `JSON.parse` cannot see the problem, so these tests read the raw text.
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const LOCALES_DIR = join(__dirname, "locales");
const LOCALES = readdirSync(LOCALES_DIR);

function bundlePaths(locale: string) {
  const dir = join(LOCALES_DIR, locale);
  return readdirSync(dir)
    .filter((name) => name.endsWith(".json"))
    .map((name) => ({ namespace: name, path: join(dir, name) }));
}

/**
 * Every object path that declares the same key more than once, e.g.
 * `privacy.evidence`. Walks with a reviver-style hook because the duplicate is
 * gone by the time `JSON.parse` returns a value.
 */
function duplicateKeyPaths(json: string): string[] {
  const duplicates: string[] = [];
  // JS has no equivalent of Python's `object_pairs_hook`, and `JSON.parse`'s
  // reviver only ever sees the surviving value — so the duplicate must be caught
  // on the raw token stream. Hence this small scanner: track one `seen` set per
  // open object literal and report a key that lands in the same set twice.
  const stack: Array<{ path: string; seen: Set<string> }> = [];
  let index = 0;
  let currentKey: string | null = null;

  const readString = () => {
    // `index` sits on the opening quote.
    let out = "";
    index += 1;
    while (index < json.length) {
      const char = json[index];
      if (char === "\\") {
        out += json.slice(index, index + 2);
        index += 2;
        continue;
      }
      if (char === '"') {
        index += 1;
        return out;
      }
      out += char;
      index += 1;
    }
    throw new Error("Unterminated string in bundle");
  };

  while (index < json.length) {
    const char = json[index];
    if (char === '"') {
      const value = readString();
      // A string immediately followed by `:` is a key, anything else is a value.
      let probe = index;
      while (probe < json.length && /\s/.test(json[probe])) probe += 1;
      if (json[probe] === ":") {
        currentKey = value;
        const frame = stack[stack.length - 1];
        if (frame) {
          if (frame.seen.has(value)) {
            const path = frame.path ? `${frame.path}.${value}` : value;
            if (!duplicates.includes(path)) duplicates.push(path);
          }
          frame.seen.add(value);
        }
      }
      continue;
    }
    if (char === "{") {
      const parent = stack[stack.length - 1];
      const parentPath = parent?.path ?? "";
      const path = currentKey
        ? parentPath
          ? `${parentPath}.${currentKey}`
          : currentKey
        : parentPath;
      stack.push({ path, seen: new Set() });
      currentKey = null;
      index += 1;
      continue;
    }
    if (char === "}") {
      stack.pop();
      currentKey = null;
      index += 1;
      continue;
    }
    index += 1;
  }

  return duplicates;
}

describe("translation bundles", () => {
  it.each(LOCALES)("declares every %s key exactly once per object", (locale) => {
    const offenders = bundlePaths(locale)
      .map(({ namespace, path }) => ({
        namespace,
        duplicates: duplicateKeyPaths(readFileSync(path, "utf8")),
      }))
      .filter(({ duplicates }) => duplicates.length > 0);

    expect(offenders).toEqual([]);
  });

  it("detects a duplicate key", () => {
    // Pins the checker itself: a false negative here would make the guard above
    // pass vacuously, which is the failure mode it exists to prevent.
    expect(
      duplicateKeyPaths('{"a":{"b":{"c":1},"b":{"d":2}},"e":3}'),
    ).toEqual(["a.b"]);
    expect(duplicateKeyPaths('{"a":{"b":1},"c":{"b":2}}')).toEqual([]);
    // A repeated key inside a *string value* must not be mistaken for a key.
    expect(duplicateKeyPaths('{"a":"\\"b\\": 1","b":2}')).toEqual([]);
  });

  it("keeps en and de on the same namespaces", () => {
    const names = (locale: string) =>
      bundlePaths(locale)
        .map(({ namespace }) => namespace)
        .sort();
    expect(names("de")).toEqual(names("en"));
  });
});
