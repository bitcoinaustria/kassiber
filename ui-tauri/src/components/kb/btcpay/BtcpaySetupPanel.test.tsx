import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

import "@/i18n";

import { BtcpaySetupPanel } from "./BtcpaySetupPanel";

function render(props: Partial<React.ComponentProps<typeof BtcpaySetupPanel>> = {}) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <BtcpaySetupPanel
        savedInstances={[]}
        wallets={[]}
        connectionLabel="Shop"
        showErrors={false}
        labelField={<span>label-field</span>}
        onDraftChange={() => {}}
        onUseCsvInstead={() => {}}
        {...props}
      />
    </QueryClientProvider>,
  );
}

describe("BtcpaySetupPanel", () => {
  it("guides a new instance through a least-privilege key", () => {
    const html = render();

    expect(html).toContain("Connect BTCPay Server");
    expect(html).toContain("label-field");
    expect(html).toContain("Read-only plus BTCPay wallet history");
    expect(html).toContain("btcpay.store.canviewstoresettings");
    expect(html).toContain('aria-checked="true"');
    expect(html).toContain("Open BTCPay to create this key");
    expect(html).toContain("Enter the server URL first.");
    expect(html).toContain("Create the key manually");
    expect(html).toContain("Check key and stores");
    expect(html).toContain("Use manual CSV import instead");
  });

  it("starts from a saved instance without asking for a new key", () => {
    const html = render({
      savedInstances: [{ name: "shop-btcpay", display_name: "Shop BTCPay", is_default: true }],
    });

    expect(html).toContain("Shop BTCPay (shop-btcpay)");
    expect(html).not.toContain("connection-btcpay-api-key");
    expect(html).not.toContain("Read-only (recommended)");
  });
});
