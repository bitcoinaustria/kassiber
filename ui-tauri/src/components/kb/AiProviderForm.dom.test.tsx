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

const ACK_NAME = /I understand prompts and selected Kassiber context may leave this device/;

function ackBox(): HTMLElement {
  return screen.getByRole("checkbox", { name: ACK_NAME });
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

  it("reports only the latest of two runs for the same values", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    mount();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    // A double click: the same values, twice.
    await clickTest();
    await clickTest();
    expect(testCalls()).toHaveLength(2);

    // The first run fails late; the second is still the one that counts.
    await act(async () => {
      first.resolve({
        kind: "error",
        schema_version: 1,
        error: { code: "ai_unavailable", message: "first run timed out", retryable: true },
      });
    });
    expect(screen.queryByText(/first run timed out/)).toBeNull();
    expect(screen.getByText("Testing…")).toBeTruthy();

    await act(async () => {
      second.resolve(testAnswer({ detected_api_format: "chat_completions" }));
    });
    expect(screen.getByText(/Connected/)).toBeTruthy();
    await submit();
    expect(
      (mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<string, unknown>)
        .detected_api_format,
    ).toBe("chat_completions");
  });

  it("keeps the latest run's failure over an older run's success", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    mount();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    await clickTest();
    await act(async () => {
      second.resolve({
        kind: "error",
        schema_version: 1,
        error: { code: "ai_auth_failed", message: "key revoked", retryable: false },
      });
    });
    await act(async () => {
      first.resolve(testAnswer({ detected_api_format: "responses" }));
    });
    expect(screen.getByText("Test failed: key revoked")).toBeTruthy();
    expect(screen.queryByText(/Connected/)).toBeNull();
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).not.toHaveProperty(
      "detected_api_format",
    );
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
      (ackBox() as HTMLButtonElement).getAttribute("aria-checked"),
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

  it("tests an address off this device as remote, once acknowledged", async () => {
    mocks.invoke.mockResolvedValueOnce(testAnswer({}));
    mount();
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    // Straight from the field: the posture moves and the test waits for the
    // acknowledgement it now needs.
    await clickTest();
    expect(posture()).toBe("remote");
    expect(testCalls()).toEqual([]);
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[0]).toMatchObject({
      base_url: "https://gateway.example.test/v1",
      kind: "remote",
      acknowledged: true,
    });
  });
});

describe("AiProviderForm off-device acknowledgement for Test connection", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(testAnswer({}));
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const ACK_FIRST =
    "Testing sends a short prompt to this provider. Acknowledge that prompts may leave this device first.";

  it("does not test a remote preset until the box is ticked", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "OpenRouter" }));
    type("API key (required)", "sk-or-test");
    await clickTest();
    expect(screen.getByText(ACK_FIRST)).toBeTruthy();
    expect(mocks.invoke).not.toHaveBeenCalled();

    fireEvent.click(ackBox());
    await clickTest();
    expect(screen.queryByText(ACK_FIRST)).toBeNull();
    expect(testCalls()).toEqual([
      expect.objectContaining({
        base_url: "https://openrouter.ai/api/v1",
        kind: "remote",
        api_key: "sk-or-test",
        acknowledged: true,
      }),
    ]);
  });

  it("needs no box for a local server on this device", async () => {
    mount();
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    await clickTest();
    expect(testCalls()).toHaveLength(1);
    expect(testCalls()[0]).not.toHaveProperty("acknowledged");
  });

  it("relies on a saved off-device provider's own acknowledgement", async () => {
    mount({
      name: "gateway",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
    });
    expect(screen.queryByRole("checkbox", { name: ACK_NAME })).toBeNull();
    await clickTest();
    expect(testCalls()).toHaveLength(1);
    expect(testCalls()[0]).not.toHaveProperty("acknowledged");
  });

  it("asks again when a saved local provider becomes remote", async () => {
    mount({
      name: "box",
      base_url: "http://127.0.0.1:1234/v1",
      kind: "local",
      has_api_key: false,
      acknowledged_at: "2026-10-01T00:00:00Z",
    });
    fireEvent.click(screen.getByRole("button", { name: "remote" }));
    await clickTest();
    expect(mocks.invoke).not.toHaveBeenCalled();
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ kind: "remote", acknowledged: true });
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
    fireEvent.click(ackBox());
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

