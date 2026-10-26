import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { OnboardingStepper } from "./stepper";

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
