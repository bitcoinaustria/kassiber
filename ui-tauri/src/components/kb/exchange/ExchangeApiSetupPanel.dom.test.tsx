// @vitest-environment happy-dom
//
// Mounted: Test connection is asynchronous, and whether the dialog may save
// depends on the credentials the form holds when the answer arrives.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

import type { ExchangeKind, ExchangeSetupDraft } from "./exchangeApiModel";

const mocks = vi.hoisted(() => ({
  test: vi.fn(),
  openExternalUrl: vi.fn(async () => {}),
}));

vi.mock("@/daemon/transport", () => ({
  openExternalUrl: mocks.openExternalUrl,
}));

vi.mock("@/daemon/client", async () => {
  const actual = await vi.importActual<typeof import("@/daemon/client")>("@/daemon/client");
  return {
    DaemonRequestError: actual.DaemonRequestError,
    useDaemonMutation: (kind: string) => {
      if (kind !== "ui.connections.exchange.test") throw new Error(`unexpected ${kind}`);
      return { mutateAsync: mocks.test, isPending: false };
    },
  };
});

import { DaemonRequestError } from "@/daemon/client";
import { ExchangeApiSetupPanel } from "./ExchangeApiSetupPanel";

const KEY = "synthetic-pasted-key";
const SECRET = "synthetic-pasted-secret";

function daemonError(code: string, message: string, details?: unknown) {
  return new DaemonRequestError("ui.connections.exchange.test", {
    kind: "error",
    schema_version: 1,
    error: { code, message, hint: null, details, retryable: false },
  });
}

function mount(exchange: ExchangeKind) {
  const drafts: ExchangeSetupDraft[] = [];
  render(
    <ExchangeApiSetupPanel
      exchange={exchange}
      labelField={<span>label-field</span>}
      footer={<span>footer</span>}
      showErrors={false}
      onDraftChange={(draft) => drafts.push(draft)}
    />,
  );
  return { last: () => drafts[drafts.length - 1] };
}

function paste(label: string | RegExp, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

async function clickTest() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
  });
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  mocks.test.mockReset();
  mocks.openExternalUrl.mockClear();
});

afterEach(() => {
  cleanup();
});

