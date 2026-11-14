// @vitest-environment happy-dom
//
// The wallet page offers the trust dialog beside a sync error an untrusted or
// changed certificate caused, for a backend kind that can pin one, and never
// contacts the server just by showing the offer.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

const daemon = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@/daemon/client", () => ({
  DaemonRequestError: class extends Error {},
  useDaemonMutation: (kind: string) => ({
    mutateAsync: (args: unknown) => daemon.invoke(kind, args),
  }),
}));

import { CertificateTrustOffer } from "./CertificateTrustOffer";

beforeEach(() => {
  daemon.invoke.mockReset();
  void i18n.changeLanguage("en");
});
afterEach(cleanup);

describe("CertificateTrustOffer", () => {
  it.each(["untrusted", "changed"] as const)(
    "offers the dialog for a %s certificate without fetching",
    async (problem) => {
      render(<CertificateTrustOffer problem={problem} backend={{ name: "home", kind: "electrum" }} />);
      const button = screen.getByRole("button", { name: "Check certificate…" });
      await act(async () => {
        fireEvent.click(button);
      });
      expect(screen.getByRole("button", { name: "Fetch certificate" })).toBeTruthy();
      expect(daemon.invoke).not.toHaveBeenCalled();
    },
  );

  it.each([
    { problem: null, backend: { name: "home", kind: "electrum" } },
    { problem: "untrusted" as const, backend: { name: "pay", kind: "btcpay" } },
    { problem: "untrusted" as const, backend: { name: "node", kind: "lnd" } },
    { problem: "untrusted" as const, backend: { kind: "electrum" } },
    { problem: "untrusted" as const, backend: null },
  ])("offers nothing for $problem on $backend", ({ problem, backend }) => {
    render(<CertificateTrustOffer problem={problem} backend={backend} />);
    expect(screen.queryByRole("button")).toBeNull();
  });
});
