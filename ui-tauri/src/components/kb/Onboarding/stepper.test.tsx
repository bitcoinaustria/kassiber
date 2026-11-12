import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import i18n from "@/i18n";

import { OnboardingStepCount, OnboardingStepper } from "./stepper";

describe("OnboardingStepper", () => {
  it("names every step button, not only finished ones", () => {
    const html = renderToStaticMarkup(
      <OnboardingStepper
        steps={[
          { label: "Your books", summary: "My Books" },
          { label: "Sync" },
          { label: "Review" },
        ]}
        current={1}
      />,
    );

    expect(html).toContain('aria-label="Your books: My Books"');
    expect(html).toMatch(/aria-current="step"[^>]*aria-label="Sync"|aria-label="Sync"[^>]*aria-current="step"/);
    expect(html).toContain('aria-label="Review"');
  });
});

describe("OnboardingStepCount", () => {
  afterEach(async () => {
    await i18n.changeLanguage("en");
  });

  it("counts from one over the step track's list", () => {
    // The express path opens on Security, the fourth of five steps.
    expect(
      renderToStaticMarkup(<OnboardingStepCount current={3} total={5} />),
    ).toContain("Step 4 of 5");
    expect(
      renderToStaticMarkup(<OnboardingStepCount current={0} total={5} />),
    ).toContain("Step 1 of 5");
  });

  it("reads in Austrian German", async () => {
    await i18n.changeLanguage("de");
    expect(
      renderToStaticMarkup(<OnboardingStepCount current={4} total={5} />),
    ).toContain("Schritt 5 von 5");
  });
});
