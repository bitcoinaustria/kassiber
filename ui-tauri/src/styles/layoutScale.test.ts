/**
 * Guards the layout scale in globals.css against drift.
 *
 * Screens used to pick their own corners, shadows and glass, which is how the
 * app ended up with cards at four radii and page titles at four sizes. The
 * scale now lives in tokens and two surface classes (`kb-surface`,
 * `kb-surface-inset`); these checks keep new code from hand-rolling around
 * them. They read the raw source because a class name is only a string until
 * Tailwind sees it.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(__dirname, "..");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return name.endsWith(".tsx") && !name.includes(".test.") ? [path] : [];
  });
}

// The shadcn primitives define the scale rather than consume it.
const FILES = sourceFiles(SRC)
  .map((path) => relative(SRC, path))
  .filter((path) => !path.startsWith("components/ui/"));

const RULES = {
  // `--radius` is 0, so a bare `rounded` renders square corners.
  bareRounded: /(?<![\w:[/-])rounded(?![\w[(-])/,
  arbitraryRadius: /rounded(?:-[a-z]{1,2})?-\[/,
  arbitraryShadow: /shadow-\[/,
  // A card sits on the flat page, where a blur has nothing to blur.
  backdropBlur: /backdrop-blur/,
  // A page card is `kb-surface` (or `Card`), a tile inside one `kb-surface-inset`.
  handRolledCard: /rounded-(?:lg|xl|2xl|3xl) border bg-card(?![\w/-])/,
} as const;

type Rule = keyof typeof RULES;

/** Surfaces with a reason to step outside the scale, by file. */
const ALLOWED: Partial<Record<Rule, Record<string, string>>> = {
  arbitraryRadius: {
    "components/ai-02.tsx": "the composer's pill radius",
    "components/kb/AssistantDock.tsx": "the dock, concentric with the composer",
  },
  arbitraryShadow: {
    "components/kb/AssistantDock.tsx": "the dock floats over the page",
    "components/kb/WalletMaterialScannerDialog.tsx": "the viewfinder's cutout mask",
    "components/transactions/dashboard/TransactionsDashboard.tsx":
      "the sticky period bar's bottom edge",
  },
  backdropBlur: {
    "components/kb/AssistantDock.tsx": "the dock floats over the page",
    "components/overview-dashboard/PortfolioInspector.tsx":
      "the inspector floats over the chart",
    "components/transactions/NewTransactionDialog.tsx":
      "the sticky footer scrolls over the form",
  },
};

function violations(rule: Rule): string[] {
  const allowed = ALLOWED[rule] ?? {};
  return FILES.filter((path) => !(path in allowed)).flatMap((path) =>
    readFileSync(join(SRC, path), "utf8")
      .split("\n")
      .flatMap((line, index) =>
        (line.match(/"[^"\n]*"|`[^`\n]*`/g) ?? [])
          .filter((literal) => RULES[rule].test(literal))
          .map((literal) => `${path}:${index + 1} ${literal}`),
      ),
  );
}

describe("layout scale", () => {
  it.each(Object.keys(RULES) as Rule[])("has no %s outside its allowlist", (rule) => {
    expect(violations(rule)).toEqual([]);
  });

  it("allowlists only files that still need the exception", () => {
    const stale = (Object.keys(ALLOWED) as Rule[]).flatMap((rule) =>
      Object.keys(ALLOWED[rule] ?? {}).filter(
        (path) =>
          !readFileSync(join(SRC, path), "utf8")
            .match(/"[^"\n]*"|`[^`\n]*`/g)
            ?.some((literal) => RULES[rule].test(literal)),
      ).map((path) => `${rule}: ${path}`),
    );
    expect(stale).toEqual([]);
  });
});