describe("AiProviderForm API format and test results", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  function local() {
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    type("Default model (optional)", "qwen3:8b");
  }

  function result(): HTMLElement {
    const element = document.querySelector<HTMLElement>("[data-test-result]");
    if (!element) throw new Error("no test result");
    return element;
  }

  it("sends the chosen format with the test and the saved row", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ api_format: "chat_completions", detected_api_format: "chat_completions" }));
    mount();
    local();
    expect(screen.getByText("API format: Auto (recommended)")).toBeTruthy();
    const chat = screen.getByRole("radio", { name: "Chat Completions" });
    fireEvent.click(chat);
    expect((chat as HTMLInputElement).checked).toBe(true);
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ api_format: "chat_completions", model: "qwen3:8b" });
    expect(result().textContent).toContain("Format: Chat Completions (as set)");

    await submit();
    expect(mocks.mutations["ai.providers.create"]).toHaveBeenCalledWith(
      expect.objectContaining({
        api_format: "chat_completions",
        detected_api_format: "chat_completions",
      }),
    );
  });

  it("tests and saves auto by default, and leaves the format out for a CLI agent", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({}));
    mount();
    local();
    await clickTest();
    expect(testCalls()[0].api_format).toBe("auto");

    type("Base URL or CLI locator", "codex-cli://default");
    expect(document.querySelector("[data-provider-advanced]")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "remote" }));
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[1]).not.toHaveProperty("api_format");
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).not.toHaveProperty(
      "api_format",
    );
  });

  it("forgets a test result once the format changes", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ detected_api_format: "chat_completions" }));
    mount();
    local();
    await clickTest();
    expect(result().textContent).toContain("Format: Chat Completions (detected)");
    fireEvent.click(screen.getByRole("radio", { name: "Responses API" }));
    expect(document.querySelector("[data-test-result]")).toBeNull();
    await submit();
    const args = mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<string, unknown>;
    expect(args.api_format).toBe("responses");
    expect(args).not.toHaveProperty("detected_api_format");
  });

  it("shows what answered, over which format, and that tools were accepted", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({
        model_count: 12,
        detected_api_format: "chat_completions",
        tools_accepted: true,
        probe: { status: "ok", reason: null, model: "qwen3:8b", finish_reason: "stop" },
      }),
    );
    mount();
    local();
    await clickTest();
    expect(result().getAttribute("data-test-result")).toBe("ok");
    const text = result().textContent ?? "";
    expect(text).toContain("Connected — 12 models reachable.");
    expect(text).toContain("qwen3:8b answered the test prompt.");
    expect(text).toContain("Format: Chat Completions (detected)");
    expect(text).toContain("Tools accepted.");
    expect(document.querySelector("[data-tools-refused]")).toBeNull();
  });

  it("says when the endpoint refused tools", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({ detected_api_format: "responses", tools_accepted: false }),
    );
    mount();
    local();
    await clickTest();
    expect(document.querySelector("[data-tools-refused]")?.textContent).toBe(
      "This endpoint answered but refused tools, so the assistant can't read your book through it.",
    );
    expect(result().textContent).not.toContain("Tools accepted.");
  });

  it("does not call a listing without a reply a success", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({
        model_count: 342,
        detected_api_format: null,
        tools_accepted: null,
        probe: { status: "skipped", reason: "no_model", model: null, finish_reason: null },
      }),
    );
    mount();
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    await clickTest();
    expect(result().getAttribute("data-test-result")).toBe("no_model");
    expect(result().textContent).toContain(
      "Connected — 342 models listed, but no reply was checked.",
    );
    expect(result().textContent).toContain("Choose a model to check that it can answer.");
    expect(result().textContent).not.toContain("reachable");
  });

  it("shows a failed test plainly", async () => {
    mocks.invoke.mockResolvedValue({
      kind: "error",
      schema_version: 1,
      error: {
        code: "ai_auth_failed",
        message: "AI provider rejected the request as unauthorized",
        retryable: false,
      },
    });
    mount();
    local();
    await clickTest();
    expect(result().getAttribute("data-test-result")).toBe("fail");
    expect(result().textContent).toBe(
      "Test failed: AI provider rejected the request as unauthorized",
    );
  });

  it("resets the format when a pinned HTTP provider becomes a CLI agent", async () => {
    mount({
      name: "router",
      display_name: "Router",
      base_url: "https://openrouter.ai/api/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      api_format: "responses",
      detected_api_format: "responses",
    });
    type("Base URL or CLI locator", "codex-cli://default");
    expect(document.querySelector("[data-provider-advanced]")).toBeNull();
    await submit();
    const update = mocks.mutations["ai.providers.update"];
    expect(update).toHaveBeenCalledOnce();
    expect(update.mock.calls[0][0]).toMatchObject({
      name: "router",
      base_url: "codex-cli://default",
      api_format: "auto",
      clear: ["detected_api_format"],
    });
    expect(update.mock.calls[0][0]).not.toHaveProperty("detected_api_format");
  });

  it("opens Advanced on a saved non-auto format and names the last detection", () => {
    mount({
      name: "gateway",
      display_name: "Gateway",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      api_format: "auto",
      detected_api_format: "chat_completions",
    });
    expect(screen.getByText("Last detected: Chat Completions")).toBeTruthy();
    expect(
      (screen.getByRole("radio", { name: "Auto (recommended)" }) as HTMLInputElement).checked,
    ).toBe(true);
    // The detection belongs to the saved URL.
    type("Base URL or CLI locator", "https://other.example.test/v1");
    expect(screen.queryByText("Last detected: Chat Completions")).toBeNull();
    cleanup();

    mount({
      name: "gateway",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      api_format: "responses",
    });
    expect(
      (document.querySelector("[data-provider-advanced]") as HTMLDetailsElement).open,
    ).toBe(true);
    expect((screen.getByRole("radio", { name: "Responses API" }) as HTMLInputElement).checked).toBe(
      true,
    );
  });
});

