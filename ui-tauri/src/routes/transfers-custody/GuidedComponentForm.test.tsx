import { Fragment } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";
import { GuidedComponentForm } from "./GuidedComponentForm";
import { createInitialGuidedForm } from "./guidedComponentModel";

vi.mock("@/daemon/client", () => ({
  useDaemonMutation: () => ({ isPending: false, mutateAsync: vi.fn() }),
}));

function movementForm(destinationAsset = "BTC") {
  const form = createInitialGuidedForm();
  form.legs[0].amountBtc = "1";
  form.legs[1].amountBtc = "0.99";
  form.legs[1].asset = destinationAsset;
  form.legs[2].amountBtc = "0.01";
  return form;
}

function movementText(markup: string) {
  const summary = markup.match(/role="status">([\s\S]*?)<\/dl>/)?.[1] ?? "";
  return summary.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();
}

describe("guided movement presentation", () => {
  it("keeps the actual destination asset in a BTC-to-LBTC quantity movement", () => {
    const text = movementText(renderToStaticMarkup(<GuidedComponentForm initialForm={movementForm("LBTC")} />));
    expect(text).toContain("Sources 1 BTC = Destinations 0.99 LBTC + Fees 0.01 BTC");
    expect(text).not.toContain("0.99 BTC");
  });

  it("renders the source once in a same-asset balance equation", () => {
    const text = movementText(renderToStaticMarkup(<GuidedComponentForm initialForm={movementForm()} />));
    expect(text).toContain("Sources 1 BTC = Destinations 0.99 BTC + Fees 0.01 BTC");
    expect(text.match(/Sources/g)).toHaveLength(1);
  });

  it("does not infer a shared conservation unit for unrelated asset families", () => {
    const markup = renderToStaticMarkup(<GuidedComponentForm initialForm={movementForm("USD")} />);
    expect(movementText(markup)).toBe("");
  });

  it("keeps each label attached to its own control when create and revise forms coexist", () => {
    const first = movementForm();
    const second = movementForm();
    first.conservationMode = second.conservationMode = "conversion";
    const markup = renderToStaticMarkup(<Fragment>
      <GuidedComponentForm initialForm={first} />
      <GuidedComponentForm initialForm={second} variant="embedded" edit={{ componentId: "component-1", state: "draft" }} />
    </Fragment>);
    const ids = [...markup.matchAll(/\sid="([^"]+)"/g)].map((match) => match[1]);
    expect(new Set(ids).size).toBe(ids.length);
    const labels = [...markup.matchAll(/<label[^>]*\sfor="([^"]+)"/g)].map((match) => match[1]);
    for (const id of labels) expect(ids.filter((candidate) => candidate === id)).toHaveLength(1);
  });
});
