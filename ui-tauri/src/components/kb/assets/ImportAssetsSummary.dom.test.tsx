// @vitest-environment happy-dom
//
// Mounted: a provider import's result offers to track the other crypto assets
// it skipped. Turning them on stores the wallet's scope, then imports the same
// file again, or asks for the file again when its path was not kept.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

const mocks = vi.hoisted(() => ({
  update: vi.fn(),
}));

vi.mock("@/daemon/client", async () => {
  const actual = await vi.importActual<typeof import("@/daemon/client")>("@/daemon/client");
  return {
    DaemonRequestError: actual.DaemonRequestError,
    useDaemon: (kind: string) => {
      if (kind !== "status") throw new Error(`unexpected ${kind}`);
      return { data: { kind: "status", data: { database_encrypted: false } }, error: null };
    },
    useDaemonMutation: (kind: string) => {
      if (kind !== "ui.wallets.update") throw new Error(`unexpected ${kind}`);
      return { mutateAsync: mocks.update, isPending: false };
    },
  };
});

import { DaemonRequestError } from "@/daemon/client";
import { ImportAssetsSummary, type ImportReplay } from "./ImportAssetsSummary";

const SKIPPED = { scope: "bitcoin", skipped: { ETH: 4, ADA: 1 }, excluded: { AAPL: 1 } };
const REPLAY: ImportReplay = {
  wallet: "w-ct",
  source_format: "cointracking_csv",
  source_file: "/home/user/exports/cointracking.csv",
};

async function confirmTracking(button: string) {
  fireEvent.click(screen.getByRole("button", { name: "Also track these assets" }));
  fireEvent.change(screen.getByLabelText(/Type CHANGE LOCAL DATA/), {
    target: { value: "CHANGE LOCAL DATA" },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: button }));
  });
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  mocks.update.mockReset();
  mocks.update.mockResolvedValue({ kind: "ui.wallets.update", schema_version: 1, data: {} });
});

afterEach(() => {
  cleanup();
});

describe("ImportAssetsSummary", () => {
  it("imports the same file again after tracking other assets", async () => {
    const onReimport = vi.fn().mockResolvedValue(undefined);
    const onScopeStored = vi.fn();
    render(
      <ImportAssetsSummary
        wallet="CoinTracking"
        assets={SKIPPED}
        swapPairs={0}
        replay={REPLAY}
        onScopeStored={onScopeStored}
        onReimport={onReimport}
      />,
    );
    expect(screen.getByText("Also found ETH, ADA: 5 entries")).toBeTruthy();
    expect(screen.getByText(/Not imported: AAPL \(1 entry\)/)).toBeTruthy();
    await confirmTracking("Track and import again");
    // The wallet the import targeted, by the id it was imported with.
    expect(mocks.update).toHaveBeenCalledWith({
      wallet: "w-ct",
      asset_scope: "crypto",
      auth_response: { plaintext_change_ack: "CHANGE LOCAL DATA" },
    });
    expect(onReimport).toHaveBeenCalledWith(REPLAY);
    expect(onScopeStored).not.toHaveBeenCalled();
  });

  it("asks for the file again when its path was not kept", async () => {
    const onReimport = vi.fn();
    const onScopeStored = vi.fn();
    const { rerender } = render(
      <ImportAssetsSummary
        wallet="CoinTracking"
        assets={SKIPPED}
        replay={null}
        onScopeStored={onScopeStored}
        onReimport={onReimport}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Also track these assets" }));
    expect(screen.getByText(/choose the same file again afterwards/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText(/Type CHANGE LOCAL DATA/), {
      target: { value: "CHANGE LOCAL DATA" },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Track other assets" }));
    });
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({ wallet: "CoinTracking", asset_scope: "crypto" }),
    );
    // Kassiber never re-reads a file the user did not choose for this import.
    expect(onReimport).not.toHaveBeenCalled();
    expect(onScopeStored).toHaveBeenCalledWith("crypto");
    // The caller shows the stored scope; the notice asks for the file.
    rerender(
      <ImportAssetsSummary
        wallet="CoinTracking"
        assets={{ ...SKIPPED, scope: "crypto" }}
        replay={null}
        onScopeStored={onScopeStored}
        onReimport={onReimport}
      />,
    );
    expect(screen.getByRole("status").textContent).toContain("Choose the same file again");
  });

  it("keeps the scope unchanged when the acknowledgement or passphrase is refused", async () => {
    mocks.update.mockRejectedValue(
      new DaemonRequestError("ui.wallets.update", {
        kind: "error",
        schema_version: 1,
        error: { code: "local_auth_denied", message: "passphrase verification failed" },
      }),
    );
    const onReimport = vi.fn();
    render(
      <ImportAssetsSummary
        wallet="CoinTracking"
        assets={SKIPPED}
        replay={REPLAY}
        onScopeStored={vi.fn()}
        onReimport={onReimport}
      />,
    );
    await confirmTracking("Track and import again");
    expect(screen.getByRole("alert").textContent).toBe("The passphrase was not accepted.");
    expect(onReimport).not.toHaveBeenCalled();
  });

  it("shows nothing for an import that did not report assets", () => {
    const { container } = render(
      <ImportAssetsSummary
        wallet="Ledger"
        assets={undefined}
        replay={REPLAY}
        onScopeStored={vi.fn()}
        onReimport={vi.fn()}
      />,
    );
    expect(container.textContent).toBe("");
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    render(
      <ImportAssetsSummary
        wallet="CoinTracking"
        assets={SKIPPED}
        swapPairs={1}
        replay={REPLAY}
        onScopeStored={vi.fn()}
        onReimport={vi.fn()}
      />,
    );
    expect(screen.getByText("Außerdem gefunden: ETH, ADA: 5 Einträge")).toBeTruthy();
    expect(screen.getByText("Krypto-zu-Krypto-Tausch: 1 Paar verknüpft")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Diese Assets auch erfassen" }));
    expect(screen.getByRole("button", { name: "Erfassen und neu importieren" })).toBeTruthy();
  });

  it("names the coins an exchange Kassiber reads only Bitcoin from left out, offering no scope", async () => {
    const unread = { scope: "bitcoin", skipped: {}, excluded: {}, unread: { ETH: 2 } };
    const { rerender } = render(
      <ImportAssetsSummary
        wallet="Bitcoin.de"
        assets={unread}
        replay={null}
        onScopeStored={vi.fn()}
        onReimport={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        "Not imported: ETH (2 entries). Kassiber reads only Bitcoin from this exchange; record other coins yourself with a generic CSV or JSON import.",
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Also track these assets" })).toBeNull();
    await act(async () => {
      await i18n.changeLanguage("de");
    });
    rerender(
      <ImportAssetsSummary
        wallet="Bitcoin.de"
        assets={{ ...unread, unread: { ETH: 1 } }}
        replay={null}
        onScopeStored={vi.fn()}
        onReimport={vi.fn()}
      />,
    );
    expect(screen.getByText(/^Nicht importiert: ETH \(1 Eintrag\)\. Kassiber liest von dieser Börse nur Bitcoin/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Diese Assets auch erfassen" })).toBeNull();
  });
});
