// @vitest-environment happy-dom
//
// Mounted: the provider form's connection test is asynchronous, and what it
// may save depends on the values the form holds when the answer arrives.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  mutations: {} as Record<string, ReturnType<typeof vi.fn>>,
}));

vi.hoisted(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/transport", () => ({
  getTransport: () => ({ invoke: mocks.invoke }),
}));

vi.mock("@/daemon/client", () => ({
  useDaemonMutation: (kind: string) => {
    mocks.mutations[kind] ??= vi.fn(async () => ({}));
    return { mutateAsync: mocks.mutations[kind], isPending: false };
  },
}));

import { AiProviderForm, type ExistingAiProvider } from "./AiProviderForm";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function testAnswer(data: Record<string, unknown>) {
  return {
    kind: "ai.test_connection",
    schema_version: 1,
    data: {
      model_count: 3,
      check_kind: "models",
      api_format: "auto",
      detected_api_format: null,
      tools_accepted: true,
      probe: { status: "ok", reason: null, model: "m", finish_reason: "stop" },
      ...data,
    },
  };
}

function mount(initial: ExistingAiProvider | null = null) {
  const onClose = vi.fn();
  const onSaved = vi.fn();
  render(<AiProviderForm open initial={initial} onClose={onClose} onSaved={onSaved} />);
  return { onClose, onSaved };
}

function field(label: RegExp | string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function type(label: RegExp | string, value: string) {
  fireEvent.change(field(label), { target: { value } });
}

function testCalls(): Array<Record<string, unknown>> {
  return mocks.invoke.mock.calls
    .map(([request]) => request as { kind: string; args: Record<string, unknown> })
    .filter((request) => request.kind === "ai.test_connection")
    .map((request) => request.args);
}

async function clickTest() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
  });
}

async function submit() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: /^(Add provider|Save)$/ }));
  });
}

describe("AiProviderForm (mounted)", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  it("drops a test answer for an endpoint the form no longer holds", async () => {
    const pending = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(pending.promise);
    mount();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    expect(testCalls()).toHaveLength(1);

    // Retargeted while the first endpoint is still answering.
    type("Base URL or CLI locator", "http://127.0.0.1:9090/v1");
    await act(async () => {
      pending.resolve(testAnswer({ detected_api_format: "chat_completions" }));
    });
    expect(screen.queryByText(/Connected/)).toBeNull();

    await submit();
    const create = mocks.mutations["ai.providers.create"];
    expect(create).toHaveBeenCalledOnce();
    const args = create.mock.calls[0][0] as Record<string, unknown>;
    expect(args.base_url).toBe("http://127.0.0.1:9090/v1");
    // The other endpoint's detection is not saved with this one.
    expect(args).not.toHaveProperty("detected_api_format");
  });

  it("saves the detection of a test that matches what is saved", async () => {
    mocks.invoke.mockResolvedValueOnce(testAnswer({ detected_api_format: "chat_completions" }));
    mount();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    expect(screen.getByText(/Connected/)).toBeTruthy();

    await submit();
    const args = mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<
      string,
      unknown
    >;
    expect(args.detected_api_format).toBe("chat_completions");
  });

  it("ignores an older run's answer once a newer run started", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    mount();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    type("Base URL or CLI locator", "http://127.0.0.1:9090/v1");
    await clickTest();

    await act(async () => {
      first.resolve(testAnswer({ detected_api_format: "chat_completions" }));
    });
    expect(screen.getByText("Testing…")).toBeTruthy();
    await act(async () => {
      second.resolve(testAnswer({ detected_api_format: "responses" }));
    });
    expect(screen.getByText(/Connected/)).toBeTruthy();

    await submit();
    const args = mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<
      string,
      unknown
    >;
    expect(args.detected_api_format).toBe("responses");
  });
});

function posture(): string | null {
  const pressed = screen
    .getAllByRole("button", { name: /^(local|remote|tee)$/ })
    .find((button) => button.getAttribute("aria-pressed") === "true");
  return pressed?.textContent ?? null;
}

const OFF_DEVICE_NOTE = "This address isn't on this device, so it's treated as remote.";

describe("AiProviderForm posture from the URL", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  it("moves a local provider to remote for an address off this device, and says so", () => {
    mount();
    expect(posture()).toBe("local");
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    // Nothing moves while the address is still being typed.
    expect(posture()).toBe("local");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");
    expect(screen.getByRole("status").textContent).toBe(OFF_DEVICE_NOTE);
    // The off-device acknowledgement is asked for, not assumed.
    expect(
      (screen.getByRole("checkbox") as HTMLButtonElement).getAttribute("aria-checked"),
    ).toBe("false");

    // Back to an address on this device: the note goes, the posture stays.
    type("Base URL or CLI locator", "http://127.0.0.1:8000/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(screen.queryByText(OFF_DEVICE_NOTE)).toBeNull();
    expect(posture()).toBe("remote");
  });

  it("keeps local for this device's addresses", () => {
    mount();
    for (const url of ["http://localhost:11434/v1", "http://127.0.0.1:8000/v1", "http://[::1]:8000/v1"]) {
      type("Base URL or CLI locator", url);
      fireEvent.blur(field("Base URL or CLI locator"));
      expect(posture()).toBe("local");
    }
    expect(screen.queryByText(OFF_DEVICE_NOTE)).toBeNull();
  });

  it("never moves the user's remote or TEE choice", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "remote" }));
    type("Base URL or CLI locator", "http://localhost:11434/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");

    fireEvent.click(screen.getByRole("button", { name: "tee" }));
    type("Base URL or CLI locator", "https://enclave.example.test/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("tee");
    expect(screen.queryByText(OFF_DEVICE_NOTE)).toBeNull();
  });

  it("treats a CLI agent as remote", () => {
    mount();
    type("Base URL or CLI locator", "claude-cli://default");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");
    expect(screen.getByRole("status").textContent).toBe(
      "CLI agents can send prompts off this device, so this provider is treated as remote.",
    );
  });

  it("asks for the acknowledgement when saving moves the posture", async () => {
    mount();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    // Submitted straight from the field, without leaving it.
    await act(async () => {
      fireEvent.submit(field("Base URL or CLI locator").closest("form")!);
    });
    expect(posture()).toBe("remote");
    expect(
      screen.getByText("Acknowledge that prompts may leave this device before saving."),
    ).toBeTruthy();
    expect(mocks.mutations["ai.providers.create"]).not.toHaveBeenCalled();
  });

  it("tests an address off this device as remote", async () => {
    mocks.invoke.mockResolvedValueOnce(testAnswer({}));
    mount();
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    await clickTest();
    expect(testCalls()[0]).toMatchObject({
      base_url: "https://gateway.example.test/v1",
      kind: "remote",
    });
  });
});

