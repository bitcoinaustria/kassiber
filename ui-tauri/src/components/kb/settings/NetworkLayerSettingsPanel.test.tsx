import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { BackendLayerRow } from "./NetworkLayerSettingsPanel";
import type { Backend } from "./SettingsModel";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

const local: Backend = {
  id: "cln", name: "Routing node", health: "", on: true, auth: "none",
  chain: "bitcoin", network: "main", net: "LN", kind: "coreln",
  url: "cln://local", rpcFile: "Configured RPC file",
};

function row(backend: Backend) {
  return renderToStaticMarkup(
    <BackendLayerRow backend={backend} onEdit={() => {}} onSetDefault={() => {}} />,
  );
}

describe("Core Lightning backend rows", () => {
  // The core refuses commando before sync (lightning-cli takes the rune only
  // in argv), so the list says so instead of letting a sync fail later.
  it.each([
    { url: "cln://commando" },
    { auth: "apikey" },
    { commandoPeerId: "Configured peer" },
  ])("marks a commando backend unsupported with the hint (%o)", (change) => {
    const html = row({ ...local, ...change });
    expect(html).toContain('role="alert"');
    expect(html).toContain("networkLayer.commandoUnsupported");
    expect(html).toContain("backendModal.corelnCommandoHint");
  });

  it("leaves a local backend unmarked", () => {
    const html = row(local);
    expect(html).not.toContain("networkLayer.commandoUnsupported");
    expect(html).not.toContain('role="alert"');
  });
});
