// @vitest-environment happy-dom
//
// A CLI proxy lists one account's models as `<prefix>/<model>`
// (docs/reference/ai-cli-proxy.md#accounts-and-prefixes): the picker shows the
// model with its prefix beside it, keeps the full id on the second line, and
// keeps each prefix's models together.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import type { AiModelRow, AiProviderRow } from "@/lib/aiCapabilities";

import { ModelListRow } from "./ModelListRow";
import { compareModelRoutePrefix, modelRoutePrefix } from "./providerModelSearch";

const proxy: AiProviderRow = {
  name: "cli-proxy",
  display_name: "CLI proxy",
  base_url: "http://127.0.0.1:8317/v1",
  kind: "cli_proxy",
  has_api_key: true,
  is_default: true,
};

function row(model: AiModelRow) {
  render(
    <ModelListRow
      id="row"
      provider={proxy}
      model={model}
      posture="remote"
      isSelected={false}
      isHighlighted={false}
      isFavorite={false}
      showProvider={false}
      jumpLabel={null}
      disabled={false}
      onSelect={vi.fn()}
      onHighlight={vi.fn()}
      favoriteShortcut="Mod+D"
      onToggleFavorite={vi.fn()}
    />,
  );
}

describe("CLI proxy prefixed models in the picker", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
  });
  afterEach(() => {
    cleanup();
  });

  it("names the model and labels its prefix, with the full id below", () => {
    row({
      id: "work/claude-sonnet-5-5",
      owned_by: "anthropic",
      prefix: "work",
      base_model: "claude-sonnet-5-5",
      api_format: "chat_completions",
    });
    expect(screen.getByText("claude-sonnet-5-5")).toBeTruthy();
    const chip = document.querySelector("[data-model-prefix]");
    expect(chip?.textContent).toBe("work");
    expect(chip?.getAttribute("title")).toBe(
      "Served only by the proxy account with the prefix work",
    );
    expect(screen.getByText("work/claude-sonnet-5-5")).toBeTruthy();
  });

  it("leaves an unprefixed model as it was", () => {
    row({ id: "claude-sonnet-5-5", owned_by: "anthropic" });
    expect(document.querySelector("[data-model-prefix]")).toBeNull();
    expect(screen.getAllByText("claude-sonnet-5-5")).toHaveLength(1);
  });

  it("groups unprefixed models first, then each prefix, keeping the order inside", () => {
    const models: AiModelRow[] = [
      { id: "work/gpt-6.1-sol", prefix: "work" },
      { id: "claude-sonnet-5-5" },
      { id: "home/claude-sonnet-5-5", prefix: "home" },
      { id: "gpt-6.1-sol" },
      { id: "work/claude-sonnet-5-5", prefix: "work" },
    ];
    expect([...models].sort(compareModelRoutePrefix).map((model) => model.id)).toEqual([
      "claude-sonnet-5-5",
      "gpt-6.1-sol",
      "home/claude-sonnet-5-5",
      "work/gpt-6.1-sol",
      "work/claude-sonnet-5-5",
    ]);
    expect(modelRoutePrefix({ id: "x", prefix: " " })).toBeNull();
  });
});
