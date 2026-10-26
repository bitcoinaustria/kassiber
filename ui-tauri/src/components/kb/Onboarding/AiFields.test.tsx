import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM } from "./constants";
import { AiFields } from "./AiFields";

describe("AiFields offline setup", () => {
  it("keeps AI off while offline", () => {
    const html = renderToStaticMarkup(
      <AiFields
        form={{
          ...DEFAULT_FORM,
          backendSetupMode: "skip",
          aiSetupMode: "disabled",
        }}
        update={vi.fn()}
      />,
    );

    expect(html).toContain("AI is off while offline.");
    expect(html).not.toContain("Remote provider");
  });
});
