// @vitest-environment happy-dom
//
// The trust dialog fetches only on the user's click, shows the fingerprint,
// and trusts by sending exactly the fingerprint shown.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import { CERTIFICATE_ERROR_KEYS } from "@/lib/certificateTrust";
import i18n from "@/i18n";

const daemon = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@/daemon/client", () => {
  class DaemonRequestError extends Error {
    envelope: { error?: { code?: string } };
    constructor(code: string) {
      super(code);
      this.envelope = { error: { code } };
    }
  }
  return {
    DaemonRequestError,
    useDaemonMutation: (kind: string) => ({
      mutateAsync: (args: unknown) => daemon.invoke(kind, args),
    }),
  };
});

import { DaemonRequestError } from "@/daemon/client";
import { TrustCertificateDialog } from "./TrustCertificateDialog";

const FINGERPRINT =
  "81:4C:F5:98:8C:BF:B3:07:03:EA:D7:34:00:E1:33:39:A2:A4:53:E6:CE:4E:5F:FF:68:DD:8D:9C:5B:17:89:6A";
const presented = {
  backend: "home",
  kind: "electrum",
  host: "node.local",
  port: 50002,
  fingerprint_sha256: FINGERPRINT,
  subject: "node.local",
  issuer: "node.local",
  not_before: "2026-01-01T00:00:00Z",
  not_after: "2036-01-01T00:00:00Z",
  self_signed: true,
  names: ["node.local"],
  replaces_fingerprint_sha256: null,
  clears_insecure: false,
};

beforeEach(() => {
  daemon.invoke.mockReset();
  void i18n.changeLanguage("en");
});
afterEach(cleanup);

function open(props: Partial<Parameters<typeof TrustCertificateDialog>[0]> = {}) {
  const onTrusted = vi.fn();
  const onClose = vi.fn();
  render(
    <TrustCertificateDialog
      open
      backend="home"
      backendLabel="Home node"
      onClose={onClose}
      onTrusted={onTrusted}
      {...props}
    />,
  );
  return { onTrusted, onClose };
}

describe("TrustCertificateDialog", () => {
  it("contacts nothing until the user asks, then trusts the shown fingerprint", async () => {
    daemon.invoke.mockImplementation(async (kind: string) =>
      kind === "ui.backends.certificate.fetch"
        ? { data: presented }
        : { data: { ...presented, trusted: true, certificate: "/data/certificates/home.pem" } },
    );
    const { onTrusted } = open();
    expect(daemon.invoke).not.toHaveBeenCalled();
    expect(screen.queryByText(FINGERPRINT)).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fetch certificate" }));
    });
    expect(daemon.invoke).toHaveBeenCalledWith("ui.backends.certificate.fetch", { name: "home" });
    expect(screen.getByText(FINGERPRINT)).toBeTruthy();
    expect(screen.getByText("node.local:50002")).toBeTruthy();
    expect(screen.getByText("Itself (self-signed)")).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Trust this server" }));
    });
    expect(daemon.invoke).toHaveBeenLastCalledWith("ui.backends.certificate.trust", {
      name: "home",
      fingerprint: FINGERPRINT,
    });
    expect(onTrusted).toHaveBeenCalledWith(
      expect.objectContaining({ certificate: "/data/certificates/home.pem" }),
    );
    expect(screen.getByRole("status").textContent).toContain("node.local");
  });

  it("warns before replacing a changed certificate and names the old one", async () => {
    daemon.invoke.mockResolvedValue({
      data: { ...presented, replaces_fingerprint_sha256: "01:F4", clears_insecure: true },
    });
    open({ problem: "changed" });
    expect(screen.getByRole("alert").textContent).toContain("certificate changed");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fetch certificate" }));
    });
    expect(screen.getByText(/fingerprint 01:F4/)).toBeTruthy();
    expect(screen.getByText(/turns certificate verification back on/)).toBeTruthy();
  });

  it("explains a refusal and trusts nothing", async () => {
    daemon.invoke.mockImplementation(async (kind: string) => {
      if (kind === "ui.backends.certificate.fetch") return { data: presented };
      throw new (DaemonRequestError as unknown as new (code: string) => Error)(
        "backend_certificate_mismatch",
      );
    });
    const { onTrusted } = open();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fetch certificate" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Trust this server" }));
    });
    expect(screen.getByRole("alert").textContent).toContain("different certificate");
    expect(onTrusted).not.toHaveBeenCalled();
  });

  it("has an English and German message for every refusal", () => {
    for (const lang of ["en", "de"]) {
      for (const key of CERTIFICATE_ERROR_KEYS) {
        const full = `settings:certificateTrust.errors.${key}`;
        expect(i18n.getFixedT(lang)(full as never), `${lang} ${key}`).not.toBe(full);
      }
    }
  });
});
