import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_FORM } from "./constants";
import { AiFields } from "./AiFields";

describe("AiFields offline setup", () => {
  it("keeps AI disabled until it is enabled later in Settings", () => {
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

    expect(html).toContain("Offline setup turns AI off");
    expect(html).not.toContain("Use a remote provider");
  });
});