describe("AiProviderForm model suggestions", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  function suggestions(): string[] {
    const listId = field("Default model (optional)").getAttribute("list");
    if (!listId) return [];
    return Array.from(document.querySelectorAll(`#${listId} option`)).map(
      (option) => (option as HTMLOptionElement).value,
    );
  }

  it("offers the models the test listed that can run the assistant", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({
        model_count: 4,
        models: [
          { id: "gpt-5", supports_tools: null },
          { id: "text-embedding-3-small", supports_tools: null },
          { id: "acme/no-tools", supports_tools: false },
          { id: "gpt-4o", supports_tools: null },
        ],
        detected_api_format: null,
        tools_accepted: null,
        probe: { status: "skipped", reason: "no_model", model: null, finish_reason: null },
      }),
    );
    mount();
    type("Name", "OpenAI");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    expect(suggestions()).toEqual([]);
    await clickTest();
    expect(suggestions()).toEqual(["gpt-5", "gpt-4o"]);
    expect(screen.getByText("Type to choose from the 2 models the test listed.")).toBeTruthy();

    // Choosing one keeps the list; another endpoint drops it.
    type("Default model (optional)", "gpt-5");
    expect(suggestions()).toEqual(["gpt-5", "gpt-4o"]);
    type("Base URL or CLI locator", "http://127.0.0.1:9999/v1");
    expect(suggestions()).toEqual([]);
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });
});

describe("AiProviderForm no-retention flag", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const OMIT_NAME = /Send requests without Kassiber's no-retention flag/;

  function omitBox(): HTMLElement {
    return screen.getByRole("checkbox", { name: OMIT_NAME });
  }

  function local() {
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    type("Default model (optional)", "m");
  }

  it("keeps the flag unless the user explicitly ticks the option", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ store_accepted: true }));
    mount();
    local();
    expect(omitBox().getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText(/the provider's own retention applies to your prompts/)).toBeTruthy();
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ omit_store_field: false });
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).toMatchObject({
      omit_store_field: false,
    });
  });

  it("sends requests without the flag once ticked, and says so", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ store_accepted: null }));
    mount();
    local();
    fireEvent.click(omitBox());
    expect(omitBox().getAttribute("aria-checked")).toBe("true");
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ omit_store_field: true });
    expect(document.querySelector("[data-store-omitted]")?.textContent).toBe(
      "Sent without Kassiber's no-retention flag; the provider's own retention applies.",
    );
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).toMatchObject({
      omit_store_field: true,
    });
  });

  it("explains an endpoint that refuses the flag, and opens Advanced without ticking it", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({ store_accepted: false, detected_api_format: "chat_completions" }),
    );
    mount();
    local();
    const advanced = document.querySelector("[data-provider-advanced]") as HTMLDetailsElement;
    expect(advanced.open).toBe(false);
    await clickTest();
    expect(document.querySelector("[data-store-refused]")?.textContent).toBe(
      "This endpoint refuses Kassiber's no-retention flag (store: false), so chats with it will fail. Under Advanced you can send requests without the flag; the provider's own retention then applies.",
    );
    expect(advanced.open).toBe(true);
    expect(omitBox().getAttribute("aria-checked")).toBe("false");
    // Ticking it changes what is tested, so the old result goes.
    fireEvent.click(omitBox());
    expect(document.querySelector("[data-store-refused]")).toBeNull();
  });

  it("shows a saved choice, and leaves it out for a CLI agent", async () => {
    mount({
      name: "gw",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      omit_store_field: true,
    });
    expect((document.querySelector("[data-provider-advanced]") as HTMLDetailsElement).open).toBe(
      true,
    );
    expect(omitBox().getAttribute("aria-checked")).toBe("true");
    type("Base URL or CLI locator", "codex-cli://default");
    expect(screen.queryByRole("checkbox", { name: OMIT_NAME })).toBeNull();
    await submit();
    expect(mocks.mutations["ai.providers.update"].mock.calls[0][0]).not.toHaveProperty(
      "omit_store_field",
    );
  });
});