describe("AiProviderForm presets", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const ACK = /I understand prompts and selected Kassiber context may leave this device/;

  function choose(label: string) {
    fireEvent.click(screen.getByRole("button", { name: label }));
  }

  it.each([
    {
      label: "Ollama",
      name: "Ollama",
      url: "http://localhost:11434/v1",
      posture: "local",
      model: "qwen3.6:35b",
      notes: "Local Ollama endpoint.",
      hint: "Runs models on this device. Start it with ollama serve.",
      keyHint: "(usually not required)",
    },
    {
      label: "OpenAI",
      name: "OpenAI",
      url: "https://api.openai.com/v1",
      posture: "remote",
      model: "",
      notes: "OpenAI; prompts may leave this device.",
      hint: "OpenAI's own models, with an API key from your OpenAI account.",
      keyHint: "(required)",
    },
    {
      label: "OpenRouter",
      name: "OpenRouter",
      url: "https://openrouter.ai/api/v1",
      posture: "remote",
      model: "",
      notes: "OpenRouter; prompts may leave this device.",
      hint: "One key for models from many vendors.",
      keyHint: "(required)",
    },
    {
      label: "Other OpenAI-compatible",
      name: "",
      url: "",
      posture: "local",
      model: "",
      notes: "",
      hint: /Any server that speaks the OpenAI API/,
      keyHint: "(usually not required)",
    },
    {
      label: "Codex CLI",
      name: "Codex CLI",
      url: "codex-cli://default",
      posture: "remote",
      model: "default",
      notes: "Codex CLI; prompts may leave this device.",
      hint: "Uses the Codex CLI you're signed in to; it can send prompts to its own model service.",
      keyHint: "(optional for keyless deployments)",
    },
  ])("$label fills the form", ({ label, name, url, posture: expected, model, notes, hint, keyHint }) => {
    mount();
    // Start from another preset, so every field is seen to change.
    choose(label === "Ollama" ? "oMLX" : "Ollama");
    choose(label);
    expect(field("Name").value).toBe(name);
    expect(field("Base URL or CLI locator").value).toBe(url);
    expect(field("Default model (optional)").value).toBe(model);
    expect(field("Notes (optional)").value).toBe(notes);
    expect(posture()).toBe(expected);
    expect(screen.getByRole("button", { name: label }).getAttribute("aria-pressed")).toBe("true");
    expect(
      document.querySelector("[data-preset-hint]")?.textContent,
    ).toEqual(typeof hint === "string" ? hint : expect.stringMatching(hint));
    expect(screen.getByText(keyHint)).toBeTruthy();
    // A remote preset shows the off-device warning and asks for the
    // acknowledgement; a local one does not.
    expect(Boolean(screen.queryByText(ACK))).toBe(expected === "remote");
  });

  it("offers the CLI agents and the local servers as before", () => {
    mount();
    const labels = screen
      .getAllByRole("button")
      .filter((button) => button.hasAttribute("data-provider-preset"))
      .map((button) => button.textContent);
    expect(labels).toEqual([
      "Ollama",
      "oMLX",
      "OpenAI",
      "OpenRouter",
      "Other OpenAI-compatible",
      "Claude CLI",
      "Codex CLI",
      "OpenCode CLI",
      "GitHub Copilot CLI",
    ]);
  });

  it("needs the key for OpenRouter before saving", async () => {
    mount();
    choose("OpenRouter");
    fireEvent.click(screen.getByRole("checkbox"));
    await submit();
    expect(screen.getByText("Enter your OpenRouter API key before saving.")).toBeTruthy();
    expect(mocks.mutations["ai.providers.create"]).not.toHaveBeenCalled();

    type("API key (required)", "sk-or-test");
    await submit();
    expect(mocks.mutations["ai.providers.create"]).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        kind: "remote",
        acknowledged: true,
      }),
    );
    expect(mocks.mutations["ai.providers.set_api_key"]).toHaveBeenCalledWith({
      name: "openrouter",
      api_key: "sk-or-test",
    });
  });

  it("follows the URL's posture for another OpenAI-compatible server", () => {
    mount();
    choose("OpenAI");
    choose("Other OpenAI-compatible");
    expect(posture()).toBe("local");
    type("Base URL or CLI locator", "https://llm.example.test/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");
    expect(screen.getByText(OFF_DEVICE_NOTE)).toBeTruthy();
  });
});
