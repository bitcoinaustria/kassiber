// @vitest-environment happy-dom
//
// Mounted, not static: revealing a passphrase is a click that changes the
// input's type, and the revealed state must not survive a remount.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TextField } from "./fields";

afterEach(() => {
  cleanup();
});

function renderPassphraseField(revealable: boolean) {
  return render(
    <TextField
      label="Passphrase"
      name="database-passphrase"
      type="password"
      revealable={revealable}
      value="correct horse battery"
      placeholder="At least 12 characters"
      onChange={vi.fn()}
    />,
  );
}

const passphraseInput = () =>
  screen.getByLabelText<HTMLInputElement>("Passphrase");

describe("TextField passphrase reveal", () => {
  it("toggles between masked and shown text", () => {
    renderPassphraseField(true);
    const toggle = screen.getByRole("button", { name: "Show passphrase" });

    expect(passphraseInput().type).toBe("password");
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(toggle.getAttribute("aria-controls")).toBe("database-passphrase");

    fireEvent.click(toggle);
    expect(passphraseInput().type).toBe("text");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    expect(toggle.getAttribute("title")).toBe("Hide passphrase");

    fireEvent.click(toggle);
    expect(passphraseInput().type).toBe("password");
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
  });

  it("masks the passphrase again after the field remounts", () => {
    const first = renderPassphraseField(true);
    fireEvent.click(screen.getByRole("button", { name: "Show passphrase" }));
    expect(passphraseInput().type).toBe("text");

    first.unmount();
    renderPassphraseField(true);

    expect(passphraseInput().type).toBe("password");
  });

  it("offers no toggle unless asked", () => {
    renderPassphraseField(false);

    expect(screen.queryByRole("button", { name: "Show passphrase" })).toBeNull();
    expect(passphraseInput().type).toBe("password");
  });
});