describe("ExchangeApiSetupPanel", () => {
  it("guides each exchange to a read-only key with its exact scopes", () => {
    mount("kraken");
    expect(screen.getByRole("heading", { name: "Create a read-only API key" })).toBeTruthy();
    const grant = screen.getByRole("list", { name: "Turn on only" });
    expect(grant.textContent).toContain("Query closed orders & trades");
    expect(grant.textContent).toContain("Query ledger entries");
    expect(screen.getByRole("list", { name: "Leave off" }).textContent).toContain("Withdraw funds");
    // Kraken calls its secret the private key.
    expect(screen.getByLabelText("Private key")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open the Kraken API key page" }));
    expect(mocks.openExternalUrl).toHaveBeenCalledWith("https://pro.kraken.com/app/settings/api");
    cleanup();

    mount("binance");
    expect(screen.getByRole("list", { name: "Turn on only" }).textContent).toBe("Enable Reading");
    expect(screen.getByLabelText("Secret key")).toBeTruthy();
    cleanup();

    mount("coinbase");
    expect(screen.getByRole("list", { name: "Turn on only" }).textContent).toContain(
      "wallet:transactions:read",
    );
    expect(screen.getByText(/Coinbase Developer Platform are not supported yet/)).toBeTruthy();
    cleanup();

    // Bitpanda authenticates with the key alone.
    mount("bitpanda");
    expect(screen.getByRole("list", { name: "Turn on only" }).textContent).toContain("Trade (Read)");
    expect(screen.queryByLabelText(/secret/i)).toBeNull();
    expect(screen.getByText(/refuses a key that can trade/)).toBeTruthy();
  });

  it("tests only complete credentials, sends exactly them, and never shows them", async () => {
    const panel = mount("kraken");
    const button = screen.getByRole("button", { name: "Test connection" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(panel.last()).toMatchObject({ tested: false, issue: "api_key" });
    paste("API key", ` ${KEY} `);
    expect(panel.last().issue).toBe("api_secret");
    expect(button.disabled).toBe(true);
    paste("Private key", SECRET);
    expect(panel.last().issue).toBe("test_first");
    expect(mocks.test).not.toHaveBeenCalled();

    mocks.test.mockResolvedValue({
      kind: "ui.connections.exchange.test",
      schema_version: 1,
      data: { exchange: "kraken", ok: true, read_only: null },
    });
    await clickTest();
    expect(mocks.test).toHaveBeenCalledTimes(1);
    expect(mocks.test).toHaveBeenCalledWith({ exchange: "kraken", api_key: KEY, api_secret: SECRET });
    expect(panel.last()).toMatchObject({ tested: true, issue: null });
    // Kraken does not say what else a key may do.
    expect(screen.getByRole("status").textContent).toContain(
      "Kraken does not say what else a key may do",
    );
    for (const input of [screen.getByLabelText("API key"), screen.getByLabelText("Private key")]) {
      expect((input as HTMLInputElement).type).toBe("password");
    }
    expect(document.body.textContent).not.toContain(KEY);
    expect(document.body.textContent).not.toContain(SECRET);

    // A changed key needs a new test before it can be saved.
    paste("Private key", `${SECRET}-2`);
    expect(panel.last()).toMatchObject({ tested: false, issue: "test_first" });
    expect(screen.getByText(/The key changed since the last test/)).toBeTruthy();
  });

  it("says when the exchange confirmed the key can only read", async () => {
    const panel = mount("bitpanda");
    paste("API key", KEY);
    mocks.test.mockResolvedValue({
      kind: "ui.connections.exchange.test",
      schema_version: 1,
      data: { exchange: "bitpanda", ok: true, read_only: true },
    });
    await clickTest();
    expect(mocks.test).toHaveBeenCalledWith({ exchange: "bitpanda", api_key: KEY });
    expect(screen.getByRole("status").textContent).toBe(
      "The key works, and Bitpanda confirms it can only read.",
    );
    expect(panel.last().tested).toBe(true);
  });

  it("explains a refused key, the permissions it still holds, and the daemon's words", async () => {
    const panel = mount("binance");
    paste("API key", KEY);
    paste("Secret key", SECRET);
    mocks.test.mockRejectedValue(
      daemonError(
        "auth_error",
        "This Binance API key can do more than read; Kassiber only accepts read-only keys",
        { permissions: ["enableSpotAndMarginTrading", "enableWithdrawals"] },
      ),
    );
    await clickTest();
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Binance refused the key.");
    expect(alert.textContent).toContain(
      "Permissions still on: enableSpotAndMarginTrading, enableWithdrawals",
    );
    expect(alert.textContent).toContain("Details: This Binance API key can do more than read");
    expect(panel.last()).toMatchObject({ tested: false, issue: "test_first" });
  });

  it("explains offline mode and a backend without these kinds", async () => {
    mount("kraken");
    paste("API key", KEY);
    paste("Private key", SECRET);
    mocks.test.mockRejectedValueOnce(daemonError("network_egress_disabled", "Network egress is disabled"));
    await clickTest();
    expect(screen.getByRole("alert").textContent).toContain(
      "Offline mode is on, so Kassiber did not contact Kraken.",
    );
    mocks.test.mockRejectedValueOnce(
      daemonError("daemon_unavailable", "daemon kind 'ui.connections.exchange.test' is not wired to real UI data yet", {
        kind: "ui.connections.exchange.test",
      }),
    );
    await clickTest();
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Exchange API connections need Kassiber's Rust backend.");
    // The Python daemon's wording would only confuse here.
    expect(alert.textContent).not.toContain("not wired");
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    mount("bitpanda");
    expect(screen.getByRole("heading", { name: "API-Key nur zum Lesen erstellen" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Verbindung testen" })).toBeTruthy();
    expect(screen.getByRole("list", { name: "Nur einschalten" })).toBeTruthy();
  });
});
